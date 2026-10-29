//! The waiting merge (`pending.json`) on each pull: replaced when HEAD
//! moved, dropped when the upstream now merges cleanly, otherwise reported
//! with the newer commits that wait behind it. It blocks nothing.

use std::fs;

use kluris_core::sync::PENDING;
use kluris_core::sync::resolve::{Choice, ResolveRequest, resolve};

use crate::common::*;
use crate::overlapping;

fn pending_doc(w: &World) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(w.ours.join(".git").join(PENDING)).unwrap()).unwrap()
}

#[test]
fn a_waiting_merge_whose_ours_is_not_head_is_replaced() {
    let w = World::new();
    overlapping(&w);
    let first = w.sync();
    assert_eq!(first.conflicts.len(), 1, "{first:?}");
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let head = w.head(&w.ours);
    let before = w.snapshot();
    let r = w.pull();
    assert!(r.left_as_is.is_none(), "{r:?}");
    assert_eq!(r.conflicts, first.conflicts);
    assert_eq!(r.behind_conflict, 0);
    assert_eq!(pending_doc(&w)["ours"], head.as_str());
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_unreadable_waiting_merge_is_replaced() {
    let w = World::new();
    overlapping(&w);
    let path = w.ours.join(".git").join(PENDING);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not a waiting merge\n").unwrap();
    let r = w.pull();
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    assert_eq!(pending_doc(&w)["ours"], w.head(&w.ours).as_str());
}

#[test]
fn a_waiting_merge_the_upstream_now_merges_cleanly_with_is_dropped_and_pulled() {
    let w = World::new();
    let (base, ours, _) = overlapping(&w);
    assert_eq!(w.sync().conflicts.len(), 1);
    // The teammate takes their edit back: the upstream merges cleanly now.
    w.theirs_push(&[("notes/first.md", &base)], &[]);
    let r = w.sync();
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.pushed), (2, true), "{r:?}");
    assert!(!w.ours.join(".git").join(PENDING).exists());
    assert_eq!(w.read(&w.ours, "notes/first.md"), ours);
    assert_eq!(w.remote_head(), w.head(&w.ours));
}

#[test]
fn newer_commits_wait_behind_the_recorded_conflict() {
    let w = World::new();
    let (_, ours, theirs) = overlapping(&w);
    let first = w.sync();
    let pinned = w.remote_head();
    let recorded = fs::read(w.ours.join(".git").join(PENDING)).unwrap();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.theirs_push(&[("ideas/other.md", &neuron("", "# Other\n"))], &[]);
    let before = w.snapshot();
    let r = w.sync();
    assert!(r.left_as_is.is_none(), "{r:?}");
    assert_eq!(r.conflicts, first.conflicts);
    assert_eq!(r.conflicts[0].theirs.as_deref(), Some(theirs.as_str()));
    assert_eq!(
        (r.behind_conflict, r.pulled, r.pushed),
        (2, 0, false),
        "{r:?}"
    );
    assert_eq!(
        fs::read(w.ours.join(".git").join(PENDING)).unwrap(),
        recorded
    );
    assert_eq!(w.snapshot(), before);

    // The merge finishes against the commit the conflict was shown with;
    // the newer commits come in after it.
    let done = resolve(
        &w.ours,
        &w.git(),
        &ResolveRequest {
            path: "notes/first.md",
            choice: Choice::Ours,
            sync: w.request(),
        },
    )
    .unwrap();
    assert!(done.committed && done.pushed, "{done:?}");
    let merge = done.commit.unwrap();
    assert_eq!(
        w.run(&w.ours, &["rev-parse", &format!("{merge}^2")]).trim(),
        pinned
    );
    assert_eq!(w.read(&w.ours, "notes/first.md"), ours);
    assert!(w.read(&w.ours, "ideas/other.md").contains("# Other"));
    assert_eq!(w.remote_head(), w.head(&w.ours));
}
