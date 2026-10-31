//! `kluris resolve [--brain NAME] PATH --ours|--theirs|--file FILE --merge
//! ID`, `kluris resolve --abort`, and the open conflict in `kluris status`:
//! the command surface over real git. The rules are in
//! `kluris-core/tests/sync_engine/finish.rs`.

use std::fs;

use serde_json::{Value, json};

mod sync_home;

use sync_home::*;

const RESOLVE_KEYS: [&str; 8] = [
    "ok",
    "brain",
    "path",
    "choice",
    "remaining",
    "committed",
    "commit",
    "pushed",
];

/// Two conflicts in `alpha`: `notes/first.md` and `notes/second.md`.
fn two_conflicts(h: &mut Home) -> std::path::PathBuf {
    let root = h.brain("alpha");
    h.commit(&root, "notes/second.md", &neuron("Second", BODY));
    h.git(&root, &["push", "-q"]);
    let mate = h.teammate("alpha");
    h.git(&mate, &["pull", "-q"]);
    for rel in ["notes/first.md", "notes/second.md"] {
        let title = if rel.contains("first") {
            "First"
        } else {
            "Second"
        };
        h.write(
            &mate,
            rel,
            &neuron(title, &BODY.replace("Three.", "Three, theirs.")),
        );
        h.write(
            &root,
            rel,
            &neuron(title, &BODY.replace("Three.", "Three, ours.")),
        );
    }
    h.git(&mate, &["add", "-A"]);
    h.git(&mate, &["commit", "-q", "-m", "theirs"]);
    h.git(&mate, &["push", "-q"]);
    h.git(&root, &["add", "-A"]);
    h.git(&root, &["commit", "-q", "-m", "ours"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(
        doc["brains"][0]["sync"]["conflicts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    root
}

#[test]
fn resolving_path_by_path_then_committing_and_pushing() {
    let mut h = Home::new();
    let root = two_conflicts(&mut h);
    let merge = h.merge("alpha");
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--theirs",
        "--merge",
        &merge,
        "--json",
    ]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), RESOLVE_KEYS);
    assert_eq!(
        doc,
        json!({
            "ok": true, "brain": "alpha", "path": "notes/first.md", "choice": "theirs",
            "remaining": ["notes/second.md"], "committed": false, "commit": null, "pushed": false,
        })
    );
    let merged = h.dir.path().join("merged.md");
    let both = neuron("Second", &BODY.replace("Three.", "Three, both."));
    fs::write(&merged, &both).unwrap();
    let merged = merged.display().to_string();
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/second.md",
        "--file",
        &merged,
        "--merge",
        &merge,
        "--json",
    ]);
    assert_eq!(code, 0, "{doc}");
    let head = h.git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(doc["committed"], true);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(doc["pushed"], true);
    assert_eq!(doc["remaining"], json!([]));
    assert_eq!(doc["choice"], "file");
    assert_eq!(h.read(&root, "notes/second.md"), both);
    assert!(h.read(&root, "notes/first.md").contains("Three, theirs."));
    // The next sync is clean.
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["sync"]["conflicts"], json!([]));
}

#[test]
fn a_read_during_an_open_conflict_fetches_and_resolve_still_finishes() {
    // The conflict records its two commits: a fresh read's fetch, which
    // brings objects and writes no ref, changes nothing it settles.
    let mut h = Home::new();
    let root = h.two_conflicts_and_more();
    let pending = root.join(".git").join("kluris-sync").join("pending.json");
    let recorded = fs::read(&pending).unwrap();
    let head = h.git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    let refs = h.git(&root, &["for-each-ref"]);
    let mut wake_up = std::process::Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    h.hermetic(&mut wake_up);
    let out = wake_up
        .env_remove("KLURIS_NO_AUTO_SYNC")
        .args(["wake-up", "--brain", "alpha", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let mate = h.teammate("alpha");
    let remote = h.git(&mate, &["rev-parse", "HEAD"]).trim().to_string();
    let commit = format!("{remote}^{{commit}}");
    h.git(&root, &["cat-file", "-e", &commit]);
    assert_eq!(h.git(&root, &["for-each-ref"]), refs, "a ref changed");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]).trim(), head);
    assert_eq!(fs::read(&pending).unwrap(), recorded);

    let merge = h.merge("alpha");
    for (rel, side) in [
        ("notes/first.md", "--theirs"),
        ("notes/second.md", "--ours"),
    ] {
        let args = [
            "resolve", "--brain", "alpha", rel, side, "--merge", &merge, "--json",
        ];
        let (code, doc) = h.json(&args);
        assert_eq!(code, 0, "{doc}");
        if rel == "notes/second.md" {
            assert_eq!(doc["committed"], true, "{doc}");
            assert_eq!(doc["pushed"], true, "{doc}");
        }
    }
    assert!(h.read(&root, "notes/first.md").contains("Three, theirs."));
    assert!(h.read(&root, "notes/second.md").contains("Three, ours."));
    assert!(h.read(&root, "notes/third.md").contains("# Third"));
    assert!(!pending.exists());
}

impl Home {
    /// [`two_conflicts`], then the teammate pushes `notes/third.md`.
    fn two_conflicts_and_more(&mut self) -> std::path::PathBuf {
        let root = two_conflicts(self);
        self.teammate_pushes("alpha", "notes/third.md", &neuron("Third", BODY));
        root
    }
}

#[test]
fn text_output_one_line_per_call() {
    let mut h = Home::new();
    two_conflicts(&mut h);
    let merge = h.merge("alpha");
    let resolve = |rel| {
        h.run(&[
            "resolve", "--brain", "alpha", rel, "--ours", "--merge", &merge,
        ])
    };
    let (code, out, err) = resolve("notes/first.md");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "Resolved notes/first.md (ours); 1 conflict left: notes/second.md\n"
    );
    let (code, out, err) = resolve("notes/second.md");
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("Resolved notes/second.md (ours); committed ")
            && out.ends_with(" and pushed\n"),
        "{out}"
    );
}

#[test]
fn errors_are_clean() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    // No open conflict.
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--ours",
        "--merge",
        "000000000000",
        "--json",
    ]);
    assert_eq!(code, 1);
    assert_eq!(keys(&doc), ["ok", "error"]);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("no open sync conflict"),
        "{doc}"
    );
    h.conflict("alpha", &root);
    let (code, _) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1);
    let merge = h.merge("alpha");
    for (args, words) in [
        (&["notes/first.md"][..], "--ours, --theirs or --file"),
        (&["notes/first.md", "--ours", "--theirs"], "only one of"),
        (&["--ours"], "PATH"),
        (&["notes/first.md", "--abort"], "--abort"),
        (&["notes/glossary.md", "--ours"], "not conflicted"),
    ] {
        let mut all = vec!["resolve", "--brain", "alpha", "--merge", &merge];
        all.extend_from_slice(args);
        all.push("--json");
        let (code, doc) = h.json(&all);
        assert_eq!(code, 1, "{args:?}: {doc}");
        assert!(
            doc["error"].as_str().unwrap().contains(words),
            "{args:?}: {doc}"
        );
    }
    let bad = h.dir.path().join("bad.md");
    fs::write(&bad, "---\ntitle: Foo: bar\n---\n\n# First\n").unwrap();
    let bad = bad.display().to_string();
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--file",
        &bad,
        "--merge",
        &merge,
        "--json",
    ]);
    assert_eq!(code, 1);
    assert!(
        doc["error"].as_str().unwrap().contains("frontmatter"),
        "{doc}"
    );
}

#[test]
fn abort_drops_the_conflict() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.conflict("alpha", &root);
    let (code, _) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1);
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    let (code, doc) = h.json(&["resolve", "--brain", "alpha", "--abort", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc,
        json!({"ok": true, "brain": "alpha", "aborted": true, "dropped": 1})
    );
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]), head);
    assert!(
        !root
            .join(".git")
            .join("kluris-sync")
            .join("pending.json")
            .exists()
    );
    let (code, out, _) = h.run(&["resolve", "--brain", "alpha", "--abort"]);
    assert_eq!(code, 1, "nothing left to abort");
    assert_eq!(out, "");
}

#[test]
fn status_shows_an_open_conflict_and_only_then() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let (_, clean) = h.json(&["status", "--brain", "alpha", "--json"]);
    let v2_keys = keys(&clean["brains"][0]);
    assert!(!v2_keys.contains(&"sync".to_string()), "{clean}");
    h.conflict("alpha", &root);
    let (_, synced) = h.json(&["sync", "--brain", "alpha", "--json"]);
    let (code, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let entry = &doc["brains"][0];
    let mut expected_keys = v2_keys.clone();
    expected_keys.push("sync".into());
    assert_eq!(keys(entry), expected_keys);
    // The waiting merge's conflicts, each side its blob, with the merge's
    // id; the sync reported the same paths, id and texts.
    let pending = root.join(".git").join("kluris-sync").join("pending.json");
    let recorded: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(pending).unwrap()).unwrap();
    let merge = synced["brains"][0]["sync"]["conflicts"][0]["merge"].clone();
    let mut conflicts = recorded["conflicts"].clone();
    for c in conflicts.as_array_mut().unwrap() {
        c["merge"] = merge.clone();
    }
    assert_eq!(
        entry["sync"],
        json!({"blocked": true, "conflicts": conflicts})
    );
    assert_eq!(
        keys(&entry["sync"]["conflicts"][0]),
        ["path", "kind", "merge", "base", "ours", "theirs"]
    );
    assert_eq!(
        entry["sync"]["conflicts"][0]["path"],
        synced["brains"][0]["sync"]["conflicts"][0]["path"]
    );
    let merge = h.merge("alpha");
    let (_, out, _) = h.run(&["status", "--brain", "alpha"]);
    assert!(
        out.contains(&format!("  Sync: blocked by 1 conflict; settle it with kluris resolve\n    content: notes/first.md\n      kluris resolve --brain alpha notes/first.md --ours|--theirs|--file FILE --merge {merge}\n")),
        "{out}"
    );
    // Once settled, the key is gone again.
    let (code, _) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--ours",
        "--merge",
        &merge,
        "--json",
    ]);
    assert_eq!(code, 0);
    let (_, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(keys(&doc["brains"][0]), v2_keys);
    let _: &Value = &doc;
}

#[test]
fn resolve_is_in_help() {
    let h = Home::new();
    let (_, doc) = h.json(&["help", "--json"]);
    let names: Vec<&str> = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let at = names
        .iter()
        .position(|n| *n == "resolve")
        .expect("resolve in help");
    assert_eq!(names[at - 1], "sync");
    let (_, out, _) = h.run(&["--help"]);
    assert!(out.lines().any(|l| l.starts_with("  resolve ")), "{out}");
    let (code, out, _) = h.run(&["resolve", "--help"]);
    assert_eq!(code, 0);
    for text in [
        "Usage: kluris resolve [OPTIONS] [PATH]",
        "--ours",
        "--theirs",
        "--file",
        "--abort",
        "--no-verify",
        "--brain",
        "--json",
    ] {
        assert!(out.contains(text), "{text}: {out}");
    }
}

/// The merge is committed, but the push is turned away: the text
/// says why, not "offline".
#[test]
fn a_push_the_server_rejects_after_resolve_says_why() {
    let mut h = Home::new();
    let root = two_conflicts(&mut h);
    let ssh = fake_ssh(h.dir.path());
    h.env.push(("GIT_SSH_COMMAND".into(), ssh));
    let url = "ssh://git@example.invalid/alpha.git";
    h.git(&root, &["remote", "set-url", "--push", "origin", url]);
    let merge = h.merge("alpha");
    let resolve = |rel| {
        h.run(&[
            "resolve", "--brain", "alpha", rel, "--ours", "--merge", &merge,
        ])
    };
    let (code, _, err) = resolve("notes/first.md");
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = resolve("notes/second.md");
    assert_eq!(code, 0, "{err}");
    let not_pushed = format!(", not pushed: {DENIED}{}\n", the_fix("push"));
    assert!(
        out.starts_with("Resolved notes/second.md (ours); committed ")
            && out.ends_with(&not_pushed),
        "{out}"
    );
}

/// The `merge` id of the brain's first reported conflict.
fn merge_id(doc: &Value) -> String {
    let merge = &doc["brains"][0]["sync"]["conflicts"][0]["merge"];
    merge
        .as_str()
        .unwrap_or_else(|| panic!("no merge id: {doc}"))
        .to_string()
}

/// A resolution saved for a conflict shown against the teammate's first
/// commit never settles the conflict that replaced it (a commit of ours
/// made the record stale, and the teammate changed the file again): it
/// must name the merge it answers, a stale id is refused with the current
/// conflict's id and texts, and nothing is recorded or pushed.
#[test]
fn a_resolution_never_settles_the_conflict_that_replaced_it() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.conflict("alpha", &root);
    let (code, shown) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{shown}");
    let shown_id = merge_id(&shown);
    let solution = h.dir.path().join("solution.md");
    let both = neuron("First", &BODY.replace("Three.", "Three, ours and theirs."));
    fs::write(&solution, &both).unwrap();
    let solution = solution.display().to_string();

    h.commit(&root, "notes/later.md", &neuron("Later", ""));
    let newer = BODY.replace("Three.", "Three, theirs, with a newer critical line.");
    h.teammate_pushes("alpha", "notes/first.md", &neuron("First", &newer));
    let (code, replaced) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{replaced}");
    let current = merge_id(&replaced);
    assert_ne!(current, shown_id, "a replaced record keeps its id");
    let bare = h.dir.path().join("alpha.git");
    let published = h.git(&bare, &["rev-parse", "main"]);
    let pending = root.join(".git").join("kluris-sync").join("pending.json");
    let recorded = fs::read(&pending).unwrap();
    let resolve = |merge: Option<&str>, json: bool| {
        let mut args = vec![
            "resolve",
            "--brain",
            "alpha",
            "notes/first.md",
            "--file",
            &solution,
        ];
        if let Some(merge) = merge {
            args.extend(["--merge", merge]);
        }
        if json {
            args.push("--json");
        }
        h.run(&args)
    };

    // Without the id: a usage error.
    let (code, out, err) = resolve(None, false);
    assert_eq!(code, 2, "{out}{err}");
    assert!(err.contains("--merge"), "{err}");
    // A stale id: refused, with the current conflict.
    let (code, out, _) = resolve(Some(&shown_id), true);
    assert_eq!(code, 1, "{out}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["ok"], false);
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains(&current) && error.contains(&shown_id),
        "{error}"
    );
    assert_eq!(doc["conflict"]["merge"], current.as_str(), "{doc}");
    assert_eq!(doc["conflict"]["theirs"], neuron("First", &newer), "{doc}");
    let (code, _, err) = resolve(Some(&shown_id), false);
    assert_eq!(code, 1);
    assert!(
        err.contains(&format!(
            "kluris resolve --brain alpha notes/first.md --ours|--theirs|--file FILE --merge {current}"
        )) && err.contains("with a newer critical line"),
        "{err}"
    );
    assert_eq!(
        fs::read(&pending).unwrap(),
        recorded,
        "a choice was recorded"
    );
    assert_eq!(h.git(&bare, &["rev-parse", "main"]), published, "pushed");

    // The current id settles it.
    let (code, out, _) = resolve(Some(&current), true);
    assert_eq!(code, 0, "{out}");
    let shown_text = h.git(&bare, &["show", "main:notes/first.md"]);
    assert_eq!(shown_text, both);
}
