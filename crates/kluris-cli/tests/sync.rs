//! `kluris sync [--brain NAME|all] [--no-verify] [--json]` (TEST-K3-45): the
//! command surface over real git (a bare remote and a teammate's clone per
//! brain). The merge rules themselves are in `kluris-core/tests/sync.rs`.

use std::fs;

use serde_json::{Value, json};

mod sync_home;

use sync_home::*;

const BRAIN_KEYS: [&str; 10] = [
    "name",
    "fetched",
    "merged",
    "pushed",
    "auto_merged",
    "conflicts",
    "skipped_dirty",
    "offline",
    "blocked",
    "refused",
];

#[test]
fn json_and_text_for_one_brain() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), ["ok", "brains"]);
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), BRAIN_KEYS);
    assert_eq!(
        *entry,
        json!({
            "name": "alpha", "fetched": true, "merged": true, "pushed": false,
            "auto_merged": [], "conflicts": [], "skipped_dirty": [], "offline": false,
            "blocked": [], "refused": null,
        })
    );
    assert!(h.read(&root, "notes/new.md").contains("# New"));

    let (code, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("alpha: up to date"), "{out}");
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
    let a = &brains[0];
    assert_eq!(a["name"], "alpha");
    assert_eq!(a["merged"], false);
    let c = &a["conflicts"][0];
    assert_eq!(keys(c), ["path", "kind", "base", "ours", "theirs"]);
    assert_eq!(c["path"], "notes/first.md");
    assert_eq!(c["kind"], "content");
    let b = &brains[1];
    assert_eq!(b["name"], "beta");
    assert_eq!(b["merged"], true);
    assert_eq!(b["refused"], Value::Null);

    let (code, out, _) = h.run(&["sync", "--brain", "all"]);
    assert_eq!(code, 1);
    assert!(out.contains("kluris resolve"), "{out}");
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
    assert_eq!(brains[0]["merged"], true);
    assert_eq!(keys(&brains[1]), BRAIN_KEYS);
    let refused = brains[1]["refused"].as_str().unwrap();
    assert!(refused.contains("larger git repository"), "{refused}");
    assert_eq!(brains[1]["fetched"], false);

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
    assert_eq!(doc["brains"][0]["blocked"], json!(["notes/first.md"]));
    assert_eq!(doc["brains"][0]["skipped_dirty"], json!(["notes/first.md"]));
    assert_eq!(h.read(&root, "notes/first.md"), mine);
    let (_, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert!(out.contains("commit or stash"), "{out}");
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

/// An already-open conflict is reported in the brain's entry, `--brain NAME`
/// included: the conflicts from pending.json and `refused` saying how to
/// settle it, exit 1 (decision of T-146).
#[test]
fn an_open_conflict_is_reported_in_the_entry() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.conflict("alpha", &root);
    let (code, first) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{first}");
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "brains"]);
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), BRAIN_KEYS);
    assert_eq!(entry["conflicts"], first["brains"][0]["conflicts"]);
    assert_eq!(
        entry["refused"],
        "a conflict is open: settle it with kluris resolve (or kluris resolve --abort)"
    );
    assert_eq!(entry["fetched"], false);
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 1);
    assert!(out.contains("kluris resolve"), "{out}");
}

#[test]
fn a_locked_ref_is_reported_with_its_path_not_as_offline() {
    // A fetch stopped while it held the ref (a Windows job terminated at
    // the deadline) leaves its lock (review T-163, F163-3).
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
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), BRAIN_KEYS);
    assert_eq!(entry["fetched"], false, "{doc}");
    assert_eq!(entry["offline"], false, "{doc}");
    assert_eq!(entry["merged"], false, "{doc}");
    let why = entry["refused"].as_str().unwrap();
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
    assert_eq!(doc["brains"][0]["fetched"], true);
    assert_eq!(doc["brains"][0]["merged"], true);
    assert!(h.read(&root, "notes/new.md").contains("# New"));
}

/// A fetch the remote turns away is `refused`, with git's message
/// on one line and the fix, never `offline`: nothing fetched, exit 1, and
/// the text says the same.
fn assert_refused(h: &Home, root: &std::path::Path, message: &str, command: &str) -> Value {
    let head = h.git(root, &["rev-parse", "HEAD"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), BRAIN_KEYS);
    assert_eq!(entry["offline"], false, "{doc}");
    assert_eq!(entry["pushed"], false, "{doc}");
    let why = entry["refused"].as_str().unwrap_or_else(|| panic!("{doc}"));
    assert!(why.starts_with(message), "{why}");
    assert!(why.ends_with(&the_fix(command)), "{why}");
    assert!(!why.contains('\n'), "{why}");
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 1);
    assert_eq!(out, format!("alpha: refused: {why}\n"));
    if command == "fetch" {
        assert_eq!(h.git(root, &["rev-parse", "HEAD"]), head);
    }
    entry.clone()
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
        entry["refused"].as_str().unwrap(),
        format!("{DENIED}{}", the_fix("fetch"))
    );
    assert_eq!(entry["fetched"], false);
    assert_eq!(entry["merged"], false);
}

#[test]
fn a_remote_repository_that_does_not_exist_is_refused_not_offline() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let gone = h.dir.path().join("gone.git");
    h.git(&root, &["remote", "set-url", "origin", &file_url(&gone)]);
    let entry = assert_refused(&h, &root, "fatal: '", "fetch");
    let why = entry["refused"].as_str().unwrap();
    assert!(
        why.contains("gone.git' does not appear to be a git repository"),
        "{why}"
    );
    assert_eq!(entry["fetched"], false);
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
        entry["refused"].as_str().unwrap(),
        format!("{DENIED}{}", the_fix("push"))
    );
    assert_eq!(entry["fetched"], true);
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
    let entry = &doc["brains"][0];
    assert_eq!(entry["offline"], true, "{doc}");
    assert_eq!(entry["fetched"], false, "{doc}");
    assert_eq!(entry["refused"], Value::Null, "{doc}");
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0);
    assert_eq!(out, "alpha: offline, nothing changed\n");
}
