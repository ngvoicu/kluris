//! What a pull decides: behind only, ahead only, diverged, nothing to do,
//! no upstream, no tracking ref yet, and the remote claim held elsewhere.

use kluris_core::sync::LeftReason;
use kluris_core::sync::claim::RemoteClaim;

use crate::common::*;
use crate::diverged;

#[test]
fn nothing_to_pull_or_push_changes_nothing() {
    let w = World::new();
    let before = w.snapshot();
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!(
        (r.pulled, r.to_push, r.pushed, r.behind_conflict),
        (0, 0, false, 0)
    );
    assert!(r.auto_merged.is_empty());
    assert_eq!(w.snapshot(), before);
}

#[test]
fn behind_only_fast_forwards_and_counts_the_commits_pulled() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.theirs_push(&[("ideas/other.md", &neuron("", "# Other\n"))], &[]);
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.to_push, r.pushed), (2, 0, false), "{r:?}");
    assert_eq!(w.head(&w.ours), w.remote_head());
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "ideas/map.md").contains("other.md"));
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn ahead_only_pulls_nothing_and_counts_what_to_push() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.ours_commit(&[("notes/third.md", &neuron("", "# Third\n"))], &[]);
    let (head, remote) = (w.head(&w.ours), w.remote_head());
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.to_push, r.pushed), (0, 2, false), "{r:?}");
    assert_eq!((w.head(&w.ours), w.remote_head()), (head.clone(), remote));
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.to_push, r.pushed), (0, 0, true), "{r:?}");
    assert_eq!(w.remote_head(), head);
}

#[test]
fn a_pull_lands_the_merge_and_leaves_it_to_push() {
    let w = World::new();
    diverged(&w);
    let remote = w.remote_head();
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    // Our commit and the merge commit wait for the push.
    assert_eq!((r.pulled, r.to_push, r.pushed), (1, 2, false), "{r:?}");
    let head = w.head(&w.ours);
    assert_eq!(w.parents(&w.ours, &head), 2);
    assert_eq!(w.run(&w.ours, &["rev-parse", "HEAD^2"]).trim(), remote);
    assert_eq!(w.remote_head(), remote);
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn no_upstream_is_left_as_is_with_the_fix() {
    let w = World::new();
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::NoUpstream), "{r:?}");
    assert_eq!(detail(&r), "set one with git push -u origin main");
    assert_eq!((r.pulled, r.to_push, r.pushed), (0, 0, false), "{r:?}");
    assert_eq!(w.snapshot(), before);
}

/// The remote has no such branch and there is no tracking ref: every commit
/// from the root is ours to push, and the push makes the branch.
#[test]
fn no_tracking_ref_yet_is_ahead_only_from_the_root_and_the_push_makes_the_branch() {
    let w = World::new();
    w.run(&w.remote, &["update-ref", "-d", "refs/heads/main"]);
    w.run(&w.ours, &["update-ref", "-d", "refs/remotes/origin/main"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let head = w.head(&w.ours);
    let from_root: usize = w
        .run(&w.ours, &["rev-list", "--count", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    assert_eq!(
        (r.pulled, r.to_push, r.pushed),
        (0, from_root, false),
        "{r:?}"
    );
    assert_eq!(w.head(&w.ours), head);
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.to_push, r.pushed), (0, true), "{r:?}");
    assert_eq!(w.remote_head(), head);
    assert_eq!(
        w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"])
            .trim(),
        head
    );
}

/// Another process holds the remote claim: the pull skips the fetch and
/// pulls from what is already fetched; the push waits for the next run.
#[test]
fn a_held_remote_claim_skips_the_network_and_pulls_what_is_fetched() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let tracking = w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"]);
    let claim = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    assert_eq!(r.pulled, 0, "{r:?}");
    assert_eq!(
        w.run(&w.ours, &["rev-parse", "refs/remotes/origin/main"]),
        tracking,
        "fetched while the claim was held"
    );
    // What a git fetch brought meanwhile is pulled.
    w.run(&w.ours, &["fetch", "-q"]);
    let r = w.pull();
    assert!(clean(&r), "{r:?}");
    assert_eq!(r.pulled, 1, "{r:?}");
    assert_eq!(w.head(&w.ours), w.remote_head());

    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let remote = w.remote_head();
    let r = w.sync();
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert_eq!((r.to_push, r.pushed), (1, false), "{r:?}");
    assert_eq!(w.remote_head(), remote);
    drop(claim);
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
}
