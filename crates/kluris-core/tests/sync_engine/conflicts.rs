//! What goes to a person: a conflict writes the waiting merge
//! (`pending.json`) and nothing else, and is reported with each side's text.

use std::fs;

use kluris_core::sync::{ConflictKind, PENDING, SyncReport};

use crate::common::*;
use crate::{criss_cross, overlapping};

/// The sync ends in exactly one conflict of `kind` at `path`, and nothing
/// changed but the waiting merge: HEAD, the index, the status, the files and
/// the remote are as they were. The next sync reports the same conflict and
/// changes nothing either.
fn assert_conflict(w: &World, path: &str, kind: ConflictKind) -> SyncReport {
    let before = w.snapshot();
    let ours = w.head(&w.ours);
    let r = w.sync();
    assert!(r.left_as_is.is_none(), "{r:?}");
    assert_eq!(
        (r.pulled, r.pushed, r.behind_conflict),
        (0, false, 0),
        "{r:?}"
    );
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    let c = &r.conflicts[0];
    assert_eq!((c.path.as_str(), c.kind), (path, kind), "{r:?}");
    assert_eq!(w.snapshot(), before, "a conflict changed something");
    no_markers_anywhere(w, &w.ours);

    let pending = w.ours.join(".git").join(PENDING);
    let recorded = fs::read_to_string(&pending).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&recorded).unwrap();
    let keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["base", "ours", "theirs", "conflicts", "created"]);
    assert_eq!(doc["ours"], ours.as_str());
    assert_eq!(doc["theirs"], w.remote_head().as_str());
    assert_eq!(doc["conflicts"][0]["path"], path);
    assert_eq!(doc["conflicts"][0]["kind"], kind.as_str());
    let entry_keys: Vec<&str> = doc["conflicts"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(entry_keys, ["path", "kind", "base", "ours", "theirs"]);

    let again = w.sync();
    assert_eq!(again, r);
    assert_eq!(fs::read_to_string(&pending).unwrap(), recorded);
    assert_eq!(w.snapshot(), before);
    r
}

#[test]
fn overlapping_body_edits_are_a_conflict_with_each_sides_text() {
    let w = World::new();
    let (base, ours, theirs) = overlapping(&w);
    let r = assert_conflict(&w, "notes/first.md", ConflictKind::Content);
    let c = &r.conflicts[0];
    assert_eq!(c.base.as_deref(), Some(base.as_str()));
    assert_eq!(c.ours.as_deref(), Some(ours.as_str()));
    assert_eq!(c.theirs.as_deref(), Some(theirs.as_str()));
    assert!(!c.binary);
}

#[test]
fn delete_vs_edit_is_a_conflict_with_no_text_on_the_deleted_side() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    let ours = base.replace("Three.", "Three, ours.");
    w.theirs_push(&[], &["notes/first.md"]);
    w.ours_commit(&[("notes/first.md", &ours)], &[]);
    let r = assert_conflict(&w, "notes/first.md", ConflictKind::DeleteEdit);
    let c = &r.conflicts[0];
    assert_eq!(c.theirs, None);
    assert_eq!(c.ours.as_deref(), Some(ours.as_str()));
    assert_eq!(c.base.as_deref(), Some(base.as_str()));
    assert!(!c.binary);
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(w.ours.join(".git").join(PENDING)).unwrap())
            .unwrap();
    assert!(doc["conflicts"][0]["theirs"].is_null(), "{doc}");
    assert!(doc["conflicts"][0]["base"].is_string() && doc["conflicts"][0]["ours"].is_string());
}

#[test]
fn a_binary_side_is_flagged_and_has_no_text() {
    let w = World::new();
    fs::write(w.ours.join("notes").join("img.bin"), b"\x00\x01BASE\n").unwrap();
    w.commit(&w.ours, &["notes/img.bin"]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    fs::write(w.theirs.join("notes").join("img.bin"), b"\x00\x01THEIRS\n").unwrap();
    w.commit(&w.theirs, &["notes/img.bin"]);
    w.run(&w.theirs, &["push", "-q"]);
    fs::write(w.ours.join("notes").join("img.bin"), b"\x00\x01OURS\n").unwrap();
    w.commit(&w.ours, &["notes/img.bin"]);
    let r = assert_conflict(&w, "notes/img.bin", ConflictKind::Content);
    let c = &r.conflicts[0];
    assert!(c.binary, "{c:?}");
    assert_eq!((&c.base, &c.ours, &c.theirs), (&None, &None, &None));
}

#[test]
fn a_side_that_is_not_utf8_is_flagged_binary() {
    let w = World::new();
    w.write(&w.ours, "notes/latin.txt", "caf\u{e9}\n");
    w.commit(&w.ours, &["notes/latin.txt"]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.write(&w.theirs, "notes/latin.txt", "caf\u{e9}, theirs\n");
    w.commit(&w.theirs, &["notes/latin.txt"]);
    w.run(&w.theirs, &["push", "-q"]);
    fs::write(w.ours.join("notes").join("latin.txt"), b"caf\xe9, ours\n").unwrap();
    w.commit(&w.ours, &["notes/latin.txt"]);
    let r = assert_conflict(&w, "notes/latin.txt", ConflictKind::Content);
    let c = &r.conflicts[0];
    assert!(c.binary, "{c:?}");
    assert_eq!(c.ours, None);
    assert_eq!(c.theirs.as_deref(), Some("caf\u{e9}, theirs\n"));
    assert_eq!(c.base.as_deref(), Some("caf\u{e9}\n"));
}

#[test]
fn different_parents_are_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/keep.md");
    w.theirs_push(
        &[(
            "notes/keep.md",
            &base.replace("parent: ./map.md", "parent: ../ideas/map.md"),
        )],
        &[],
    );
    w.ours_commit(
        &[(
            "notes/keep.md",
            &base.replace("parent: ./map.md", "parent: ../brain.md"),
        )],
        &[],
    );
    assert_conflict(&w, "notes/keep.md", ConflictKind::Parent);
}

#[test]
fn a_non_kluris_key_changed_differently_is_a_conflict() {
    let w = World::new();
    w.ours_commit(
        &[("notes/keep.md", &neuron("owner: team\n", "# Keep\n"))],
        &[],
    );
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/keep.md", &neuron("owner: bob\n", "# Keep\n"))],
        &[],
    );
    w.ours_commit(
        &[("notes/keep.md", &neuron("owner: ana\n", "# Keep\n"))],
        &[],
    );
    assert_conflict(&w, "notes/keep.md", ConflictKind::Frontmatter);
}

/// With several merge bases nothing is decided field by field: a neuron
/// changed on both sides goes to a person (`ambiguous_base`), even when the
/// edits do not overlap.
#[test]
fn a_criss_cross_history_sends_a_neuron_changed_on_both_sides_to_a_person() {
    let w = World::new();
    criss_cross(&w);
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("Two.", "Two, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Four.", "Four, ours."))],
        &[],
    );
    assert_conflict(&w, "notes/first.md", ConflictKind::AmbiguousBase);
}

/// Ours reverts one side's approved change, theirs edits that same text:
/// the first base equals ours' revert, so trusting it would drop the revert.
#[test]
fn a_criss_cross_revert_is_never_dropped() {
    let w = World::new();
    let (_, _, first) = criss_cross(&w);
    let old = w.show(&w.ours, &first, "notes/first.md");
    let token = if old.contains("Q_APPROVED") {
        "P_APPROVED"
    } else {
        "Q_APPROVED"
    };
    w.ours_commit(&[("notes/first.md", &old)], &[]);
    let theirs = w
        .read(&w.theirs, "notes/first.md")
        .replace(token, &format!("THEIRS_REPLACED_{token}"));
    w.theirs_push(&[("notes/first.md", &theirs)], &[]);
    assert_conflict(&w, "notes/first.md", ConflictKind::AmbiguousBase);
}

#[test]
fn a_file_where_the_teammate_made_a_directory_goes_to_a_person() {
    let w = World::new();
    w.theirs_push(
        &[("notes/topic.md/child.md", &neuron("", "# Child\n"))],
        &[],
    );
    let ours = neuron("", "# Topic\n");
    w.ours_commit(&[("notes/topic.md", &ours)], &[]);
    let r = assert_conflict(&w, "notes/topic.md", ConflictKind::Structure);
    assert_eq!(r.conflicts[0].ours.as_deref(), Some(ours.as_str()));
    assert_eq!(r.conflicts[0].theirs, None);
}

#[test]
fn a_directory_where_the_teammate_made_a_file_goes_to_a_person() {
    let w = World::new();
    let theirs = neuron("", "# Topic, theirs\n");
    w.theirs_push(&[("notes/topic.md", &theirs)], &[]);
    w.ours_commit(
        &[("notes/topic.md/child.md", &neuron("", "# Child\n"))],
        &[],
    );
    let r = assert_conflict(&w, "notes/topic.md", ConflictKind::Structure);
    assert_eq!(r.conflicts[0].theirs.as_deref(), Some(theirs.as_str()));
}

#[test]
fn a_rename_on_one_side_and_a_delete_on_the_other_goes_to_a_person() {
    let w = World::new();
    w.run(&w.ours, &["mv", "notes/keep.md", "notes/renamed.md"]);
    w.commit(&w.ours, &["notes/keep.md", "notes/renamed.md"]);
    w.theirs_push(&[], &["notes/keep.md"]);
    assert_conflict(&w, "notes/renamed.md", ConflictKind::Structure);
}
