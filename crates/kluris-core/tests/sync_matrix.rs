//! The adversarial matrix of T-152: renames into and out of folders, folders
//! renamed on both sides, nested folder renames with adds, case-only renames,
//! a rename with a delete inside, binary files and symlinks in renamed
//! folders. For every scenario `kluris sync` either merges with every blob
//! both sides introduced still in the tree, or opens person conflicts that
//! change nothing; and each `kluris resolve` choice then keeps exactly that
//! side's bytes at every conflicted path, and every other change of both
//! sides.

use std::collections::BTreeMap;
use std::fs;

use kluris_core::sync::resolve::{Choice, ResolveRequest, resolve};

mod common;

use common::*;

/// Every file of a tree: path -> (mode, blob id).
fn tree(w: &World, rev: &str) -> BTreeMap<String, (String, String)> {
    w.run(&w.ours, &["ls-tree", "-r", "-z", rev])
        .split('\0')
        .filter_map(|record| {
            let (head, path) = record.split_once('\t')?;
            let mut f = head.split(' ');
            let (mode, _, oid) = (f.next()?, f.next()?, f.next()?);
            Some((path.to_string(), (mode.to_string(), oid.to_string())))
        })
        .collect()
}

fn generated(path: &str) -> bool {
    path == "brain.md" || path.ends_with("/map.md") || path == "map.md"
}

/// The new content a side introduced: its files whose blob the base has
/// nowhere (a rename alone moves a blob and introduces none).
fn introduced(w: &World, base: &str, rev: &str) -> BTreeMap<String, String> {
    let before: Vec<String> = tree(w, base).into_values().map(|(_, o)| o).collect();
    tree(w, rev)
        .into_iter()
        .filter(|(path, (_, oid))| !generated(path) && !before.contains(oid))
        .map(|(path, (_, oid))| (path, oid))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Merged,
    /// The conflicts' kinds, in path order.
    Conflicts(Vec<String>),
}

/// Run one scenario: `setup` leaves ours with local commits and theirs
/// pushed. Returns what sync did; panics on any lost change.
fn check(name: &str, setup: &dyn Fn(&World)) -> Outcome {
    // One world at a time (the worlds share a lock): the first is dropped
    // before each choice gets its own.
    let (from_ours, from_theirs, conflicts) = {
        let w = World::new();
        setup(&w);
        w.run(&w.ours, &["fetch", "-q"]);
        let ours = w.head(&w.ours);
        let theirs = w
            .run(&w.ours, &["rev-parse", "origin/main"])
            .trim()
            .to_string();
        let base = w
            .run(&w.ours, &["merge-base", "HEAD", "origin/main"])
            .trim()
            .to_string();
        let from_ours = introduced(&w, &base, &ours);
        let from_theirs = introduced(&w, &base, &theirs);
        let before = w.snapshot();
        let r = w.sync().unwrap_or_else(|e| panic!("{name}: {e}"));
        no_markers_anywhere(&w, &w.ours);
        if r.conflicts.is_empty() {
            assert!(
                r.merged && r.pushed && r.blocked.is_empty(),
                "{name}: {r:?}"
            );
            let final_oids: Vec<String> = tree(&w, "HEAD").into_values().map(|(_, o)| o).collect();
            for (path, oid) in from_ours.iter().chain(&from_theirs) {
                assert!(final_oids.contains(oid), "{name}: lost {path} ({oid})");
            }
            assert_eq!(w.status(&w.ours), "", "{name}");
            return Outcome::Merged;
        }
        assert!(!r.merged && !r.pushed, "{name}: {r:?}");
        assert_eq!(w.snapshot(), before, "{name}: a conflict changed something");
        (from_ours, from_theirs, r.conflicts)
    };
    let kinds = conflicts
        .iter()
        .map(|c| c.kind.as_str().to_string())
        .collect();

    for (label, choice) in [("ours", Choice::Ours), ("theirs", Choice::Theirs)] {
        let w = World::new();
        setup(&w);
        let r = w.sync().unwrap();
        assert_eq!(
            r.conflicts, conflicts,
            "{name} --{label}: the same conflicts"
        );
        let mut last = None;
        for c in &r.conflicts {
            let done = resolve(
                &w.ours,
                &w.git(),
                &ResolveRequest {
                    path: &c.path,
                    choice: choice.clone(),
                    sync: w.request(),
                },
            )
            .unwrap_or_else(|e| panic!("{name} --{label} {}: {e}", c.path));
            last = Some(done);
        }
        let done = last.unwrap_or_else(|| panic!("{name}: no conflict on the second run"));
        assert!(done.committed && done.pushed, "{name} --{label}: {done:?}");
        let fin = tree(&w, "HEAD");
        let below = |p: &str, c: &str| p == c || p.starts_with(&format!("{c}/"));
        for c in &r.conflicts {
            let chosen = if label == "ours" { &c.ours } else { &c.theirs };
            match chosen {
                Some(oid) => assert_eq!(
                    fin.get(&c.path).map(|(_, o)| o),
                    Some(oid),
                    "{name} --{label}: {} must hold that side's bytes",
                    c.path
                ),
                None => assert!(
                    !fin.contains_key(&c.path),
                    "{name} --{label}: {} must be gone",
                    c.path
                ),
            }
        }
        let fin_oids: Vec<&String> = fin.values().map(|(_, o)| o).collect();
        let (mine, other) = if label == "ours" {
            (&from_ours, &from_theirs)
        } else {
            (&from_theirs, &from_ours)
        };
        for (path, oid) in mine {
            assert!(
                fin_oids.contains(&oid),
                "{name} --{label}: lost its own {path}"
            );
        }
        for (path, oid) in other {
            let in_conflict = r.conflicts.iter().any(|c| below(path, &c.path))
                || r.conflicts
                    .iter()
                    .any(|c| c.ours.as_ref() == Some(oid) || c.theirs.as_ref() == Some(oid));
            if !in_conflict {
                assert!(
                    fin_oids.contains(&oid),
                    "{name} --{label}: lost the other side's {path}"
                );
            }
        }
        assert_eq!(w.status(&w.ours), "", "{name} --{label}");
    }
    Outcome::Conflicts(kinds)
}

/// Commit everything in a clone with plain git (renames as they are).
fn git_commit(w: &World, at: &std::path::Path, message: &str) {
    w.run(at, &["add", "-A"]);
    w.run(at, &["commit", "-q", "-m", message]);
}

fn push_theirs(w: &World) {
    w.run(&w.theirs, &["push", "-q"]);
}

/// A shared starting point: `notes/deep/x.md`, a binary file and a symlink,
/// committed on ours, pushed, pulled by theirs.
fn seed(w: &World) {
    w.write(&w.ours, "notes/deep/x.md", &neuron("", "# Deep\n\nOne.\n"));
    fs::write(
        w.ours.join("notes").join("img.bin"),
        b"\x00\x01BINARY BASE\xff",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("keep.md", w.ours.join("notes").join("link.md")).unwrap();
    git_commit(w, &w.ours, "seed");
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q", "--ff-only"]);
}

fn edit(w: &World, at: &std::path::Path, rel: &str, from: &str, to: &str) {
    let text = w.read(at, rel).replace(from, to);
    w.write(at, rel, &text);
}

fn mv(w: &World, at: &std::path::Path, from: &str, to: &str) {
    if let Some((dir, _)) = to.rsplit_once('/') {
        fs::create_dir_all(World::path(at, dir)).unwrap();
    }
    w.run(at, &["mv", from, to]);
}

/// A named scenario: its setup.
type Scenario = (&'static str, Box<dyn Fn(&World)>);

fn scenarios() -> Vec<Scenario> {
    vec![
        (
            "file renamed into another folder / edited",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes/keep.md", "ideas/keep.md");
                git_commit(w, &w.ours, "move");
                edit(w, &w.theirs, "notes/keep.md", "# Keep", "# Keep\n\nTheirs.");
                git_commit(w, &w.theirs, "edit");
                push_theirs(w);
            }),
        ),
        (
            "file renamed out of a nested folder / edited",
            Box::new(|w: &World| {
                seed(w);
                mv(w, &w.ours, "notes/deep/x.md", "notes/x.md");
                git_commit(w, &w.ours, "move up");
                edit(w, &w.theirs, "notes/deep/x.md", "One.", "One, theirs.");
                git_commit(w, &w.theirs, "edit");
                push_theirs(w);
            }),
        ),
        (
            "folder renamed on both sides to different names",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes", "alpha");
                git_commit(w, &w.ours, "to alpha");
                mv(w, &w.theirs, "notes", "beta");
                git_commit(w, &w.theirs, "to beta");
                push_theirs(w);
            }),
        ),
        (
            "nested folder renamed / file added in it",
            Box::new(|w: &World| {
                seed(w);
                mv(w, &w.ours, "notes/deep", "notes/inner");
                git_commit(w, &w.ours, "rename nested");
                w.write(
                    &w.theirs,
                    "notes/deep/new.md",
                    &neuron("", "# New in deep\n"),
                );
                git_commit(w, &w.theirs, "add in deep");
                push_theirs(w);
            }),
        ),
        (
            "file added / folder renamed (ours adds)",
            Box::new(|w: &World| {
                w.write(&w.ours, "ideas/added.md", &neuron("", "# Added\n"));
                git_commit(w, &w.ours, "add");
                mv(w, &w.theirs, "ideas", "thoughts");
                git_commit(w, &w.theirs, "rename");
                push_theirs(w);
            }),
        ),
        (
            "folder renamed / file added (theirs adds)",
            Box::new(|w: &World| {
                mv(w, &w.ours, "ideas", "thoughts");
                git_commit(w, &w.ours, "rename");
                w.write(&w.theirs, "ideas/added.md", &neuron("", "# Added\n"));
                git_commit(w, &w.theirs, "add");
                push_theirs(w);
            }),
        ),
        (
            "case-only rename (ours) / edited",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes/keep.md", "notes/Keep.md");
                git_commit(w, &w.ours, "case");
                edit(w, &w.theirs, "notes/keep.md", "# Keep", "# Keep\n\nTheirs.");
                git_commit(w, &w.theirs, "edit");
                push_theirs(w);
            }),
        ),
        (
            "edited / case-only rename (theirs)",
            Box::new(|w: &World| {
                edit(w, &w.ours, "notes/keep.md", "# Keep", "# Keep\n\nOurs.");
                git_commit(w, &w.ours, "edit");
                mv(w, &w.theirs, "notes/keep.md", "notes/Keep.md");
                git_commit(w, &w.theirs, "case");
                push_theirs(w);
            }),
        ),
        (
            "folder case-only rename (theirs) / file added",
            Box::new(|w: &World| {
                w.write(&w.ours, "ideas/added.md", &neuron("", "# Added\n"));
                git_commit(w, &w.ours, "add");
                // In two steps: a case-insensitive file system refuses it in one.
                mv(w, &w.theirs, "ideas", "ideas-tmp");
                mv(w, &w.theirs, "ideas-tmp", "Ideas");
                git_commit(w, &w.theirs, "case");
                push_theirs(w);
            }),
        ),
        (
            "folder renamed / a file in it deleted",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes", "lobe");
                git_commit(w, &w.ours, "rename");
                w.run(&w.theirs, &["rm", "-q", "notes/keep.md"]);
                git_commit(w, &w.theirs, "delete");
                push_theirs(w);
            }),
        ),
        (
            "folder renamed / a file in it edited",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes", "lobe");
                git_commit(w, &w.ours, "rename");
                edit(w, &w.theirs, "notes/first.md", "Three.", "Three, theirs.");
                git_commit(w, &w.theirs, "edit");
                push_theirs(w);
            }),
        ),
        (
            "binary file changed on both sides",
            Box::new(|w: &World| {
                seed(w);
                fs::write(w.ours.join("notes").join("img.bin"), b"\x00OURS\xff").unwrap();
                git_commit(w, &w.ours, "ours");
                fs::write(w.theirs.join("notes").join("img.bin"), b"\x00THEIRS\xff").unwrap();
                git_commit(w, &w.theirs, "theirs");
                push_theirs(w);
            }),
        ),
        (
            "binary file in a renamed folder / changed",
            Box::new(|w: &World| {
                seed(w);
                mv(w, &w.ours, "notes", "lobe");
                git_commit(w, &w.ours, "rename");
                fs::write(w.theirs.join("notes").join("img.bin"), b"\x00THEIRS\xff").unwrap();
                git_commit(w, &w.theirs, "theirs");
                push_theirs(w);
            }),
        ),
        (
            "symlink in a renamed folder / its target edited",
            Box::new(|w: &World| {
                seed(w);
                mv(w, &w.ours, "notes", "lobe");
                git_commit(w, &w.ours, "rename");
                edit(w, &w.theirs, "notes/keep.md", "# Keep", "# Keep\n\nTheirs.");
                git_commit(w, &w.theirs, "edit");
                push_theirs(w);
            }),
        ),
        (
            "file renamed on both sides to different names",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes/keep.md", "notes/ours.md");
                git_commit(w, &w.ours, "rename ours");
                mv(w, &w.theirs, "notes/keep.md", "notes/theirs.md");
                git_commit(w, &w.theirs, "rename theirs");
                push_theirs(w);
            }),
        ),
        (
            "folder renamed + file added there, with a content conflict",
            Box::new(|w: &World| {
                mv(w, &w.ours, "ideas", "thoughts");
                edit(w, &w.ours, "notes/first.md", "Three.", "Three, ours.");
                git_commit(w, &w.ours, "rename, edit");
                w.write(&w.theirs, "ideas/added.md", &neuron("", "# Added\n"));
                edit(w, &w.theirs, "notes/first.md", "Three.", "Three, theirs.");
                git_commit(w, &w.theirs, "add, edit");
                push_theirs(w);
            }),
        ),
        (
            "binary file / directory at the same path",
            Box::new(|w: &World| {
                fs::write(
                    w.ours.join("notes").join("topic.md"),
                    b"\x00BINARY OURS\xff",
                )
                .unwrap();
                git_commit(w, &w.ours, "binary");
                w.write(
                    &w.theirs,
                    "notes/topic.md/child.md",
                    &neuron("", "# Child\n"),
                );
                git_commit(w, &w.theirs, "directory");
                push_theirs(w);
            }),
        ),
        (
            "file renamed / deleted",
            Box::new(|w: &World| {
                mv(w, &w.ours, "notes/keep.md", "notes/renamed.md");
                git_commit(w, &w.ours, "rename");
                w.run(&w.theirs, &["rm", "-q", "notes/keep.md"]);
                git_commit(w, &w.theirs, "delete");
                push_theirs(w);
            }),
        ),
    ]
}

#[test]
fn every_rename_scenario_keeps_every_change() {
    let mut report = Vec::new();
    for (name, setup) in scenarios() {
        if !cfg!(unix) && name.starts_with("symlink") {
            continue;
        }
        let outcome = check(name, setup.as_ref());
        report.push(format!("{name}: {outcome:?}"));
    }
    println!("{}", report.join("\n"));
}
