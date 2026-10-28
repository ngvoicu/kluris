//! `kluris sync` and `kluris resolve` read their own fetch's line in
//! `FETCH_HEAD`, over real git. Another `git fetch` in the brain, of a
//! remote holding commits nobody approved for origin, runs while sync's
//! fetch does (the remote's upload-pack starts it):
//! - when sync's fetch has objects to receive, git appends sync's line
//!   after the other fetch's, and sync takes the line of its remote's URL;
//! - when it has none, git writes sync's line first and the other fetch
//!   rewrites the file: sync finds no line of its own and fetches again,
//!   and refuses the brain when that keeps happening.
//!
//! Either way the private commit never reaches origin, the brain's HEAD,
//! index or files, or a conflict record.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod sync_home;

use sync_home::*;

const PRIVATE: &str = "PRIVATE: NEVER APPROVED FOR ORIGIN";

/// When the other fetch runs.
enum Concurrent {
    /// Right after the remote's upload-pack has served sync's fetch.
    Before,
    /// Once sync's fetch has written its line: on the first fetch only, or
    /// on every one.
    After { every_time: bool },
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

/// A clone of origin at `<home>/vault`, a remote `vault` of the brain, with
/// a commit adding `notes/private.md` (and, when `conflicting`, changing
/// `notes/first.md` against the brain's own change). Its commit.
fn vault(h: &Home, root: &Path, conflicting: bool) -> String {
    let bare = h.dir.path().join("alpha.git");
    let vault = h.dir.path().join("vault");
    h.git(
        h.dir.path(),
        &[
            "clone",
            "-q",
            &bare.to_string_lossy(),
            &vault.to_string_lossy(),
        ],
    );
    h.configure(&vault, "Eve Vault", "eve@example.com");
    h.commit(&vault, "notes/private.md", &neuron("Private", PRIVATE));
    if conflicting {
        h.commit(
            &vault,
            "notes/first.md",
            &neuron("First", &BODY.replace("Three.", "Three, private.")),
        );
    }
    h.git(root, &["remote", "add", "vault", &vault.to_string_lossy()]);
    h.git(&vault, &["rev-parse", "HEAD"]).trim().to_string()
}

/// Origin's upload-pack starts a `git fetch vault refs/heads/main` in the
/// brain, `when`.
fn concurrent_fetch(h: &Home, root: &Path, when: Concurrent) {
    let script = h.dir.path().join("concurrent-upload-pack.sh");
    let fetch_head = root.join(".git").join("FETCH_HEAD");
    let marker = h.dir.path().join("concurrent-fetch-ran");
    let fetch = format!(
        "git -C '{}' fetch -q --no-tags --refmap= vault refs/heads/main >&2\n",
        sh_path(root)
    );
    let body = match when {
        Concurrent::Before => fetch,
        Concurrent::After { every_time } => {
            let once = if every_time {
                String::new()
            } else {
                format!(
                    "[ -e '{m}' ] && exit 0\ntouch '{m}'\n",
                    m = sh_path(&marker)
                )
            };
            format!(
                "{once}i=0\nwhile ! grep -q '{oid}' '{fh}' 2>/dev/null; do\n  i=$((i+1)); [ $i -gt 300 ] && break; sleep 0.1\ndone\n{fetch}",
                oid = remote_head(h),
                fh = sh_path(&fetch_head),
            )
        }
    };
    fs::write(
        &script,
        format!("#!/bin/sh\ngit upload-pack \"$@\" || exit $?\n{body}"),
    )
    .unwrap();
    h.git(
        root,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
}

/// The brain already has origin's objects (so sync's fetch receives none)
/// and no `FETCH_HEAD`.
fn prefetched(h: &Home, root: &Path) {
    h.git(
        root,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--refmap=",
            "origin",
            "refs/heads/main",
        ],
    );
    fs::remove_file(root.join(".git").join("FETCH_HEAD")).unwrap();
}

/// `notes/private.md` (and the vault's commit) is in neither origin, the
/// brain's HEAD, index or files, nor a conflict record.
fn assert_not_published(h: &Home, root: &Path, private: &str) {
    let bare = h.dir.path().join("alpha.git");
    let origin = h.git(&bare, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(!origin.contains("notes/private.md"), "origin: {origin}");
    let head = h.git(root, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(!head.contains("notes/private.md"), "HEAD: {head}");
    let index = h.git(root, &["ls-files"]);
    assert!(!index.contains("notes/private.md"), "index: {index}");
    assert!(!root.join("notes").join("private.md").exists());
    let first = h.read(root, "notes/first.md");
    assert!(!first.contains("private"), "{first}");
    let pending = root.join(".git").join("kluris-sync").join("pending.json");
    if let Ok(text) = fs::read_to_string(pending) {
        assert!(!text.contains(private), "{text}");
    }
    let log = h.git(root, &["log", "--format=%H", "HEAD"]);
    assert!(!log.contains(private), "{log}");
}

fn sync(h: &Home) -> (i32, Value) {
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    (code, doc["brains"][0].clone())
}

#[test]
fn a_merge_takes_its_own_remotes_line() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.commit(&root, "notes/local.md", &neuron("Local", "Approved.\n"));
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    assert_eq!(entry["pushed"], true, "{entry}");
    assert!(root.join("notes").join("remote.md").is_file());
    assert_not_published(&h, &root, &private);
}

#[test]
fn a_fast_forward_whose_line_was_rewritten_fetches_again() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = vault(&h, &root, false);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    let (code, entry) = sync(&h);
    assert!(h.dir.path().join("concurrent-fetch-ran").exists());
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]).trim(), remote_head(&h));
    assert_not_published(&h, &root, &private);
}

#[test]
fn a_merge_that_would_conflict_with_the_other_fetch_merges_origin() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.commit(
        &root,
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, ours.")),
    );
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = vault(&h, &root, true);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    let (code, entry) = sync(&h);
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["conflicts"], serde_json::json!([]), "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    assert_eq!(entry["pushed"], true, "{entry}");
    assert_not_published(&h, &root, &private);
}

#[test]
fn resolve_pushes_the_merge_with_origin_only() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.conflict("alpha", &root);
    let (code, entry) = sync(&h);
    assert_eq!(code, 1, "{entry}");
    assert_eq!(entry["conflicts"][0]["path"], "notes/first.md", "{entry}");
    let private = vault(&h, &root, false);
    let _ = fs::remove_file(root.join(".git").join("FETCH_HEAD"));
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--ours",
        "--json",
    ]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true, "{doc}");
    assert_eq!(doc["pushed"], true, "{doc}");
    assert_not_published(&h, &root, &private);
}

/// What the brain's work tree holds: HEAD, the index's entries, `git
/// status` and every file's bytes, `.git` aside.
fn work_tree(h: &Home, root: &Path) -> (String, String, String, Vec<(PathBuf, Vec<u8>)>) {
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
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    (
        h.git(root, &["rev-parse", "HEAD"]),
        h.git(root, &["ls-files", "--stage"]),
        h.git(root, &["status", "--porcelain"]),
        files,
    )
}

#[test]
fn a_line_rewritten_on_every_fetch_refuses_the_brain() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.commit(&root, "notes/local.md", &neuron("Local", "Approved.\n"));
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = vault(&h, &root, false);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: true });
    let before = work_tree(&h, &root);
    let tracking = h.git(&root, &["rev-parse", "refs/remotes/origin/main"]);
    let origin = remote_head(&h);
    let (code, entry) = sync(&h);
    assert_eq!(code, 1, "{entry}");
    assert_eq!(
        entry["refused"],
        "another git fetch in this brain rewrote FETCH_HEAD while Kluris fetched origin, on each try; nothing was merged or pushed: run kluris sync again",
        "{entry}"
    );
    assert_eq!(entry["merged"], false, "{entry}");
    assert_eq!(entry["pushed"], false, "{entry}");
    assert_eq!(work_tree(&h, &root), before);
    assert_eq!(
        h.git(&root, &["rev-parse", "refs/remotes/origin/main"]),
        tracking
    );
    assert_eq!(remote_head(&h), origin);
    assert_not_published(&h, &root, &private);
}
