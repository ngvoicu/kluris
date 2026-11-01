//! A resolution is one transaction under the worktree's tree claim: it
//! reads the waiting merge, records a choice, finishes the merge and
//! removes the record without another resolver, an abort or a pull
//! changing the record in between.

use std::fs;
use std::time::Duration;

use kluris_core::sync::claim::TreeClaim;
#[cfg(unix)]
use kluris_core::sync::engine;
use kluris_core::sync::resolve::{Choice, ResolveError, ResolveRequest, abort, resolve};
use kluris_core::sync::{LeftReason, PENDING, SyncRequest};

#[cfg(unix)]
use crate::chosen;
use crate::common::*;
use crate::two_conflicts;

fn pending(w: &World) -> Vec<u8> {
    fs::read(w.ours.join(".git").join(PENDING)).unwrap()
}

/// A request whose wait for the tree claim is short.
fn hurried(w: &World) -> SyncRequest<'static> {
    SyncRequest {
        tree_bound: Duration::from_millis(200),
        ..w.request()
    }
}

fn request<'a>(
    path: &'a str,
    merge: &'a str,
    choice: Choice,
    sync: SyncRequest<'a>,
) -> ResolveRequest<'a> {
    ResolveRequest {
        path,
        merge,
        choice,
        sync,
    }
}

/// Left as it is because the work tree is busy.
fn busy(result: Result<impl std::fmt::Debug, ResolveError>) -> bool {
    matches!(result, Err(ResolveError::Left(left)) if left.reason == LeftReason::Busy)
}

#[test]
fn a_resolution_while_another_holds_the_tree_claim_records_nothing() {
    let w = World::new();
    two_conflicts(&w);
    let merge = w.merge_id();
    let recorded = pending(&w);
    let held = TreeClaim::exclusive(&w.ours.join(".git"), Duration::ZERO).unwrap();
    let r = resolve(
        &w.ours,
        &w.git(),
        &request("notes/first.md", &merge, Choice::Ours, hurried(&w)),
    );
    assert!(busy(r.clone()), "{r:?}");
    assert_eq!(pending(&w), recorded);
    drop(held);
    let r = resolve(
        &w.ours,
        &w.git(),
        &request("notes/first.md", &merge, Choice::Ours, hurried(&w)),
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn an_abort_while_another_holds_the_tree_claim_drops_nothing() {
    let w = World::new();
    two_conflicts(&w);
    let held = TreeClaim::exclusive(&w.ours.join(".git"), Duration::ZERO).unwrap();
    let r = abort(&w.ours, &w.git(), Duration::from_millis(200));
    assert!(busy(r.clone()), "{r:?}");
    assert!(w.ours.join(".git").join(PENDING).exists());
    drop(held);
    assert_eq!(abort(&w.ours, &w.git(), Duration::ZERO).unwrap(), 2);
}

/// A git that pauses the first time it reads `refs/heads/main` (a
/// resolver's check of the branch's commit, inside its transaction) until
/// the test lets it go: the paths of the marker it leaves and of the one it
/// waits for.
#[cfg(unix)]
fn pausing_git(
    w: &World,
) -> (
    kluris_core::git::SystemGit,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    use std::os::unix::fs::PermissionsExt;
    let (paused, go) = (w.dir.path().join("paused"), w.dir.path().join("go"));
    let script = w.dir.path().join("pausing-git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \" $* \" in\n *' rev-parse --verify --quiet refs/heads/main '*)\n  if [ ! -e '{p}' ]; then\n   : > '{p}'\n   n=0\n   while [ ! -e '{g}' ] && [ $n -lt 3000 ]; do sleep 0.01; n=$((n+1)); done\n  fi;;\nesac\nexec git \"$@\"\n",
            p = paused.display(),
            g = go.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (w.git().program(&script), paused, go)
}

/// Wait, with a bound, for `marker`.
#[cfg(unix)]
fn reached(marker: &std::path::Path) {
    for _ in 0..3000 {
        if marker.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{} never appeared", marker.display());
}

/// Two resolvers at once: while one is inside its transaction, the other
/// either waits and is told it is busy, or its choice survives. A choice
/// reported as recorded is never lost.
#[cfg(unix)]
#[test]
fn a_second_resolver_never_loses_a_recorded_choice() {
    let w = World::new();
    two_conflicts(&w);
    let merge = w.merge_id();
    let (pausing, paused, go) = pausing_git(&w);
    let (first, second) = std::thread::scope(|s| {
        let first = s.spawn(|| {
            resolve(
                &w.ours,
                &pausing,
                &request("notes/first.md", &merge, Choice::Ours, w.request()),
            )
        });
        reached(&paused);
        let second = resolve(
            &w.ours,
            &w.git(),
            &request("ideas/idea.md", &merge, Choice::Theirs, hurried(&w)),
        );
        fs::write(&go, "").unwrap();
        (first.join().unwrap(), second)
    });
    let first = first.unwrap();
    let chosen = chosen(&w);
    assert!(chosen.contains(&"notes/first.md".to_string()), "{chosen:?}");
    match second {
        Ok(r) => assert!(
            chosen.contains(&"ideas/idea.md".to_string()),
            "lost {r:?}: {chosen:?}"
        ),
        Err(e) => assert!(busy(Err::<(), _>(e.clone())), "{e:?}"),
    }
    assert_eq!(first.remaining, ["ideas/idea.md"]);
}

/// A pull while a resolver is inside its transaction waits for the tree
/// claim and changes nothing.
#[cfg(unix)]
#[test]
fn a_pull_while_a_resolver_records_is_busy() {
    let w = World::new();
    two_conflicts(&w);
    let merge = w.merge_id();
    let (pausing, paused, go) = pausing_git(&w);
    let pulled = std::thread::scope(|s| {
        let resolver = s.spawn(|| {
            resolve(
                &w.ours,
                &pausing,
                &request("notes/first.md", &merge, Choice::Ours, w.request()),
            )
        });
        reached(&paused);
        let ctx = engine::open(&w.ours, &w.git(), &hurried(&w)).unwrap();
        let pulled = engine::pull(&ctx);
        fs::write(&go, "").unwrap();
        resolver.join().unwrap().unwrap();
        pulled
    });
    assert!(left(&pulled, LeftReason::Busy), "{pulled:?}");
    assert_eq!(chosen(&w), ["notes/first.md"]);
}
