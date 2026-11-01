//! The explicit commands on the sync session, over the built binary, real
//! git and bare remotes on this machine: `kluris resolve` records each
//! choice under the exclusive tree claim and, with the last one, finishes
//! the merge, then the session's close pulls anything newer and pushes;
//! `kluris sync` opens and closes explicit sessions; `kluris create
//! --remote` sets the upstream and pushes the new brain to an empty remote.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, SystemTime};

use kluris_core::sync::background::{LAST_PUSH, LastPush, PushResult, timestamp};
use kluris_core::sync::claim::RemoteClaim;
use serde_json::{Value, json};

mod sync_home;

use sync_home::*;

const RESOLVE_KEYS: [&str; 9] = [
    "ok",
    "brain",
    "path",
    "choice",
    "remaining",
    "committed",
    "commit",
    "pushed",
    "sync",
];

const CREATE_KEYS: [&str; 7] = ["ok", "name", "path", "type", "lobes", "companions", "sync"];

/// What a teammate refused by an empty remote's first push sees.
const REMOTE_HAS_COMMITS: &str = "the remote already has commits; clone it and run kluris register on the clone, or point --remote at an empty repository";

fn head(h: &Home, root: &Path) -> String {
    h.git(root, &["rev-parse", "HEAD"]).trim().to_string()
}

fn bare_main(h: &Home, name: &str) -> String {
    let bare = h.dir.path().join(format!("{name}.git"));
    h.git(&bare, &["rev-parse", "main"]).trim().to_string()
}

fn pending_path(root: &Path) -> PathBuf {
    root.join(".git").join("kluris-sync").join("pending.json")
}

/// The choices the waiting merge holds, `path` to `choice`.
fn recorded(root: &Path) -> Vec<(String, String)> {
    let doc: Value =
        serde_json::from_str(&fs::read_to_string(pending_path(root)).unwrap()).unwrap();
    doc["resolved"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item["path"].as_str().unwrap().to_string(),
                        item["choice"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve(h: &Home, rel: &str, choice: &[&str], merge: &str) -> (i32, Value) {
    let mut args = vec!["resolve", "--brain", "alpha", rel];
    args.extend_from_slice(choice);
    args.extend(["--merge", merge, "--json"]);
    h.json(&args)
}

fn reason(sync: &Value) -> &Value {
    &sync["left_as_is"]["reason"]
}

/// A failed background push's record in the brain at `root`.
fn write_failure_record(root: &Path) -> PathBuf {
    let path = root.join(".git").join(LAST_PUSH);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    LastPush {
        at: timestamp(SystemTime::now()),
        head: "0".repeat(40),
        result: PushResult::NeedsPull,
        detail: Some("the remote has commits this brain does not".into()),
        commits: 1,
    }
    .write(&path)
    .unwrap();
    path
}

// --- resolve ---------------------------------------------------------------------------

#[test]
fn each_choice_is_recorded_and_the_last_one_finishes_the_merge_and_pushes() {
    let mut h = Home::new();
    let rels = ["notes/first.md", "notes/second.md", "notes/third.md"];
    let (root, merge) = h.conflicts("alpha", &rels);

    let (code, doc) = resolve(&h, rels[0], &["--ours"], &merge);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), RESOLVE_KEYS, "{doc}");
    assert_eq!(doc["choice"], "ours");
    assert_eq!(doc["remaining"], json!([rels[1], rels[2]]));
    assert_eq!(doc["committed"], false);
    assert_eq!(doc["pushed"], false);
    assert_eq!(doc["sync"]["pushed"], false, "{doc}");
    assert_eq!(recorded(&root), [(rels[0].to_string(), "ours".to_string())]);

    let (code, doc) = resolve(&h, rels[1], &["--theirs"], &merge);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["remaining"], json!([rels[2]]));
    assert_eq!(
        recorded(&root),
        [
            (rels[0].to_string(), "ours".to_string()),
            (rels[1].to_string(), "theirs".to_string()),
        ]
    );

    let merged = h.dir.path().join("merged.md");
    let both = neuron("notes/third.md", &BODY.replace("Three.", "Three, both."));
    fs::write(&merged, &both).unwrap();
    let file = merged.display().to_string();
    let (code, doc) = resolve(&h, rels[2], &["--file", &file], &merge);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), RESOLVE_KEYS, "{doc}");
    assert_eq!(doc["choice"], "file");
    assert_eq!(doc["committed"], true, "{doc}");
    assert_eq!(doc["commit"], head(&h, &root).as_str());
    assert_eq!(doc["pushed"], true, "{doc}");
    assert_eq!(doc["remaining"], json!([]));
    assert_eq!(doc["sync"]["pushed"], true, "{doc}");
    assert_eq!(doc["sync"]["to_push"], 0, "{doc}");
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
    assert!(!pending_path(&root).exists());
    assert!(h.read(&root, rels[0]).contains("Three, ours."));
    assert!(h.read(&root, rels[1]).contains("Three, theirs."));
    assert_eq!(h.read(&root, rels[2]), both);
    let parents = h.git(&root, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().count(), 3, "a merge: {parents}");
}

#[test]
fn the_last_choice_finishes_the_merge_then_the_close_pulls_what_is_newer_and_pushes() {
    let mut h = Home::new();
    let (root, merge) = h.conflicts("alpha", &["notes/first.md"]);
    h.teammate_pushes("alpha", "notes/newer.md", &neuron("Newer", BODY));
    let (code, doc) = resolve(&h, "notes/first.md", &["--theirs"], &merge);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true, "{doc}");
    assert_eq!(doc["pushed"], true, "{doc}");
    let sync = &doc["sync"];
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["conflicts"], json!([]), "{doc}");
    assert!(h.read(&root, "notes/newer.md").contains("# Newer"));
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
}

#[test]
fn a_different_id_is_refused_with_the_current_conflicts_and_id_in_sync() {
    let mut h = Home::new();
    let (root, merge) = h.conflicts("alpha", &["notes/first.md"]);
    let before = fs::read(pending_path(&root)).unwrap();
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], "000000000000");
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["ok"], false);
    assert_eq!(keys(&doc), ["ok", "error", "merge", "conflict", "sync"]);
    assert!(doc["error"].as_str().unwrap().contains(&merge), "{doc}");
    assert_eq!(doc["merge"], merge.as_str());
    let conflicts = &doc["sync"]["conflicts"];
    assert_eq!(conflicts[0]["merge"], merge.as_str(), "{doc}");
    assert_eq!(conflicts[0]["path"], "notes/first.md");
    assert_eq!(
        conflicts[0]["theirs"],
        neuron("First", &theirs_body()),
        "{doc}"
    );
    assert_eq!(fs::read(pending_path(&root)).unwrap(), before);
}

#[test]
fn no_open_conflict_is_refused_after_the_pull_with_the_sync() {
    let mut h = Home::new();
    h.brain("alpha");
    h.teammate_pushes("alpha", "notes/newer.md", &neuron("Newer", BODY));
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], "000000000000");
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "error", "sync"]);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("no open sync conflict"),
        "{doc}"
    );
    assert_eq!(doc["sync"]["pulled"], 1, "{doc}");
}

#[test]
fn two_resolvers_at_once_lose_neither_choice() {
    let mut h = Home::new();
    let rels = ["notes/first.md", "notes/second.md"];
    let (root, merge) = h.conflicts("alpha", &rels);
    let children: Vec<_> = [(rels[0], "--theirs"), (rels[1], "--ours")]
        .iter()
        .map(|(rel, side)| {
            h.command(&[
                "resolve", "--brain", "alpha", rel, side, "--merge", &merge, "--json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
        })
        .collect();
    let docs: Vec<Value> = children
        .into_iter()
        .map(|child| {
            let out = child.wait_with_output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            assert_eq!(out.status.code(), Some(0), "{text}");
            serde_json::from_str(&text).unwrap()
        })
        .collect();
    let finished: Vec<&Value> = docs.iter().filter(|d| d["committed"] == true).collect();
    assert_eq!(finished.len(), 1, "{docs:?}");
    assert!(h.read(&root, rels[0]).contains("Three, theirs."));
    assert!(h.read(&root, rels[1]).contains("Three, ours."));
    assert!(!pending_path(&root).exists());
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
}

#[test]
fn abort_is_an_unknown_flag() {
    let mut h = Home::new();
    let (root, _) = h.conflicts("alpha", &["notes/first.md"]);
    let before = fs::read(pending_path(&root)).unwrap();
    let (code, out, err) = h.run(&["resolve", "--brain", "alpha", "--abort"]);
    assert_eq!(code, 2, "{out}{err}");
    assert!(err.contains("--abort"), "{err}");
    assert_eq!(fs::read(pending_path(&root)).unwrap(), before);
    let (code, out, _) = h.run(&["resolve", "--help"]);
    assert_eq!(code, 0);
    assert!(!out.contains("--abort"), "{out}");
}

/// A waiting merge found for a commit the branch has moved on from is no
/// open conflict: the next pull replaces it, and nothing refuses it as
/// stale.
#[test]
fn a_waiting_merge_the_branch_moved_on_from_is_replaced_by_the_next_pull() {
    let mut h = Home::new();
    let (root, shown) = h.conflicts("alpha", &["notes/first.md"]);
    h.commit(&root, "notes/later.md", &neuron("Later", ""));
    let before = fs::read(pending_path(&root)).unwrap();
    let origin = h.dir.path().join("alpha.git").display().to_string();

    // Offline: nothing replaces it, and none is open.
    h.git(&root, &["remote", "set-url", "origin", &offline_url()]);
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], &shown);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "error", "sync"]);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("no open sync conflict"),
        "{doc}"
    );
    assert_eq!(*reason(&doc["sync"]), "offline", "{doc}");
    assert_eq!(fs::read(pending_path(&root)).unwrap(), before);

    // Online: the pull replaces it, and the old id is not the new merge's.
    h.git(&root, &["remote", "set-url", "origin", &origin]);
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], &shown);
    assert_eq!(code, 1, "{doc}");
    let current = doc["merge"].as_str().unwrap().to_string();
    assert_ne!(current, shown, "{doc}");
    assert_eq!(doc["sync"]["conflicts"][0]["merge"], current.as_str());
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], &current);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["pushed"], true, "{doc}");
}

/// The merge lands, but the push is turned away: exit 1, and the text says
/// why, not "offline".
#[test]
fn a_push_the_server_rejects_after_the_merge_exits_1_and_says_why() {
    let mut h = Home::new();
    let rels = ["notes/first.md", "notes/second.md"];
    let (root, merge) = h.conflicts("alpha", &rels);
    let ssh = fake_ssh(h.dir.path());
    h.env.push(("GIT_SSH_COMMAND".into(), ssh));
    let url = "ssh://git@example.invalid/alpha.git";
    h.git(&root, &["remote", "set-url", "--push", "origin", url]);
    let resolve = |rel| {
        h.run(&[
            "resolve", "--brain", "alpha", rel, "--ours", "--merge", &merge,
        ])
    };
    let (code, _, err) = resolve(rels[0]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = resolve(rels[1]);
    assert_eq!(code, 1, "{out}{err}");
    let not_pushed = format!(", not pushed: {DENIED}{}\n", the_fix("push"));
    assert!(
        out.starts_with("Resolved notes/second.md (ours); committed ")
            && out.ends_with(&not_pushed),
        "{out}"
    );
    assert!(err.contains("kluris: alpha: "), "{err}");
}

/// The merge lands, and the close's pull meets a new conflict: exit 1,
/// with the new conflict in `remaining` and `sync`.
#[test]
fn a_new_conflict_after_the_merge_exits_1() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let mate = h.teammate("alpha");
    h.commit(&root, "notes/second.md", &neuron("Second", BODY));
    h.git(&root, &["push", "-q"]);
    h.git(&mate, &["pull", "-q"]);
    // Ours changes both files; theirs only the first: one conflict.
    h.write(&root, "notes/first.md", &neuron("First", &ours_body()));
    h.write(&root, "notes/second.md", &neuron("Second", &ours_body()));
    h.git(&root, &["commit", "-q", "-am", "ours"]);
    h.teammate_pushes("alpha", "notes/first.md", &neuron("First", &theirs_body()));
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let merge = doc["brains"][0]["sync"]["conflicts"][0]["merge"]
        .as_str()
        .unwrap()
        .to_string();
    // Then theirs changes the second as well.
    h.teammate_pushes(
        "alpha",
        "notes/second.md",
        &neuron("Second", &theirs_body()),
    );
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], &merge);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["ok"], true, "{doc}");
    assert_eq!(doc["committed"], true, "{doc}");
    assert_eq!(doc["pushed"], false, "{doc}");
    assert_eq!(doc["remaining"], json!(["notes/second.md"]), "{doc}");
    let conflicts = &doc["sync"]["conflicts"];
    assert_eq!(conflicts[0]["path"], "notes/second.md", "{doc}");
    assert_ne!(conflicts[0]["merge"], merge.as_str(), "{doc}");
}

/// Uncommitted changes in the merge's way: exit 1, the choices kept.
#[test]
fn a_merge_blocked_by_uncommitted_files_exits_1_and_keeps_the_choices() {
    let mut h = Home::new();
    let rels = ["notes/first.md", "notes/second.md"];
    let (root, merge) = h.conflicts("alpha", &rels);
    let (code, doc) = resolve(&h, rels[0], &["--theirs"], &merge);
    assert_eq!(code, 0, "{doc}");
    h.write(&root, rels[0], &neuron("First", "Edited, not committed.\n"));
    let (code, doc) = resolve(&h, rels[1], &["--ours"], &merge);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["ok"], false, "{doc}");
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("uncommitted changes are in the way: notes/first.md"),
        "{doc}"
    );
    assert!(doc["sync"].is_object(), "{doc}");
    assert_eq!(
        recorded(&root),
        [
            (rels[0].to_string(), "theirs".to_string()),
            (rels[1].to_string(), "ours".to_string()),
        ]
    );
}

#[test]
fn a_failed_background_push_record_goes_once_resolve_pushed() {
    let mut h = Home::new();
    let (root, merge) = h.conflicts("alpha", &["notes/first.md"]);
    let record = write_failure_record(&root);
    let (code, doc) = resolve(&h, "notes/first.md", &["--ours"], &merge);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["pushed"], true, "{doc}");
    assert_eq!(doc["sync"]["last_push"], Value::Null, "{doc}");
    assert!(!record.exists(), "the record stayed");
    let (_, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(doc["brains"][0].get("sync"), None, "{doc}");
}

// --- kluris sync -----------------------------------------------------------------------

/// An explicit session waits for the remote claim another command holds.
#[test]
fn sync_waits_for_the_remote_claim_and_pushes() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.commit(&root, "notes/second.md", &neuron("Second", BODY));
    let held = RemoteClaim::try_take(&root.join(".git")).unwrap();
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(1500));
        drop(held);
    });
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    release.join().unwrap();
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc["brains"][0]), ["name", "sync"]);
    assert_eq!(doc["brains"][0]["sync"]["pushed"], true, "{doc}");
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
}

#[test]
fn sync_all_opens_every_brain_and_a_push_clears_its_failure_record() {
    let mut h = Home::new();
    let roots = [h.brain("alpha"), h.brain("beta")];
    let records: Vec<PathBuf> = roots
        .iter()
        .map(|root| {
            h.commit(root, "notes/second.md", &neuron("Second", BODY));
            write_failure_record(root)
        })
        .collect();
    let (code, doc) = h.json(&["sync", "--brain", "all", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), ["ok", "brains"]);
    for (at, name) in ["alpha", "beta"].iter().enumerate() {
        let entry = &doc["brains"][at];
        assert_eq!(keys(entry), ["name", "sync"], "{doc}");
        assert_eq!(entry["name"], *name);
        assert_eq!(entry["sync"]["pushed"], true, "{doc}");
        assert_eq!(entry["sync"]["last_push"], Value::Null, "{doc}");
        assert_eq!(bare_main(&h, name), head(&h, &roots[at]));
        assert!(!records[at].exists(), "{name}: the record stayed");
    }
    let (code, out, err) = h.run(&["sync", "--brain", "all"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "alpha: up to date\nbeta: up to date\n");
}

/// What a case does to the brain at its root before it syncs.
type SetUp = fn(&mut Home, &Path);

#[test]
fn sync_exits_1_on_a_refusal_a_conflict_or_a_block_and_0_offline() {
    let cases: [(&str, SetUp, i32); 4] = [
        (
            "offline",
            |h, root| {
                h.git(root, &["remote", "set-url", "origin", &offline_url()]);
            },
            0,
        ),
        (
            "refused",
            |h, root| {
                let url = h.dir.path().join("gone.git").display().to_string();
                h.git(root, &["remote", "set-url", "origin", &url]);
            },
            1,
        ),
        (
            "uncommitted",
            |h, root| {
                h.teammate_pushes("alpha", "notes/first.md", &neuron("First", &theirs_body()));
                h.write(
                    root,
                    "notes/first.md",
                    &neuron("First", "Mine, uncommitted.\n"),
                );
            },
            1,
        ),
        ("conflict", |h, root| h.conflict("alpha", root), 1),
    ];
    for (what, set_up, exit) in cases {
        let mut h = Home::new();
        let root = h.brain("alpha");
        set_up(&mut h, &root);
        let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
        assert_eq!(code, exit, "{what}: {doc}");
        let sync = &doc["brains"][0]["sync"];
        if what == "conflict" {
            assert_eq!(sync["conflicts"][0]["path"], "notes/first.md", "{doc}");
        } else {
            assert_eq!(*reason(sync), what, "{doc}");
        }
    }
}

#[test]
fn sync_of_a_brain_with_no_remote_says_how_to_add_one_and_exits_0() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.git(&root, &["remote", "remove", "origin"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["brains"][0]["sync"]["left_as_is"],
        json!({
            "reason": "no_upstream", "paths": [],
            "detail": "add one with git remote add origin <url>, then git push -u origin main",
        }),
        "{doc}"
    );
    let (code, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "alpha: no upstream, nothing changed; add one with git remote add origin <url>, then git push -u origin main\n"
    );
}

#[test]
fn sync_of_a_shallow_clone_is_refused() {
    let mut h = Home::new();
    h.brain("alpha");
    let bare = h.dir.path().join("alpha.git");
    let shallow = h.dir.path().join("shallow");
    let url = file_url(&bare);
    let target = shallow.display().to_string();
    h.git(
        h.dir.path(),
        &["clone", "-q", "--depth", "1", &url, &target],
    );
    h.write(&shallow, "kluris.yml", "name: shallow\ndescription: d\n");
    h.register("shallow", &shallow);
    let (code, doc) = h.json(&["sync", "--brain", "shallow", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert!(
        doc["error"].as_str().unwrap().contains("shallow clone"),
        "{doc}"
    );
}

// --- create --remote -------------------------------------------------------------------

/// `kluris create alpha` in the home, with `--remote url` when given.
fn create(h: &Home, remote: Option<&str>) -> (i32, Value) {
    let home = h.dir.path().display().to_string();
    let mut args = vec!["create", "alpha", "--path", &home, "--json"];
    if let Some(url) = remote {
        args.extend(["--remote", url]);
    }
    h.json(&args)
}

/// An empty bare repository at `<home>/<name>.git`; its `file://` URL.
fn empty_remote(h: &Home, name: &str) -> String {
    let bare = h.dir.path().join(format!("{name}.git"));
    let path = bare.display().to_string();
    h.git(h.dir.path(), &["init", "-q", "--bare", "-b", "main", &path]);
    file_url(&bare)
}

fn upstream_config(h: &Home, root: &Path) -> (String, String) {
    let get = |key: &str| h.git(root, &["config", "--get", key]).trim().to_string();
    (get("branch.main.remote"), get("branch.main.merge"))
}

#[test]
fn create_with_an_empty_remote_sets_the_upstream_and_pushes() {
    let h = Home::new();
    let url = empty_remote(&h, "alpha");
    let (code, doc) = create(&h, Some(&url));
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), CREATE_KEYS, "{doc}");
    let sync = &doc["sync"];
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(sync["to_push"], 0, "{doc}");
    assert_eq!(*reason(sync), Value::Null, "{doc}");
    let root = h.dir.path().join("alpha");
    assert_eq!(
        upstream_config(&h, &root),
        ("origin".to_string(), "refs/heads/main".to_string())
    );
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
    let tracking = h.git(&root, &["rev-parse", "refs/remotes/origin/main"]);
    assert_eq!(tracking.trim(), head(&h, &root));
}

#[test]
fn create_with_a_remote_that_has_commits_is_refused_keeps_the_brain_and_merges_nothing() {
    for branch in ["main", "other"] {
        let h = Home::new();
        let url = empty_remote(&h, "alpha");
        let seed = h.dir.path().join("seed");
        fs::create_dir_all(&seed).unwrap();
        h.git(&seed, &["init", "-q", "-b", branch]);
        h.configure(&seed, "Someone Else", "else@example.com");
        h.commit(&seed, "someone-else.md", "# Someone else's\n");
        h.git(&seed, &["push", "-q", &url, branch]);
        let bare = h.dir.path().join("alpha.git");
        let published = h.git(&bare, &["rev-parse", branch]);

        let (code, doc) = create(&h, Some(&url));
        assert_eq!(code, 0, "{branch}: {doc}");
        let sync = &doc["sync"];
        assert_eq!(*reason(sync), "refused", "{branch}: {doc}");
        assert_eq!(sync["left_as_is"]["detail"], REMOTE_HAS_COMMITS, "{doc}");
        assert_eq!(sync["pushed"], false, "{doc}");
        let root = h.dir.path().join("alpha");
        assert!(root.join("brain.md").is_file(), "{branch}: the brain went");
        let (_, list) = h.json(&["list", "--json"]);
        assert!(list.to_string().contains("alpha"), "{list}");
        let count = h.git(&root, &["rev-list", "--count", "HEAD"]);
        assert_eq!(count.trim(), "1", "{branch}: something was merged");
        assert!(!root.join("someone-else.md").exists(), "{branch}: merged");
        assert_eq!(h.git(&bare, &["rev-parse", branch]), published, "pushed");
        let heads = h.git(
            &bare,
            &["for-each-ref", "--format=%(refname)", "refs/heads"],
        );
        assert_eq!(
            heads.trim(),
            format!("refs/heads/{branch}"),
            "{branch}: pushed"
        );
    }
}

#[cfg(unix)]
#[test]
fn create_offline_keeps_the_brain_and_a_later_read_pushes_it_in_the_background() {
    let h = Home::new();
    let url = empty_remote(&h, "alpha");
    let (code, doc) = create(&h, Some(&offline_url()));
    assert_eq!(code, 0, "{doc}");
    let sync = &doc["sync"];
    assert_eq!(sync["pushed"], false, "{doc}");
    assert_eq!(*reason(sync), "offline", "{doc}");
    let root = h.dir.path().join("alpha");
    assert!(root.join("brain.md").is_file());
    assert_eq!(
        upstream_config(&h, &root),
        ("origin".to_string(), "refs/heads/main".to_string())
    );
    // The network is back: the next read's background push sends it.
    h.git(&root, &["remote", "set-url", "origin", &url]);
    let (code, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    wait_for_background(&root);
    let record = LastPush::read(&root.join(".git").join(LAST_PUSH)).expect("a record");
    assert_eq!(record.result, PushResult::Pushed, "{record:?}");
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
}

#[test]
fn create_refused_access_keeps_the_brain_with_gits_message() {
    let h = Home::new();
    let gone = file_url(&h.dir.path().join("gone.git"));
    let (code, doc) = create(&h, Some(&gone));
    assert_eq!(code, 0, "{doc}");
    let sync = &doc["sync"];
    assert_eq!(*reason(sync), "refused", "{doc}");
    assert_eq!(sync["pushed"], false, "{doc}");
    let detail = sync["left_as_is"]["detail"].as_str().unwrap();
    assert!(detail.ends_with(&the_fix("fetch")), "{detail}");
    assert!(h.dir.path().join("alpha").join("brain.md").is_file());
}

/// The upstream set but its remote-tracking ref absent, as after an
/// offline create: everything from the root is to push, and the push makes
/// the remote's branch.
#[test]
fn an_upstream_without_its_tracking_ref_is_ahead_only_from_the_root() {
    let h = Home::new();
    let url = empty_remote(&h, "alpha");
    let (code, doc) = create(&h, Some(&offline_url()));
    assert_eq!(code, 0, "{doc}");
    let root = h.dir.path().join("alpha");
    h.configure(&root, "Ana Tester", "ana@example.com");
    h.commit(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["sync"]["to_push"], 2, "{doc}");
    h.git(&root, &["remote", "set-url", "origin", &url]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let sync = &doc["brains"][0]["sync"];
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(sync["to_push"], 0, "{doc}");
    assert_eq!(bare_main(&h, "alpha"), head(&h, &root));
    let tracking = h.git(&root, &["rev-parse", "refs/remotes/origin/main"]);
    assert_eq!(tracking.trim(), head(&h, &root));
}

#[test]
fn create_without_a_remote_opens_no_session() {
    let h = Home::new();
    let (code, doc) = create(&h, None);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc.get("sync"), None, "{doc}");
    let root = h.dir.path().join("alpha");
    let mut cmd = std::process::Command::new("git");
    h.hermetic(&mut cmd);
    let out = cmd
        .args(["config", "--get", "branch.main.remote"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!out.status.success(), "an upstream was set");
}
