//! One branch from start to end: the engine pins the branch HEAD names when
//! it opens a brain, pulls into it and pushes it, and refuses to land when
//! the branch changed meanwhile (a switch, a rebase, a commit).

use kluris_core::sync::LeftReason;
use kluris_core::sync::engine::{self, Pushed};

use crate::common::*;
use crate::sh_path;

/// A branch `side` at the current commit, with its own upstream.
fn side_branch(w: &World) {
    w.run(&w.ours, &["branch", "side"]);
    w.run(&w.ours, &["push", "-q", "origin", "side"]);
    w.run(&w.ours, &["config", "branch.side.remote", "origin"]);
    w.run(&w.ours, &["config", "branch.side.merge", "refs/heads/side"]);
}

/// A `reference-transaction` hook that runs `action` once, when the fetch
/// moves the tracking ref: a change made while the engine fetches.
fn during_the_fetch(w: &World, action: &str) {
    let once = w.dir.path().join("acted");
    w.hook(
        &w.ours,
        "reference-transaction",
        &format!(
            "[ \"$1\" = committed ] || exit 0\ngrep -q ' refs/remotes/origin/main$' || exit 0\n[ -e '{m}' ] && exit 0\n: > '{m}'\n{action}\nexit 0",
            m = sh_path(&once)
        ),
    );
}

#[test]
fn a_switch_to_another_branch_during_the_fetch_lands_nothing() {
    let w = World::new();
    side_branch(&w);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let (main, side) = (w.head(&w.ours), w.head(&w.ours));
    during_the_fetch(&w, "git switch -q side");
    let r = w.pull();
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert!(
        detail(&r).contains("refs/heads/side") && detail(&r).contains("refs/heads/main"),
        "{}",
        detail(&r)
    );
    assert_eq!(r.pulled, 0);
    assert_eq!(w.run(&w.ours, &["rev-parse", "side"]).trim(), side);
    assert_eq!(w.run(&w.ours, &["rev-parse", "main"]).trim(), main);
}

/// A real interactive rebase, paused at its first commit, started while the
/// engine fetches.
#[test]
fn a_rebase_started_during_the_fetch_lands_nothing() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let main = w.head(&w.ours);
    during_the_fetch(
        &w,
        "GIT_SEQUENCE_EDITOR='sed -i.bak 1s/^pick/edit/' GIT_EDITOR=: git rebase -q -i HEAD~1 >/dev/null 2>&1",
    );
    let r = w.pull();
    assert!(
        w.ours.join(".git").join("rebase-merge").exists(),
        "no rebase started"
    );
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert!(detail(&r).contains("rebase"), "{}", detail(&r));
    assert_eq!(r.pulled, 0);
    assert_eq!(w.run(&w.ours, &["rev-parse", "main"]).trim(), main);
}

/// The branch moves after the merge was made, before it lands (a hook
/// commits on it): the merge is for a commit the branch does not hold now.
#[test]
fn a_branch_that_moved_before_the_land_lands_nothing() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.hook(
        &w.ours,
        "pre-merge-commit",
        "unset GIT_INDEX_FILE\ngit commit -q --allow-empty -m 'made meanwhile'",
    );
    let r = w.pull();
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert!(
        detail(&r).contains("refs/heads/main moved"),
        "{}",
        detail(&r)
    );
    assert_eq!(r.pulled, 0);
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%s"]).trim(),
        "made meanwhile"
    );
}

/// HEAD moves to another branch once the brain is open: the push sends the
/// branch the engine pinned, never what HEAD now names.
#[test]
fn the_push_sends_the_pinned_branch_not_head() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let main = w.head(&w.ours);
    let ctx = w.open();
    w.run(&w.ours, &["switch", "-q", "-c", "private"]);
    w.write(
        &w.ours,
        "private-secret.md",
        &neuron("", "# Not for origin\n"),
    );
    w.run(&w.ours, &["add", "-A"]);
    w.run(&w.ours, &["commit", "-q", "-m", "private"]);
    assert_eq!(engine::push(&ctx), Pushed::Pushed);
    assert_eq!(w.remote_head(), main);
}

/// HEAD moves to another branch at the last moment, after the engine's
/// last check and before git's own write (a git wrapper switches as the
/// fast-forward starts): git moves that branch, and the engine reports it.
#[cfg(unix)]
#[test]
fn a_switch_just_before_gits_write_is_reported() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    side_branch(&w);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let incoming = w.remote_head();
    let main = w.head(&w.ours);
    let script = w.dir.path().join("switching-git");
    fs::write(
        &script,
        "#!/bin/sh\ncase \" $* \" in\n *' merge --ff-only '*) git switch -q side;;\nesac\nexec git \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let ctx =
        kluris_core::sync::engine::open(&w.ours, &w.git().program(&script), &w.request()).unwrap();
    let r = engine::pull(&ctx);
    assert_eq!(
        w.run(&w.ours, &["rev-parse", "side"]).trim(),
        incoming,
        "the switch did not land"
    );
    assert!(left(&r, LeftReason::Busy), "{r:?}");
    assert!(
        detail(&r).contains("refs/heads/side") && detail(&r).contains("refs/heads/main"),
        "{}",
        detail(&r)
    );
    assert_eq!(r.pulled, 0);
    assert_eq!(w.run(&w.ours, &["rev-parse", "main"]).trim(), main);
}
