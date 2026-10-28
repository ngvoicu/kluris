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

/// Every file of the work tree at `root` with its bytes, `.git` aside.
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
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
    files
}

fn snapshot(h: &Home, root: &Path) -> Snapshot {
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
        files: files(root),
    }
}

/// What the work tree at `dir` holds: its HEAD (the branch and its
/// commit), the index's entries, `git status` and every file's bytes.
#[derive(Debug, PartialEq, Eq)]
struct WorkTree {
    head: String,
    index: String,
    status: String,
    files: Vec<(String, Vec<u8>)>,
}

fn work_tree(h: &Home, dir: &Path) -> WorkTree {
    WorkTree {
        head: h.git(dir, &["symbolic-ref", "HEAD"]) + &h.git(dir, &["rev-parse", "HEAD"]),
        index: h.git(dir, &["ls-files", "--stage"]),
        status: h.git(dir, &["status", "--porcelain"]),
        files: files(dir),
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

/// Run `command` in the brain after the remote's upload-pack has sent the
/// pack, while sync's fetch is still running.
fn after_upload_pack(h: &Home, root: &Path, command: &str) {
    let script = h.dir.path().join("racing-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ngit upload-pack \"$@\" || exit $?\ngit -C '{}' {command} >&2\n",
            sh_path(root)
        ),
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

/// What `refused` says of `refs/remotes/origin/main` made a symbolic ref to
/// `target`.
fn symbolic_refusal(target: &str) -> String {
    format!(
        "the upstream's remote-tracking ref refs/remotes/origin/main is a symbolic ref to {target}: moving it would move {target}, so Kluris leaves it alone; remove it with `git symbolic-ref --delete refs/remotes/origin/main` and run kluris sync again"
    )
}

/// The tracking ref turns symbolic, pointing at the checked-out branch,
/// after sync checked it and while its fetch runs: sync checks it again
/// once the fetch is done and refuses the brain before it moves anything.
#[test]
fn a_tracking_ref_made_symbolic_during_the_fetch_is_refused() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    after_upload_pack(
        &h,
        &root,
        "symbolic-ref refs/remotes/origin/main refs/heads/main",
    );
    let before = work_tree(&h, &root);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let entry: &Value = &doc["brains"][0];
    assert_eq!(
        entry["refused"],
        symbolic_refusal("refs/heads/main"),
        "{doc}"
    );
    assert_eq!(entry["merged"], false, "{doc}");
    assert_eq!(work_tree(&h, &root), before);
    let branch_log = h.git(
        &root,
        &["reflog", "show", "--format=%gs", "refs/heads/main"],
    );
    assert!(!branch_log.contains("kluris sync"), "{branch_log}");
}

/// A brain with an approved local commit (and, when `changed`, a
/// teammate's commit on the remote), and a branch `private` checked out in
/// a second worktree at `<home>/private`, holding an unapproved
/// `notes/private.md`. Returns the brain's root and that worktree.
fn private_worktree(h: &mut Home, changed: bool) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = h.brain("alpha");
    h.commit(&root, "notes/local.md", &neuron("Local", "Approved.\n"));
    if changed {
        h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    }
    let remote = remote_head(h);
    h.git(
        &root,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--refmap=",
            "origin",
            "refs/heads/main",
        ],
    );
    h.git(&root, &["branch", "private", &remote]);
    let linked = h.dir.path().join("private");
    h.git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            &linked.to_string_lossy(),
            "private",
        ],
    );
    h.commit(
        &linked,
        "notes/private.md",
        &neuron("Private", "PRIVATE: NOT APPROVED FOR ORIGIN\n"),
    );
    (root, linked)
}

/// Sync refuses the brain, naming the tracking ref and `refs/heads/private`;
/// the remote never gets `notes/private.md`, and `private`, its worktree
/// and the brain's own work tree stay byte for byte as they were. Returns
/// the brain's entry.
fn assert_private_not_published(h: &Home, root: &Path, linked: &Path) -> Value {
    let bare = h.dir.path().join("alpha.git");
    let remote_before = remote_head(h);
    let main_before = work_tree(h, root);
    let private_before = work_tree(h, linked);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let entry = doc["brains"][0].clone();
    assert_eq!(
        entry["refused"],
        symbolic_refusal("refs/heads/private"),
        "{doc}"
    );
    assert_eq!(entry["pushed"], false, "{doc}");
    assert_eq!(remote_head(h), remote_before);
    let published = h.git(&bare, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(!published.contains("notes/private.md"), "{published}");
    assert_eq!(work_tree(h, linked), private_before);
    assert_eq!(work_tree(h, root), main_before);
    entry
}

/// The tracking ref turns into a symbolic ref to a branch checked out in
/// another worktree while sync's fetch runs, and the fetch brings a new
/// commit: the compare-and-swap fails, and sync refuses the brain rather
/// than take the ref's new value for a concurrent fetch.
#[test]
fn a_tracking_ref_made_symbolic_to_another_branch_during_a_fetch_that_moved_is_refused() {
    let mut h = Home::new();
    let (root, linked) = private_worktree(&mut h, true);
    after_upload_pack(
        &h,
        &root,
        "symbolic-ref refs/remotes/origin/main refs/heads/private",
    );
    let entry = assert_private_not_published(&h, &root, &linked);
    assert_eq!(entry["merged"], false);
}

/// The same while the fetch brings nothing new, so there is no ref to
/// move: sync still checks the ref after the fetch and refuses the brain.
#[test]
fn a_tracking_ref_made_symbolic_to_another_branch_during_a_fetch_with_nothing_new_is_refused() {
    let mut h = Home::new();
    let (root, linked) = private_worktree(&mut h, false);
    after_upload_pack(
        &h,
        &root,
        "symbolic-ref refs/remotes/origin/main refs/heads/private",
    );
    let entry = assert_private_not_published(&h, &root, &linked);
    assert_eq!(entry["merged"], false);
}

/// The tracking ref turns symbolic while sync merges (a `pre-merge-commit`
/// hook does it): the merge lands, and sync checks the ref again right
/// before the push and refuses the brain, so git's own update of the
/// tracking ref after a push never follows it.
#[test]
fn a_tracking_ref_made_symbolic_during_the_merge_is_refused_before_the_push() {
    let mut h = Home::new();
    let (root, linked) = private_worktree(&mut h, true);
    let hook = root.join(".git").join("hooks").join("pre-merge-commit");
    fs::write(
        &hook,
        "#!/bin/sh\ngit symbolic-ref refs/remotes/origin/main refs/heads/private\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bare = h.dir.path().join("alpha.git");
    let remote_before = remote_head(&h);
    let private_before = work_tree(&h, &linked);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let entry = &doc["brains"][0];
    assert_eq!(
        entry["refused"],
        symbolic_refusal("refs/heads/private"),
        "{doc}"
    );
    assert_eq!(entry["merged"], true, "{doc}");
    assert_eq!(entry["pushed"], false, "{doc}");
    assert_eq!(remote_head(&h), remote_before);
    let published = h.git(&bare, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(!published.contains("notes/private.md"), "{published}");
    assert_eq!(work_tree(&h, &linked), private_before);
    assert!(root.join("notes").join("remote.md").is_file());
    assert!(!root.join("notes").join("private.md").exists());
    assert_eq!(h.git(&root, &["status", "--porcelain"]), "");
}

/// A remote that has no such branch: sync goes on from what the tracking
/// ref held before the fetch.
#[test]
fn a_remote_without_the_branch_goes_on_from_the_tracking_ref() {
    let mut h = Home::new();
    h.brain("alpha");
    let bare = h.dir.path().join("alpha.git");
    h.git(&bare, &["update-ref", "-d", "refs/heads/main"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let entry = &doc["brains"][0];
    assert_eq!(entry["fetched"], true, "{doc}");
    assert_eq!(entry["merged"], false, "{doc}");
    assert_eq!(entry["pushed"], false, "{doc}");
    assert_eq!(entry["refused"], Value::Null, "{doc}");
}

/// A remote that has no such branch, and no tracking ref either: nothing
/// to sync from, an error naming the ref.
#[test]
fn a_remote_without_the_branch_and_no_tracking_ref_is_an_error() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let bare = h.dir.path().join("alpha.git");
    h.git(&bare, &["update-ref", "-d", "refs/heads/main"]);
    h.git(&root, &["update-ref", "-d", "refs/remotes/origin/main"]);
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap_or_else(|| panic!("{doc}"));
    assert!(error.contains("refs/remotes/origin/main"), "{error}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]), head);
}
