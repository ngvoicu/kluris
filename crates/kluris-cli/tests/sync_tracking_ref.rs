//! `kluris sync` and the upstream's remote-tracking ref over real git. Sync
//! moves that ref itself, by compare-and-swap; a ref it would have to move
//! through to a branch (a symbolic ref, or a refspec that maps the branch
//! onto a local branch) refuses the brain before anything changes.

use std::fs;
use std::path::Path;

use serde_json::Value;

mod sync_home;

use sync_home::*;

/// What the brain's repository and work tree hold: HEAD and every ref
/// (symbolic ones with their target), the index's entries, `git status`,
/// `FETCH_HEAD`, and every file's bytes.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    head: String,
    refs: String,
    index: String,
    status: String,
    fetch_head: Option<Vec<u8>>,
    files: Vec<(String, Vec<u8>)>,
}

fn snapshot(h: &Home, root: &Path) -> Snapshot {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                files.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    Snapshot {
        head: h.git(root, &["symbolic-ref", "HEAD"]) + &h.git(root, &["rev-parse", "HEAD"]),
        refs: h.git(
            root,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname) %(symref)",
            ],
        ),
        index: h.git(root, &["ls-files", "--stage"]),
        status: h.git(root, &["status", "--porcelain"]),
        fetch_head: fs::read(root.join(".git").join("FETCH_HEAD")).ok(),
        files,
    }
}

/// A brain with a commit of its own and a teammate's commit on the remote.
fn diverged(h: &mut Home) -> std::path::PathBuf {
    let root = h.brain("alpha");
    h.commit(&root, "notes/mine.md", &neuron("Mine", ""));
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    root
}

/// Sync refuses the brain, `--json` and text alike, naming `refname` and
/// ending with `fix`, and leaves it exactly as it was.
fn assert_left_alone(h: &Home, root: &Path, refname: &str, fix: &str) -> String {
    let before = snapshot(h, root);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let entry = &doc["brains"][0];
    assert_eq!(entry["fetched"], false, "{doc}");
    assert_eq!(entry["merged"], false, "{doc}");
    assert_eq!(entry["pushed"], false, "{doc}");
    assert_eq!(entry["offline"], false, "{doc}");
    let why = entry["refused"]
        .as_str()
        .unwrap_or_else(|| panic!("{doc}"))
        .to_string();
    assert!(why.contains(refname), "{why}");
    assert!(why.ends_with(fix), "{why}");
    assert!(!why.contains('\n'), "{why}");
    assert_eq!(snapshot(h, root), before);

    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 1);
    assert_eq!(out, format!("alpha: refused: {why}\n"));
    assert_eq!(snapshot(h, root), before);
    assert!(!root.join("notes").join("new.md").exists());
    why
}

#[test]
fn a_symbolic_tracking_ref_is_refused_and_nothing_moves() {
    let mut h = Home::new();
    let root = diverged(&mut h);
    h.git(
        &root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/main",
            "refs/heads/main",
        ],
    );
    let why = assert_left_alone(
        &h,
        &root,
        "refs/remotes/origin/main is a symbolic ref to refs/heads/main",
        "remove it with `git symbolic-ref --delete refs/remotes/origin/main` and run kluris sync again",
    );
    assert_eq!(
        why,
        "the upstream's remote-tracking ref refs/remotes/origin/main is a symbolic ref to refs/heads/main: moving it would move refs/heads/main, so Kluris leaves it alone; remove it with `git symbolic-ref --delete refs/remotes/origin/main` and run kluris sync again"
    );

    // The fix: the next sync makes it a plain ref and merges.
    h.git(
        &root,
        &["symbolic-ref", "--delete", "refs/remotes/origin/main"],
    );
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["merged"], true, "{doc}");
    assert_eq!(doc["brains"][0]["pushed"], true, "{doc}");
    assert!(root.join("notes").join("mine.md").is_file());
    assert!(root.join("notes").join("new.md").is_file());
}

#[test]
fn a_tracking_ref_mapped_onto_a_local_branch_is_refused_and_nothing_moves() {
    let mut h = Home::new();
    let root = diverged(&mut h);
    h.git(
        &root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/heads/main",
        ],
    );
    let why = assert_left_alone(
        &h,
        &root,
        "refs/heads/main",
        "map the branch into refs/remotes/origin/ with `git config --replace-all remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'` and run kluris sync again",
    );
    assert_eq!(
        why,
        "the upstream's remote-tracking ref is refs/heads/main, outside refs/remotes/: moving it would move that ref itself, so Kluris leaves it alone; map the branch into refs/remotes/origin/ with `git config --replace-all remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'` and run kluris sync again"
    );

    h.git(
        &root,
        &[
            "config",
            "--replace-all",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["merged"], true, "{doc}");
    assert!(root.join("notes").join("new.md").is_file());
}

/// A `reference-transaction` hook that records each remote-tracking ref
/// update git is about to make (`<old> <new> <ref>`) in `log`.
fn record_tracking_updates(root: &Path, log: &Path) {
    let hook = root
        .join(".git")
        .join("hooks")
        .join("reference-transaction");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\ngrep ' refs/remotes/' >> '{}'\nexit 0\n",
            sh_path(log)
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A shell path (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn remote_head(h: &Home) -> String {
    h.git(&h.dir.path().join("alpha.git"), &["rev-parse", "main"])
        .trim()
        .to_string()
}

#[test]
fn a_plain_tracking_ref_moves_by_compare_and_swap() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let old = h.git(&root, &["rev-parse", "refs/remotes/origin/main"]);
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let log = h.dir.path().join("tracking-updates.log");
    record_tracking_updates(&root, &log);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["merged"], true, "{doc}");
    let new = remote_head(&h);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!("{} {new} refs/remotes/origin/main\n", old.trim())
    );
    let reflog = h.git(
        &root,
        &[
            "reflog",
            "show",
            "-1",
            "--format=%gs",
            "refs/remotes/origin/main",
        ],
    );
    assert_eq!(reflog, "kluris sync: fetch origin\n");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]).trim(), new);
}

#[test]
fn a_missing_tracking_ref_is_created() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.git(&root, &["update-ref", "-d", "refs/remotes/origin/main"]);
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let log = h.dir.path().join("tracking-updates.log");
    record_tracking_updates(&root, &log);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["merged"], true, "{doc}");
    let new = remote_head(&h);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!("{} {new} refs/remotes/origin/main\n", "0".repeat(40))
    );
    assert_eq!(
        h.git(&root, &["rev-parse", "refs/remotes/origin/main"])
            .trim(),
        new
    );
    assert!(root.join("notes").join("new.md").is_file());
}

/// The tracking ref turns symbolic, pointing at the checked-out branch,
/// after sync checked it and while its fetch runs (the remote's upload-pack
/// does it once it has sent the pack). The move replaces that symbolic ref
/// and never follows it: the branch moves only by the fast-forward, which
/// brings the files with it.
#[test]
fn a_tracking_ref_made_symbolic_during_the_fetch_is_not_followed() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let script = h.dir.path().join("symref-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ngit upload-pack \"$@\" || exit $?\ngit -C '{}' symbolic-ref refs/remotes/origin/main refs/heads/main >&2\n",
            sh_path(&root)
        ),
    )
    .unwrap();
    h.git(
        &root,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let entry: &Value = &doc["brains"][0];
    assert_eq!(entry["merged"], true, "{doc}");
    let new = remote_head(&h);
    let branch_log = h.git(
        &root,
        &["reflog", "show", "--format=%gs", "refs/heads/main"],
    );
    assert!(!branch_log.contains("kluris sync"), "{branch_log}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]).trim(), new);
    assert_eq!(
        h.git(&root, &["rev-parse", "refs/remotes/origin/main"])
            .trim(),
        new
    );
    let mut symbolic = std::process::Command::new("git");
    h.hermetic(&mut symbolic);
    let symbolic = symbolic
        .args(["symbolic-ref", "--quiet", "refs/remotes/origin/main"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(symbolic.status.code(), Some(1), "still symbolic");
    assert_eq!(h.git(&root, &["status", "--porcelain"]), "");
    assert!(h.read(&root, "notes/new.md").contains("# New"));
}
