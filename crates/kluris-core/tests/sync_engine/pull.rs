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

/// A commit on a local branch `private` that the remote has never had.
fn private_commit(w: &World) -> String {
    w.run(&w.ours, &["switch", "-q", "-c", "private"]);
    w.write(
        &w.ours,
        "private-secret.md",
        &neuron("", "# Not for origin\n"),
    );
    w.run(&w.ours, &["add", "-A"]);
    w.run(&w.ours, &["commit", "-q", "-m", "private"]);
    let private = w.head(&w.ours);
    w.run(&w.ours, &["switch", "-q", "main"]);
    private
}

/// How the shared tracking ref comes to name a commit the remote's branch
/// never had: written directly, made symbolic to a local branch, or moved
/// by a fetch of another remote with the same URL whose refspec maps a
/// private branch onto it.
type Rewrite = (&'static str, fn(&World, &str));

const TRACKING_REWRITES: [Rewrite; 3] = [
    ("written directly", |w, private| {
        w.run(
            &w.ours,
            &["update-ref", "refs/remotes/origin/main", private],
        );
    }),
    ("made symbolic", |w, _| {
        w.run(
            &w.ours,
            &[
                "symbolic-ref",
                "refs/remotes/origin/main",
                "refs/heads/private",
            ],
        );
    }),
    ("moved by a same-URL fetch", |w, _| {
        w.run(&w.ours, &["push", "-q", "origin", "private"]);
        let url = w.remote.to_string_lossy().into_owned();
        w.run(&w.ours, &["remote", "add", "vault", &url]);
        let refspec = "+refs/heads/private:refs/remotes/origin/main";
        w.run(
            &w.ours,
            &["config", "--replace-all", "remote.vault.fetch", refspec],
        );
        w.run(&w.ours, &["fetch", "-q", "vault"]);
    }),
];

/// Another process holds the remote claim: a read, which never waits for
/// it, cannot learn the upstream's commit, so it lands nothing, whatever
/// the tracking ref holds, and pushes nothing; once the claim is let go, a
/// sync pulls and pushes.
#[test]
fn a_held_remote_claim_lands_nothing() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.run(&w.ours, &["fetch", "-q"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let claim = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let before = w.snapshot();
    let r = w.read_session().expect("a report");
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert!(detail(&r).contains("remote"), "{}", detail(&r));
    assert_eq!((r.pulled, r.pushed), (0, false), "{r:?}");
    assert_eq!(w.snapshot(), before);
    drop(claim);
    let r = w.sync();
    assert!(clean(&r) && r.pulled == 1 && r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
}

/// The tracking ref names a private commit while another process holds the
/// remote claim: a read adopts nothing, and nothing private is published by
/// a later sync.
#[test]
fn a_held_remote_claim_never_adopts_what_the_tracking_ref_holds() {
    for (how, rewrite) in TRACKING_REWRITES {
        let w = World::new();
        let private = private_commit(&w);
        rewrite(&w, &private);
        let claim = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
        let head = w.head(&w.ours);
        let r = w.read_session().expect("a report");
        drop(claim);
        assert!(left(&r, LeftReason::Busy), "{how}: {r:?}");
        assert_eq!(w.head(&w.ours), head, "{how}: adopted the private commit");
        w.run(
            &w.ours,
            &[
                "update-ref",
                "--no-deref",
                "refs/remotes/origin/main",
                &head,
            ],
        );
        w.sync();
        let published = w.run(&w.remote, &["ls-tree", "-r", "--name-only", "main"]);
        assert!(
            !published.contains("private-secret.md"),
            "{how}: {published}"
        );
    }
}

/// The remote has no such branch: no commit the remote named can land, so
/// a tracking ref left at a private commit is never pulled; the push makes
/// the branch again from this branch's own commit.
#[test]
fn a_remote_without_the_branch_never_adopts_the_tracking_ref() {
    let w = World::new();
    let private = private_commit(&w);
    w.run(&w.remote, &["update-ref", "-d", "refs/heads/main"]);
    w.run(
        &w.ours,
        &["update-ref", "refs/remotes/origin/main", &private],
    );
    let head = w.head(&w.ours);
    let r = w.sync();
    assert!(clean(&r) && r.pulled == 0 && r.pushed, "{r:?}");
    assert_eq!(w.head(&w.ours), head, "adopted the tracking ref's commit");
    assert_eq!(w.remote_head(), head);
}
