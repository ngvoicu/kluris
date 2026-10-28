//! `kluris sync`'s fetch and push stopped at the network time limit. The
//! fetch writes no ref, so it leaves no ref lock, on Windows (where the stop
//! is `TerminateJobObject`, so git cleans up nothing) as on Unix; a push
//! stopped while git moves the tracking ref is offline, and a lock it
//! leaves is reported by the next sync. A concurrent `git fetch` is never
//! overwritten.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kluris_core::sync::{SyncReport, sync};

mod common;

use common::*;

/// The bound these tests give each fetch and push.
const LIMIT: Duration = Duration::from_secs(3);

/// A `reference-transaction` hook that sleeps in the `prepared` state (git
/// holds the ref's lock then) whenever the transaction moves a
/// remote-tracking ref: well past [`LIMIT`].
const SLOW_TRACKING_HOOK: &str =
    "[ \"$1\" = prepared ] || exit 0\nif grep -q ' refs/remotes/'; then sleep 8; fi\nexit 0";

fn sync_within(w: &World, limit: Duration) -> SyncReport {
    let mut request = w.request();
    request.network_time_limit = limit;
    sync(&w.ours, &w.git(), &request).unwrap()
}

/// Every `*.lock` file under `.git/refs`, and `.git/packed-refs.lock`.
fn ref_locks(at: &Path) -> Vec<PathBuf> {
    let git = at.join(".git");
    let mut found = Vec::new();
    let packed = git.join("packed-refs.lock");
    if fs::symlink_metadata(&packed).is_ok() {
        found.push(packed);
    }
    let mut stack = vec![git.join("refs")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lock") {
                found.push(path);
            }
        }
    }
    found
}

fn tracking(w: &World) -> String {
    w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"])
        .trim()
        .to_string()
}

/// A shell path for a script (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[test]
fn a_fetch_stopped_while_git_would_hold_the_tracking_ref_leaves_no_lock() {
    // The fetch inside the bound writes no ref, so the slow hook runs under
    // Kluris's own update of the tracking ref, outside the bound.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.hook(&w.ours, "reference-transaction", SLOW_TRACKING_HOOK);
    let r = sync_within(&w, LIMIT);
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new(), "{r:?}");
    assert!(r.fetched && r.merged && !r.offline, "{r:?}");
    assert!(r.refused.is_none(), "{r:?}");
    assert_eq!(tracking(&w), w.remote_head());
    assert_eq!(w.head(&w.ours), w.remote_head());
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
}

#[test]
fn a_push_stopped_while_git_moves_the_tracking_ref_is_offline_and_its_lock_is_reported() {
    // git moves the tracking ref itself once the remote took the push, and
    // nothing turns that off. Stopped then, the push is offline; a
    // terminated job on Windows leaves git's lock, which the next sync
    // reports and Kluris never removes.
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    w.hook(&w.ours, "reference-transaction", SLOW_TRACKING_HOOK);
    let r = sync_within(&w, LIMIT);
    assert!(r.fetched && !r.pushed && r.offline, "{r:?}");
    assert_eq!(w.remote_head(), pushed, "the remote took the push");
    fs::remove_file(
        w.ours
            .join(".git")
            .join("hooks")
            .join("reference-transaction"),
    )
    .unwrap();
    let lock = w
        .ours
        .join(".git")
        .join("refs")
        .join("remotes")
        .join("origin")
        .join("main.lock");
    if cfg!(windows) {
        assert_eq!(ref_locks(&w.ours), vec![lock.clone()]);
        let r = sync_within(&w, LIMIT);
        assert!(!r.fetched && !r.offline && !r.pushed, "{r:?}");
        let why = r.refused.unwrap_or_default();
        assert!(why.starts_with("a ref is locked: "), "{why}");
        assert!(why.contains("main.lock"), "{why}");
        assert!(
            why.contains("if no git is running in this brain, remove that file"),
            "{why}"
        );
        assert!(lock.is_file(), "kluris removed the lock");
        fs::remove_file(&lock).unwrap();
    } else {
        // TERM: git removed its own lock.
        assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
    }
    let r = sync_within(&w, LIMIT);
    assert!(r.fetched && !r.offline && r.refused.is_none(), "{r:?}");
    assert_eq!(tracking(&w), pushed);
    assert_eq!(w.head(&w.ours), pushed);
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
}

#[test]
fn a_fetch_held_past_the_bound_by_a_slow_upload_pack_leaves_no_lock() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let slow = w.dir.path().join("slow-upload-pack.sh");
    fs::write(&slow, "#!/bin/sh\nsleep 10\nexec git upload-pack \"$@\"\n").unwrap();
    let slow = sh_path(&slow);
    w.run(
        &w.ours,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{slow}'"),
        ],
    );
    let before = tracking(&w);
    let started = Instant::now();
    let r = sync_within(&w, LIMIT);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(8), "{took:?}");
    assert!(r.offline && !r.fetched && !r.merged, "{r:?}");
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
    assert_eq!(tracking(&w), before);
    // The remote fast again: the next sync fetches and merges.
    w.run(&w.ours, &["config", "--unset", "remote.origin.uploadpack"]);
    let r = sync_within(&w, LIMIT);
    assert!(r.fetched && r.merged && !r.offline, "{r:?}");
    assert_eq!(tracking(&w), w.remote_head());
    assert_eq!(w.head(&w.ours), w.remote_head());
}

#[test]
fn a_concurrent_fetch_that_moved_the_tracking_ref_is_kept() {
    // While Kluris's fetch waits for its upload-pack to end, the teammate
    // pushes again and a plain `git fetch` in the brain moves the tracking
    // ref past what Kluris fetched: Kluris's compare-and-swap fails, and it
    // goes on from where that fetch left the ref.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.write(&w.theirs, "ideas/later.md", &neuron("", "# Later\n"));
    w.commit(&w.theirs, &["ideas/later.md"]);
    let script = w.dir.path().join("racing-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ -n \"$RACING_FETCH\" ] && exec git upload-pack \"$@\"\ngit upload-pack \"$@\" || exit $?\ngit -C '{theirs}' push -q origin main >&2\nRACING_FETCH=1 git -C '{ours}' fetch -q origin >&2\n",
            theirs = sh_path(&w.theirs),
            ours = sh_path(&w.ours),
        ),
    )
    .unwrap();
    w.run(
        &w.ours,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
    let r = sync_within(&w, Duration::from_secs(60));
    let latest = w.remote_head();
    assert_eq!(w.head(&w.theirs), latest, "the teammate's second push");
    assert!(r.fetched && r.merged && !r.offline, "{r:?}");
    assert!(r.refused.is_none(), "{r:?}");
    assert_eq!(
        tracking(&w),
        latest,
        "the concurrent fetch's ref was moved back"
    );
    let reflog = w.run(&w.ours, &["reflog", "show", "refs/remotes/origin/main"]);
    assert!(!reflog.contains("kluris sync"), "{reflog}");
    assert_eq!(w.head(&w.ours), latest);
    assert!(w.read(&w.ours, "ideas/later.md").contains("# Later"));
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
}

#[test]
fn moving_the_tracking_ref_replaces_a_symbolic_ref_and_never_follows_it() {
    use kluris_core::git::Git;
    use kluris_core::sync::plumbing::SyncGit;
    let w = World::new();
    let git = w.git();
    let repo = git.repo(&w.ours).unwrap();
    let main = w.head(&w.ours);
    let tree = w.run(&w.ours, &["rev-parse", "HEAD^{tree}"]);
    let other = w
        .run(
            &w.ours,
            &["commit-tree", tree.trim(), "-p", &main, "-m", "other"],
        )
        .trim()
        .to_string();
    w.run(
        &w.ours,
        &[
            "symbolic-ref",
            "refs/remotes/origin/main",
            "refs/heads/main",
        ],
    );
    git.move_ref(
        &repo,
        "refs/remotes/origin/main",
        &other,
        Some(&main),
        "test",
    )
    .unwrap();
    assert_eq!(
        w.run(&w.ours, &["rev-parse", "refs/heads/main"]).trim(),
        main
    );
    assert_eq!(tracking(&w), other);
    let symbolic = w
        .command(
            &w.ours,
            &["symbolic-ref", "--quiet", "refs/remotes/origin/main"],
        )
        .output()
        .unwrap();
    assert_eq!(symbolic.status.code(), Some(1), "still symbolic");
}
