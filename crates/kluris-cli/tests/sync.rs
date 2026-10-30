//! `kluris sync [--brain NAME|all] [--no-verify] [--json]`: the command
//! surface over real git (a bare remote and a teammate's clone per brain).
//! The engine itself is in `kluris-core/tests/sync_engine/`.

use std::fs;

use serde_json::{Value, json};

mod sync_home;

use sync_home::*;

const SYNC_KEYS: [&str; 8] = [
    "pulled",
    "auto_merged",
    "left_as_is",
    "conflicts",
    "behind_conflict",
    "to_push",
    "pushed",
    "last_push",
];

/// The brain entry's `sync` object, with its keys checked.
fn sync_of(doc: &Value, at: usize) -> &Value {
    let entry = &doc["brains"][at];
    assert_eq!(keys(entry), ["name", "sync"], "{doc}");
    let sync = &entry["sync"];
    assert_eq!(keys(sync), SYNC_KEYS, "{doc}");
    assert_eq!(
        keys(&sync["left_as_is"]),
        ["reason", "paths", "detail"],
        "{doc}"
    );
    sync
}

#[test]
fn json_and_text_for_one_brain() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), ["ok", "brains"]);
    assert_eq!(doc["brains"][0]["name"], "alpha");
    assert_eq!(
        *sync_of(&doc, 0),
        json!({
            "pulled": 1, "auto_merged": [],
            "left_as_is": {"reason": null, "paths": [], "detail": null},
            "conflicts": [], "behind_conflict": 0, "to_push": 0, "pushed": false,
            "last_push": null,
        })
    );
    assert!(h.read(&root, "notes/new.md").contains("# New"));

    h.commit(&root, "notes/second.md", &neuron("Second", ""));
    let (code, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "alpha: pushed\n");
    assert_eq!(err, "kluris: alpha: pushed\n");
    let (code, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "alpha: up to date\n");
    assert_eq!(err, "", "a brain with nothing to say has no line");
}

#[test]
fn brain_all_syncs_the_clean_brain_and_reports_the_conflicted_one() {
    let mut h = Home::new();
    let alpha = h.brain("alpha");
    h.brain("beta");
    h.conflict("alpha", &alpha);
    h.teammate_pushes("beta", "notes/new.md", &neuron("New", ""));
    let (code, doc) = h.json(&["sync", "--brain", "all", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    assert_eq!(brains.len(), 2);
    assert_eq!(brains[0]["name"], "alpha");
    let a = sync_of(&doc, 0);
    assert_eq!(a["pulled"], 0);
    assert_eq!(a["left_as_is"]["reason"], Value::Null);
    let c = &a["conflicts"][0];
    assert_eq!(
        keys(c),
        ["path", "kind", "ours", "theirs", "base", "binary"]
    );
    assert_eq!(c["path"], "notes/first.md");
    assert_eq!(c["kind"], "content");
    assert_eq!(
        c["ours"],
        neuron("First", &BODY.replace("Three.", "Three, ours."))
    );
    assert_eq!(
        c["theirs"],
        neuron("First", &BODY.replace("Three.", "Three, theirs."))
    );
    assert_eq!(c["binary"], false);
    assert_eq!(brains[1]["name"], "beta");
    let b = sync_of(&doc, 1);
    assert_eq!(b["pulled"], 1);
    assert_eq!(b["left_as_is"]["reason"], Value::Null);

    // The conflict stays open: reported again, and nothing changes.
    let (code, out, err) = h.run(&["sync", "--brain", "all"]);
    assert_eq!(code, 1);
    assert!(out.contains("kluris resolve"), "{out}");
    assert_eq!(
        err,
        "kluris: alpha: 1 conflict (notes/first.md); 1 commit to push\n"
    );
}

#[test]
fn a_refused_brain_does_not_stop_the_others() {
    let mut h = Home::new();
    h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    // A brain inside a larger repo.
    let outer = h.dir.path().join("outer");
    let nested = outer.join("brains").join("nested");
    fs::create_dir_all(&nested).unwrap();
    h.write(&nested, "kluris.yml", "name: nested\ndescription: d\n");
    h.write(&nested, "notes/n.md", &neuron("N", ""));
    h.git(&outer, &["init", "-q", "-b", "main"]);
    h.register("nested", &nested);

    let (code, doc) = h.json(&["sync", "--brain", "all", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    assert_eq!(brains[0]["name"], "alpha");
    assert_eq!(sync_of(&doc, 0)["pulled"], 1);
    let nested_sync = sync_of(&doc, 1);
    assert_eq!(nested_sync["left_as_is"]["reason"], "refused");
    let why = nested_sync["left_as_is"]["detail"].as_str().unwrap();
    assert!(why.contains("larger git repository"), "{why}");
    assert_eq!(nested_sync["pulled"], 0);

    let (code, doc) = h.json(&["sync", "--brain", "nested", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(keys(&doc), ["ok", "error"]);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("larger git repository")
    );
}

#[test]
fn a_brain_blocked_by_dirty_files_exits_1() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes(
        "alpha",
        "notes/first.md",
        &neuron("First", &BODY.replace("One.", "One, theirs.")),
    );
    let mine = neuron("First", &BODY.replace("Five.", "Five, mine."));
    h.write(&root, "notes/first.md", &mine);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(
        sync_of(&doc, 0)["left_as_is"],
        json!({"reason": "uncommitted", "paths": ["notes/first.md"], "detail": null})
    );
    assert_eq!(h.read(&root, "notes/first.md"), mine);
    let (_, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert!(out.contains("commit or stash"), "{out}");
    assert_eq!(
        err,
        "kluris: alpha: left as is (uncommitted): notes/first.md\n"
    );
}

#[test]
fn sync_is_in_help() {
    let h = Home::new();
    let (code, doc) = h.json(&["help", "--json"]);
    assert_eq!(code, 0);
    let names: Vec<&str> = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let at = names
        .iter()
        .position(|n| *n == "sync")
        .expect("sync in help");
    assert_eq!(names[at - 1], "commit");
    let (code, out, _) = h.run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l.starts_with("  sync ")), "{out}");
    let (code, out, _) = h.run(&["sync", "--help"]);
    assert_eq!(code, 0);
    for text in [
        "Usage: kluris sync [OPTIONS]",
        "--brain",
        "--no-verify",
        "--json",
    ] {
        assert!(out.contains(text), "{text}: {out}");
    }
}

#[test]
fn a_locked_ref_is_reported_with_its_path_not_as_offline() {
    // A fetch stopped while it held the ref (a Windows job terminated at
    // the deadline) leaves its lock.
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let lock = root
        .join(".git")
        .join("refs")
        .join("remotes")
        .join("origin")
        .join("main.lock");
    fs::write(&lock, "0000000000000000000000000000000000000000\n").unwrap();
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let sync = sync_of(&doc, 0);
    assert_eq!(sync["left_as_is"]["reason"], "refused", "{doc}");
    assert_eq!(sync["pulled"], 0, "{doc}");
    let why = refused(sync);
    let why = why.as_str().unwrap();
    assert!(why.starts_with("a ref is locked: "), "{why}");
    assert!(why.contains("main.lock"), "{why}");
    assert!(
        why.contains("if no git is running in this brain, remove that file"),
        "{why}"
    );
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 1);
    assert_eq!(out, format!("alpha: refused: {why}\n"));
    assert!(lock.is_file(), "kluris removed the lock");

    fs::remove_file(&lock).unwrap();
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(sync_of(&doc, 0)["pulled"], 1);
    assert!(h.read(&root, "notes/new.md").contains("# New"));
}

/// A fetch the remote turns away is `refused`, with git's message
/// on one line and the fix, never `offline`: nothing fetched, exit 1, and
/// the text says the same.
fn assert_refused(h: &Home, root: &std::path::Path, message: &str, command: &str) -> Value {
    let head = h.git(root, &["rev-parse", "HEAD"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let sync = sync_of(&doc, 0).clone();
    assert_eq!(sync["left_as_is"]["reason"], "refused", "{doc}");
    assert_eq!(sync["pushed"], false, "{doc}");
    let why = refused(&sync);
    let why = why.as_str().unwrap_or_else(|| panic!("{doc}"));
    assert!(why.starts_with(message), "{why}");
    assert!(why.ends_with(&the_fix(command)), "{why}");
    assert!(!why.contains('\n'), "{why}");
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 1);
    assert_eq!(out, format!("alpha: refused: {why}\n"));
    if command == "fetch" {
        assert_eq!(h.git(root, &["rev-parse", "HEAD"]), head);
    }
    sync
}

#[test]
fn a_key_the_server_rejects_is_refused_not_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let ssh = fake_ssh(h.dir.path());
    h.env.push(("GIT_SSH_COMMAND".into(), ssh));
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.invalid/alpha.git",
        ],
    );
    let entry = assert_refused(&h, &root, DENIED, "fetch");
    assert_eq!(
        refused(&entry).as_str().unwrap().to_string(),
        format!("{DENIED}{}", the_fix("fetch"))
    );
    assert_eq!(entry["pulled"], 0);
}

#[test]
fn a_remote_repository_that_does_not_exist_is_refused_not_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let gone = h.dir.path().join("gone.git");
    h.git(&root, &["remote", "set-url", "origin", &file_url(&gone)]);
    let entry = assert_refused(&h, &root, "fatal: '", "fetch");
    let why = refused(&entry).as_str().unwrap().to_string();
    assert!(
        why.contains("gone.git' does not appear to be a git repository"),
        "{why}"
    );
    assert_eq!(entry["pulled"], 0);
}

#[test]
fn a_push_the_server_rejects_is_refused_not_offline() {
    // The fetch URL answers; the push URL turns the key away.
    let mut h = Home::new();
    let root = h.brain("alpha");
    let ssh = fake_ssh(h.dir.path());
    h.env.push(("GIT_SSH_COMMAND".into(), ssh));
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "ssh://git@example.invalid/alpha.git",
        ],
    );
    h.commit(&root, "notes/second.md", &neuron("Second", ""));
    let bare = h.dir.path().join("alpha.git");
    let remote = h.git(&bare, &["rev-parse", "main"]);
    let entry = assert_refused(&h, &root, DENIED, "push");
    assert_eq!(
        refused(&entry).as_str().unwrap().to_string(),
        format!("{DENIED}{}", the_fix("push"))
    );
    assert_eq!(entry["to_push"], 1);
    assert_eq!(h.git(&bare, &["rev-parse", "main"]), remote);
}

#[test]
fn a_host_that_does_not_resolve_is_still_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@nonexistent.invalid/x.git",
        ],
    );
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        sync_of(&doc, 0)["left_as_is"],
        json!({"reason": "offline", "paths": [], "detail": null}),
        "{doc}"
    );
    let (code, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0);
    assert_eq!(out, "alpha: offline, nothing changed\n");
    assert_eq!(err, "kluris: alpha: left as is (offline)\n");
}

#[test]
fn no_upstream_is_left_as_is_with_the_fix_and_exits_0() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.git(&root, &["branch", "--unset-upstream"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        sync_of(&doc, 0)["left_as_is"],
        json!({
            "reason": "no_upstream", "paths": [],
            "detail": "set one with git push -u origin main",
        }),
        "{doc}"
    );
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "alpha: no upstream, nothing changed; set one with git push -u origin main\n"
    );
}

/// What hooks print goes to stderr: `--json` stdout stays one document.
#[test]
fn hook_output_goes_to_stderr() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    h.commit(&root, "notes/second.md", &neuron("Second", ""));
    let hooks = root.join(".git").join("hooks");
    for hook in [
        "pre-merge-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-merge",
        "pre-push",
        "reference-transaction",
    ] {
        let path = hooks.join(hook);
        fs::write(&path, format!("#!/bin/sh\necho 'from {hook}'\nexit 0\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let (code, out, err) = h.run(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{err}");
    let doc: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    let sync = sync_of(&doc, 0);
    assert_eq!(
        (&sync["pulled"], &sync["pushed"]),
        (&json!(1), &json!(true))
    );
    for hook in [
        "pre-merge-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-merge",
        "pre-push",
    ] {
        assert!(err.contains(&format!("from {hook}")), "{hook}: {err}");
    }
}

#[test]
fn a_missing_remote_named_with_network_words_is_refused_not_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    for name in ["timed out.git", "Connection refused.git"] {
        let gone = h.dir.path().join(name);
        let gone = gone.to_string_lossy();
        h.git(&root, &["remote", "set-url", "origin", &gone]);
        let entry = assert_refused(&h, &root, "fatal: '", "fetch");
        let why = refused(&entry).as_str().unwrap().to_string();
        assert!(
            why.contains(&format!("{name}' does not appear to be a git repository")),
            "{why}"
        );
        assert_eq!(entry["pulled"], 0);
    }
}

#[test]
fn a_push_to_a_missing_remote_named_with_network_words_is_refused_not_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let gone = h.dir.path().join("Connection refused.git");
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            &gone.to_string_lossy(),
        ],
    );
    h.commit(&root, "notes/second.md", &neuron("Second", ""));
    let bare = h.dir.path().join("alpha.git");
    let remote = h.git(&bare, &["rev-parse", "main"]);
    let entry = assert_refused(&h, &root, "fatal: '", "push");
    let why = refused(&entry).as_str().unwrap().to_string();
    assert!(
        why.contains("Connection refused.git' does not appear to be a git repository"),
        "{why}"
    );
    assert_eq!(entry["to_push"], 1);
    assert_eq!(h.git(&bare, &["rev-parse", "main"]), remote);
}
