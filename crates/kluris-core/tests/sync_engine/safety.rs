//! Never half-done: uncommitted files in the way, an edit made before the
//! land, a git operation in progress, locks, a git that stops partway,
//! unrelated histories, linked worktrees, and the brains the engine does not
//! open.

use std::fs;

use kluris_core::git::Git;
use kluris_core::sync::claim::RemoteClaim;
#[cfg(unix)]
use kluris_core::sync::engine::check_git;
use kluris_core::sync::engine::open;
use kluris_core::sync::git::SyncGit;
use kluris_core::sync::{LeftReason, PENDING, SyncError};

use crate::common::*;

// --- uncommitted files ---------------------------------------------------------------

#[test]
fn incoming_changes_to_an_uncommitted_file_leave_the_brain_as_is() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.write(
        &w.ours,
        "notes/first.md",
        &base.replace("Five.", "Five, mine (uncommitted)."),
    );
    w.write(
        &w.ours,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n- mine\n",
    );
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    let left = r.left_as_is.as_ref().unwrap();
    assert_eq!(left.paths, ["notes/first.md"]);
    assert_eq!(left.detail, None);
    assert_eq!((r.pulled, r.pushed), (0, false));
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_untracked_file_in_the_way_leaves_the_brain_as_is() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.write(&w.ours, "ideas/new.md", "# my own draft\n");
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    assert_eq!(r.left_as_is.unwrap().paths, ["ideas/new.md"]);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_ignored_file_in_the_way_leaves_the_brain_as_is() {
    let w = World::new();
    w.theirs_push(&[(".gitignore", "kluris.yml\nnotes/local.md\n")], &[]);
    w.run(&w.ours, &["pull", "-q"]);
    // The teammate stops ignoring it and commits theirs; ours is ignored here.
    w.write(&w.ours, "notes/local.md", "# my local note\n");
    w.write(&w.theirs, ".gitignore", "kluris.yml\n");
    w.write(&w.theirs, "notes/local.md", &neuron("", "# Local\n"));
    w.run(&w.theirs, &["add", "-A"]);
    w.run(&w.theirs, &["commit", "-q", "-m", "unignore"]);
    w.run(&w.theirs, &["push", "-q"]);
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    assert_eq!(r.left_as_is.unwrap().paths, ["notes/local.md"]);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn uncommitted_files_the_changes_do_not_touch_stay() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let mine = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n- mine\n";
    w.write(&w.ours, "glossary.md", mine);
    w.write(&w.ours, "notes/draft.md", "# draft\n");
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!(r.pulled, 1);
    assert_eq!(w.read(&w.ours, "glossary.md"), mine);
    assert_eq!(w.read(&w.ours, "notes/draft.md"), "# draft\n");
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
}

/// Ours ignores `private/` locally; the teammate starts tracking a file there.
fn ignored_in_the_way(w: &World) {
    fs::write(
        w.ours.join(".git").join("info").join("exclude"),
        "private/\n",
    )
    .unwrap();
    w.theirs_push(&[("private/draft.md", "TEAM VERSION\n")], &[]);
}

/// A file written after the first check (here by a hook) is caught by the
/// land's own check: reported, nothing lost.
#[test]
fn an_ignored_file_written_before_the_land_is_reported_and_kept() {
    let w = World::new();
    ignored_in_the_way(&w);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.hook(
        &w.ours,
        "pre-merge-commit",
        "mkdir -p private; printf 'MINE\\n' > private/draft.md",
    );
    let (head, remote) = (w.head(&w.ours), w.remote_head());
    let r = w.sync();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    assert_eq!(r.left_as_is.as_ref().unwrap().paths, ["private/draft.md"]);
    assert_eq!((r.pulled, r.pushed), (0, false));
    assert_eq!(w.read(&w.ours, "private/draft.md"), "MINE\n");
    assert_eq!((w.head(&w.ours), w.remote_head()), (head, remote));
}

#[test]
fn a_tracked_edit_made_before_the_land_is_reported_even_with_autostash() {
    let w = World::new();
    w.theirs_push(&[("ideas/idea.md", &neuron("", "# Idea, theirs\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.run(&w.ours, &["config", "merge.autostash", "true"]);
    w.hook(
        &w.ours,
        "pre-merge-commit",
        "printf '# MINE, UNCOMMITTED\\n' > ideas/idea.md",
    );
    let (head, remote) = (w.head(&w.ours), w.remote_head());
    let r = w.sync();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    assert_eq!(r.left_as_is.as_ref().unwrap().paths, ["ideas/idea.md"]);
    assert_eq!(w.read(&w.ours, "ideas/idea.md"), "# MINE, UNCOMMITTED\n");
    assert_eq!(w.run(&w.ours, &["stash", "list"]), "");
    assert_eq!((w.head(&w.ours), w.remote_head()), (head, remote));
    no_markers_anywhere(&w, &w.ours);
}

/// The fast-forward itself: an ignored file in the way and a local edit
/// both stop it, whatever merge.autostash says; nothing is stashed.
#[test]
fn the_fast_forward_never_overwrites_an_ignored_file_or_stashes() {
    let w = World::new();
    ignored_in_the_way(&w);
    w.run(&w.ours, &["fetch", "-q"]);
    w.write(&w.ours, "private/draft.md", "MINE\n");
    let git = w.git();
    let repo = git.repo(&w.ours).unwrap();
    let head = w.head(&w.ours);
    assert!(git.fast_forward_to(&repo, "origin/main").is_err());
    assert_eq!(w.read(&w.ours, "private/draft.md"), "MINE\n");
    assert_eq!(w.head(&w.ours), head);

    fs::remove_dir_all(w.ours.join("private")).unwrap();
    w.theirs_push(&[("ideas/idea.md", &neuron("", "# Idea, theirs\n"))], &[]);
    w.run(&w.ours, &["fetch", "-q"]);
    w.run(&w.ours, &["config", "merge.autostash", "true"]);
    w.write(&w.ours, "ideas/idea.md", "# mine\n");
    assert!(git.fast_forward_to(&repo, "origin/main").is_err());
    assert_eq!(w.read(&w.ours, "ideas/idea.md"), "# mine\n");
    assert_eq!(w.run(&w.ours, &["stash", "list"]), "");
    assert_eq!(w.head(&w.ours), head);
}

// --- a git operation, locks, a git that stops ---------------------------------------

#[test]
fn a_merge_in_progress_leaves_the_brain_as_is() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.run(&w.ours, &["checkout", "-q", "-b", "side"]);
    w.ours_commit(&[("notes/side.md", &neuron("", "# Side\n"))], &[]);
    w.run(&w.ours, &["checkout", "-q", "main"]);
    w.run(
        &w.ours,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--no-commit",
            "-s",
            "ours",
            "side",
        ],
    );
    let before = w.snapshot();
    let tracking = w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"]);
    let r = w.sync();
    assert!(left(&r, LeftReason::InProgress), "{r:?}");
    assert_eq!(
        detail(&r),
        "a merge is in progress; finish or abort it first"
    );
    assert_eq!((r.pulled, r.pushed), (0, false));
    assert_eq!(w.snapshot(), before);
    assert_eq!(
        w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"]),
        tracking,
        "fetched during a merge"
    );
}

/// A lock git holds while it writes, found before the land: nothing is
/// written, so no incoming file is ever staged under the old HEAD.
#[test]
fn the_branch_ref_lock_or_index_lock_leaves_the_brain_busy() {
    for lock in [["refs", "heads", "main.lock"].as_slice(), &["index.lock"]] {
        let w = World::new();
        w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
        let path = lock
            .iter()
            .fold(w.ours.join(".git"), |path, part| path.join(part));
        fs::write(&path, "").unwrap();
        let before = w.snapshot();
        let r = w.pull();
        assert!(left(&r, LeftReason::Busy), "{lock:?}: {r:?}");
        let name = lock.join("/");
        assert!(detail(&r).contains(&name), "{lock:?}: {}", detail(&r));
        assert_eq!(r.pulled, 0, "{lock:?}");
        assert_eq!(w.snapshot(), before, "{lock:?}");
        assert!(path.is_file(), "{lock:?}: the lock was removed");
        fs::remove_file(&path).unwrap();
        let r = w.pull();
        assert!(clean(&r) && r.pulled == 1, "{lock:?}: {r:?}");
    }
}

/// git stops partway through the land (a folder it cannot write to): the
/// brain is left as git left it, with git's message, and nothing is rolled
/// back.
#[cfg(unix)]
#[test]
fn a_git_that_stops_partway_is_failed_with_its_message() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let ideas = w.ours.join("ideas");
    let head = w.head(&w.ours);
    fs::set_permissions(&ideas, fs::Permissions::from_mode(0o555)).unwrap();
    let r = w.pull();
    fs::set_permissions(&ideas, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(left(&r, LeftReason::Failed), "{r:?}");
    assert!(detail(&r).contains("Permission denied"), "{}", detail(&r));
    assert_eq!(r.pulled, 0);
    assert_eq!(w.head(&w.ours), head);
}

/// A `reference-transaction` hook that stops the branch's update after git
/// wrote the work tree and the index, running `before` first.
fn stop_the_branch_update(w: &World, before: &str) {
    w.hook(
        &w.ours,
        "reference-transaction",
        &format!(
            "[ \"$1\" = prepared ] || exit 0\nif grep -q ' refs/heads/main$'; then {before}\nexit 1; fi\nexit 0"
        ),
    );
}

/// git wrote the incoming files, then stopped: what it wrote is not in the
/// way, so the land failed, with git's message, and nothing is rolled back.
#[test]
fn files_git_wrote_before_it_stopped_are_failed_not_uncommitted() {
    let w = World::new();
    let new = neuron("", "# New\n");
    w.theirs_push(&[("ideas/new.md", &new)], &[]);
    let head = w.head(&w.ours);
    stop_the_branch_update(&w, ":");
    let r = w.pull();
    assert!(left(&r, LeftReason::Failed), "{r:?}");
    // git's message: "update aborted by the reference-transaction hook",
    // "ref updates aborted by hook" in older versions.
    assert!(
        detail(&r).contains("aborted by") && detail(&r).contains("hook"),
        "{}",
        detail(&r)
    );
    assert_eq!(r.pulled, 0);
    assert_eq!(w.head(&w.ours), head);
    assert_eq!(w.read(&w.ours, "ideas/new.md"), new, "git's write was kept");
}

/// git stopped while the user wrote an incoming path: bytes that are
/// neither HEAD's nor the incoming commit's are the user's.
#[test]
fn an_edit_made_while_git_stopped_partway_is_uncommitted() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    stop_the_branch_update(&w, "printf 'MINE\\n' > ideas/new.md");
    let r = w.pull();
    assert!(left(&r, LeftReason::Uncommitted), "{r:?}");
    assert_eq!(r.left_as_is.as_ref().unwrap().paths, ["ideas/new.md"]);
    assert_eq!(w.read(&w.ours, "ideas/new.md"), "MINE\n");
}

/// A conversion git applies when it writes a file: its name, how to set it
/// up in "ours", and what the written `ideas/new.md` then holds that its
/// blob does not.
type Conversion = (&'static str, fn(&World), &'static str);

/// The conversions: line endings by config and by attribute, and a filter
/// with and without its clean side.
const CONVERSIONS: [Conversion; 4] = [
    (
        "core.autocrlf",
        |w| {
            w.run(&w.ours, &["config", "core.autocrlf", "true"]);
        },
        "\r\n",
    ),
    (
        "eol=crlf",
        |w| {
            fs::write(
                w.ours.join(".git").join("info").join("attributes"),
                "* text eol=crlf\n",
            )
            .unwrap();
        },
        "\r\n",
    ),
    (
        "smudge and clean filters",
        |w| {
            fs::write(
                w.ours.join(".git").join("info").join("attributes"),
                "*.md filter=mark\n",
            )
            .unwrap();
            w.run(&w.ours, &["config", "filter.mark.smudge", "sed 's/^/S:/'"]);
            w.run(&w.ours, &["config", "filter.mark.clean", "sed 's/^S://'"]);
        },
        "S:",
    ),
    (
        "a smudge filter alone",
        |w| {
            fs::write(
                w.ours.join(".git").join("info").join("attributes"),
                "*.md filter=mark\n",
            )
            .unwrap();
            w.run(&w.ours, &["config", "filter.mark.smudge", "sed 's/^/S:/'"]);
        },
        "S:",
    ),
];

/// What git wrote through a conversion (line endings, a filter) before it
/// stopped is still git's: the land failed. An edit made at that moment is
/// still the user's.
#[test]
fn files_git_wrote_through_a_conversion_before_it_stopped_are_failed() {
    for (name, convert, converted) in CONVERSIONS {
        // One world at a time: each holds the worlds' lock until dropped.
        {
            let w = World::new();
            w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
            convert(&w);
            stop_the_branch_update(&w, ":");
            let r = w.pull();
            assert!(
                w.read(&w.ours, "ideas/new.md").contains(converted),
                "{name}: git wrote no converted file, which proves nothing"
            );
            assert!(left(&r, LeftReason::Failed), "{name}: {r:?}");
        }
        let w = World::new();
        w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
        convert(&w);
        stop_the_branch_update(&w, "printf 'MINE\\n' > ideas/new.md");
        let r = w.pull();
        assert!(left(&r, LeftReason::Uncommitted), "{name}: {r:?}");
        assert_eq!(
            r.left_as_is.as_ref().unwrap().paths,
            ["ideas/new.md"],
            "{name}"
        );
    }
}

/// With `core.symlinks` false git writes an incoming symlink as a plain
/// file holding its target: that file is git's write too. The symlink is
/// committed by its blob, so no symlink is made on disk.
#[test]
fn a_symlink_git_wrote_as_a_plain_file_before_it_stopped_is_failed() {
    let w = World::new();
    let target = w.dir.path().join("target");
    fs::write(&target, "keep.md").unwrap();
    let oid = w.run(&w.theirs, &["hash-object", "-w", &target.to_string_lossy()]);
    let entry = format!("120000,{},notes/link.md", oid.trim());
    w.run(&w.theirs, &["update-index", "--add", "--cacheinfo", &entry]);
    w.run(&w.theirs, &["commit", "-q", "-m", "a link"]);
    w.run(&w.theirs, &["push", "-q"]);
    w.run(&w.ours, &["config", "core.symlinks", "false"]);
    stop_the_branch_update(&w, ":");
    let r = w.pull();
    let link = w.ours.join("notes").join("link.md");
    assert!(
        link.symlink_metadata().unwrap().is_file(),
        "git made a symlink"
    );
    assert_eq!(w.read(&w.ours, "notes/link.md"), "keep.md");
    assert!(left(&r, LeftReason::Failed), "{r:?}");
}

/// git wrote the first incoming files, then could not write into a folder.
#[cfg(unix)]
#[test]
fn a_folder_git_cannot_write_after_its_first_writes_is_failed() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    let first = neuron("", "# First lobe\n");
    w.theirs_push(
        &[
            ("aaa/first.md", &first),
            ("notes/new.md", &neuron("", "# New\n")),
        ],
        &[],
    );
    let notes = w.ours.join("notes");
    let head = w.head(&w.ours);
    fs::set_permissions(&notes, fs::Permissions::from_mode(0o555)).unwrap();
    let r = w.pull();
    fs::set_permissions(&notes, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        w.read(&w.ours, "aaa/first.md"),
        first,
        "git wrote nothing first"
    );
    assert!(left(&r, LeftReason::Failed), "{r:?}");
    assert!(detail(&r).contains("Permission denied"), "{}", detail(&r));
    assert_eq!(w.head(&w.ours), head);
}

#[test]
fn unrelated_histories_are_refused_before_any_merge() {
    let w = World::new();
    w.run(&w.theirs, &["checkout", "-q", "--orphan", "fresh"]);
    w.write(&w.theirs, "notes/fresh.md", &neuron("", "# Fresh\n"));
    w.run(&w.theirs, &["add", "-A"]);
    w.run(&w.theirs, &["commit", "-q", "-m", "a new history"]);
    w.run(&w.theirs, &["push", "-q", "-f", "origin", "fresh:main"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::Refused), "{r:?}");
    assert!(detail(&r).contains("no commit in common"), "{}", detail(&r));
    assert_eq!((r.pulled, r.pushed), (0, false));
    assert!(!w.ours.join(".git").join(PENDING).exists());
    assert_eq!(w.snapshot(), before);
}

// --- claims and worktrees -------------------------------------------------------------

/// Linked worktrees share the remote and its claim (the common git dir);
/// each keeps its own waiting merge (its own git dir).
#[test]
fn a_linked_worktree_shares_the_remote_claim_and_keeps_its_own_waiting_merge() {
    let w = World::new();
    let linked = w.dir.path().join("linked");
    let to = linked.to_string_lossy().into_owned();
    w.run(
        &w.ours,
        &["worktree", "add", "-q", "-b", "side", &to, "main"],
    );
    w.run(
        &linked,
        &["branch", "-q", "--set-upstream-to", "origin/main"],
    );
    let base = w.read(&linked, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("Three.", "Three, theirs."))],
        &[],
    );
    w.write(
        &linked,
        "notes/first.md",
        &base.replace("Three.", "Three, side."),
    );
    w.commit(&linked, &["notes/first.md"]);

    let claim = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let r = read_session(&linked, &w.git()).expect("a report");
    assert!(
        left(&r, LeftReason::Busy),
        "pulled under a held claim: {r:?}"
    );
    drop(claim);

    let r = w.pull_at(&linked, &w.git());
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    let own = w.run(&linked, &["rev-parse", "--absolute-git-dir"]);
    assert!(std::path::Path::new(own.trim()).join(PENDING).is_file());
    assert!(!w.ours.join(".git").join(PENDING).exists());
    // The main worktree pulls on its own, untouched by the linked one's
    // waiting merge.
    let r = w.pull();
    assert!(clean(&r) && r.pulled == 1, "{r:?}");
}

#[test]
fn a_brain_nested_in_a_larger_repo_is_not_opened() {
    let w = World::new();
    fs::create_dir_all(w.ours.join("sub")).unwrap();
    match open(&w.ours.join("sub"), &w.git(), &w.request()) {
        Err(SyncError::Nested { .. }) => {}
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn a_brain_without_git_is_not_opened() {
    let w = World::new();
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        open(&plain, &w.git(), &w.request()),
        Err(SyncError::NotARepo)
    ));
}

#[cfg(unix)]
#[test]
fn an_older_git_is_a_clean_error() {
    use kluris_core::git::SystemGit;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let fake = dir.path().join("git");
    fs::write(&fake, "#!/bin/sh\necho 'git version 2.37.9'\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    match check_git(&SystemGit::new().program(&fake)) {
        Err(e @ SyncError::GitTooOld { .. }) => assert!(e.to_string().contains("2.38"), "{e}"),
        other => panic!("{other:?}"),
    }
}
