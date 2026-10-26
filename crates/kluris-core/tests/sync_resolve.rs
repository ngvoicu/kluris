//! `kluris resolve` (TEST-K3-47): an open sync conflict settled path by path
//! with real git (a bare remote, our clone, a teammate's), then committed,
//! moved to and pushed as `sync` does.

use std::fs;

use kluris_core::sync::resolve::{
    Choice, ResolveError, ResolveReport, ResolveRequest, abort, open_conflicts, resolve,
};
use kluris_core::sync::{ConflictKind, PENDING, SyncError, SyncRequest};

mod common;

use common::*;

const RESOLVED_MESSAGE: &str =
    "kluris: sync (resolved)\n\nKluris-Sync: resolved\nKluris-Resolved-By: Ana Tester\n\n";

fn resolve_with(w: &World, path: &str, choice: Choice) -> Result<ResolveReport, ResolveError> {
    resolve_req(w, path, choice, false)
}

fn resolve_req(
    w: &World,
    path: &str,
    choice: Choice,
    no_verify: bool,
) -> Result<ResolveReport, ResolveError> {
    resolve(
        &w.ours,
        &w.git(),
        &ResolveRequest {
            path,
            choice,
            sync: SyncRequest {
                no_verify,
                ..w.request()
            },
        },
    )
}

fn pending(w: &World) -> std::path::PathBuf {
    w.ours.join(".git").join(PENDING)
}

/// Sync ends in conflicts at `paths`.
fn open(w: &World, paths: &[&str]) {
    let r = w.sync().unwrap();
    let got: Vec<&str> = r.conflicts.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(got, paths, "{r:?}");
}

/// The overlapping edit of `notes/first.md`; returns (base, ours, theirs).
fn content_conflict(w: &World) -> (String, String, String) {
    let base = w.read(&w.ours, "notes/first.md");
    let theirs = base.replace("Three.", "Three, theirs.");
    let ours = base.replace("Three.", "Three, ours.");
    w.theirs_push(&[("notes/first.md", &theirs)], &[]);
    w.ours_commit(&[("notes/first.md", &ours)], &[]);
    open(w, &["notes/first.md"]);
    (base, ours, theirs)
}

/// The commit landed: a two-parent merge with the resolved message, pushed,
/// the conflict gone, and the next sync clean.
fn assert_completed(w: &World, r: &ResolveReport) {
    assert!(r.committed && r.pushed && r.remaining.is_empty(), "{r:?}");
    let head = w.head(&w.ours);
    assert_eq!(r.commit.as_deref(), Some(head.as_str()));
    assert_eq!(w.parents(&w.ours, &head), 2);
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        RESOLVED_MESSAGE
    );
    assert_eq!(w.remote_head(), head);
    assert!(!pending(w).exists());
    assert!(open_conflicts(&w.ours, &w.git()).is_none());
    assert_eq!(w.status(&w.ours), "");
    let next = w.sync().unwrap();
    assert!(
        next.fetched && !next.merged && !next.pushed && next.conflicts.is_empty(),
        "{next:?}"
    );
    for (rel, bytes) in w.files(&w.ours) {
        if let Ok(text) = String::from_utf8(bytes) {
            assert!(!text.contains("<<<<<<<"), "marker in {rel}");
        }
    }
}

// --- each kind, each choice ---------------------------------------------------------

#[test]
fn content_resolved_ours() {
    let w = World::new();
    let (_, ours, _) = content_conflict(&w);
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert_eq!(r.choice, "ours");
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/first.md"), ours);
}

#[test]
fn content_resolved_theirs() {
    let w = World::new();
    let (_, _, theirs) = content_conflict(&w);
    let r = resolve_with(&w, "notes/first.md", Choice::Theirs).unwrap();
    assert_eq!(r.choice, "theirs");
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/first.md"), theirs);
}

#[test]
fn content_resolved_with_a_file() {
    let w = World::new();
    let (base, _, _) = content_conflict(&w);
    let merged = base.replace("Three.", "Three, ours and theirs.");
    let r = resolve_with(
        &w,
        "notes/first.md",
        Choice::File(merged.clone().into_bytes()),
    )
    .unwrap();
    assert_eq!(r.choice, "file");
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/first.md"), merged);
    assert_eq!(w.show(&w.ours, "HEAD", "notes/first.md"), merged);
}

fn delete_edit(w: &World) -> String {
    let base = w.read(&w.ours, "notes/first.md");
    let ours = base.replace("Three.", "Three, ours.");
    w.theirs_push(&[], &["notes/first.md"]);
    w.ours_commit(&[("notes/first.md", &ours)], &[]);
    open(w, &["notes/first.md"]);
    ours
}

#[test]
fn delete_edit_resolved_ours_keeps_the_edit() {
    let w = World::new();
    let ours = delete_edit(&w);
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/first.md"), ours);
    assert!(w.read(&w.ours, "notes/map.md").contains("first.md"));
}

#[test]
fn delete_edit_resolved_theirs_deletes_and_rebuilds_the_map() {
    let w = World::new();
    delete_edit(&w);
    let r = resolve_with(&w, "notes/first.md", Choice::Theirs).unwrap();
    assert_completed(&w, &r);
    assert!(!w.ours.join("notes").join("first.md").exists());
    assert!(!w.read(&w.ours, "notes/map.md").contains("first.md"));
}

fn parent_conflict(w: &World) -> (String, String) {
    let base = w.read(&w.ours, "notes/keep.md");
    let theirs = base.replace("parent: ./map.md", "parent: ../ideas/map.md");
    let ours = base.replace("parent: ./map.md", "parent: ../brain.md");
    w.theirs_push(&[("notes/keep.md", &theirs)], &[]);
    w.ours_commit(&[("notes/keep.md", &ours)], &[]);
    let r = w.sync().unwrap();
    assert_eq!(r.conflicts[0].kind, ConflictKind::Parent);
    (ours, theirs)
}

#[test]
fn parent_resolved_ours_theirs_and_file() {
    for which in ["ours", "theirs", "file"] {
        let w = World::new();
        let (ours, theirs) = parent_conflict(&w);
        let chosen = ours.replace("parent: ../brain.md", "parent: ./map.md");
        let (choice, expected) = match which {
            "ours" => (Choice::Ours, ours),
            "theirs" => (Choice::Theirs, theirs),
            _ => (Choice::File(chosen.clone().into_bytes()), chosen),
        };
        let r = resolve_with(&w, "notes/keep.md", choice).unwrap();
        assert_completed(&w, &r);
        assert_eq!(w.read(&w.ours, "notes/keep.md"), expected, "{which}");
    }
}

#[test]
fn frontmatter_resolved_with_a_file() {
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
    open(&w, &["notes/keep.md"]);
    let both = neuron("owner: ana and bob\n", "# Keep\n");
    let r = resolve_with(&w, "notes/keep.md", Choice::File(both.clone().into_bytes())).unwrap();
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/keep.md"), both);
}

// --- one by one, validation, errors -------------------------------------------------

#[test]
fn several_paths_are_resolved_one_by_one() {
    let w = World::new();
    let first = w.read(&w.ours, "notes/first.md");
    let idea = neuron("", "# Idea\n\nA.\n");
    w.ours_commit(&[("ideas/idea.md", &idea)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[
            ("notes/first.md", &first.replace("Three.", "Three, theirs.")),
            ("ideas/idea.md", &idea.replace("A.", "A, theirs.")),
        ],
        &[],
    );
    w.ours_commit(
        &[
            ("notes/first.md", &first.replace("Three.", "Three, ours.")),
            ("ideas/idea.md", &idea.replace("A.", "A, ours.")),
        ],
        &[],
    );
    open(&w, &["ideas/idea.md", "notes/first.md"]);
    let before = w.snapshot();
    let r = resolve_with(&w, "notes/first.md", Choice::Theirs).unwrap();
    assert!(!r.committed && !r.pushed && r.commit.is_none(), "{r:?}");
    assert_eq!(r.remaining, ["ideas/idea.md"]);
    assert_eq!(w.snapshot(), before, "a partial resolve changed the brain");
    assert!(pending(&w).exists());
    // Choosing again before the end replaces the choice.
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert_eq!(r.remaining, ["ideas/idea.md"]);
    let r = resolve_with(&w, "ideas/idea.md", Choice::Theirs).unwrap();
    assert_completed(&w, &r);
    assert!(w.read(&w.ours, "notes/first.md").contains("Three, ours."));
    assert!(w.read(&w.ours, "ideas/idea.md").contains("A, theirs."));
}

#[test]
fn an_invalid_file_records_nothing() {
    let w = World::new();
    content_conflict(&w);
    let recorded = fs::read(pending(&w)).unwrap();
    for bad in [
        "---\ntitle: Foo: bar\n---\n\n# First\n",
        "---\nparent: ./map.md\n---\n\n<<<<<<< ours\nOne.\n=======\nTwo.\n>>>>>>> theirs\n",
    ] {
        match resolve_with(&w, "notes/first.md", Choice::File(bad.as_bytes().to_vec())) {
            Err(e @ ResolveError::InvalidFile { .. }) => {
                assert!(e.to_string().contains("notes/first.md"), "{e}")
            }
            other => panic!("{bad:?}: {other:?}"),
        }
        assert_eq!(fs::read(pending(&w)).unwrap(), recorded);
    }
}

#[test]
fn a_yaml_neuron_file_needs_its_opt_in_block() {
    let w = World::new();
    let spec = "#---\n# title: Spec\n# updated: '2026-01-01'\n#---\nkey: base\nother: 1\n";
    w.ours_commit(&[("api/spec.yml", spec)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("api/spec.yml", &spec.replace("key: base", "key: theirs"))],
        &[],
    );
    w.ours_commit(
        &[("api/spec.yml", &spec.replace("key: base", "key: ours"))],
        &[],
    );
    open(&w, &["api/spec.yml"]);
    let no_block = "key: both\nother: 1\n";
    assert!(matches!(
        resolve_with(
            &w,
            "api/spec.yml",
            Choice::File(no_block.as_bytes().to_vec())
        ),
        Err(ResolveError::InvalidFile { .. })
    ));
    let good = spec.replace("key: base", "key: both");
    let r = resolve_with(&w, "api/spec.yml", Choice::File(good.clone().into_bytes())).unwrap();
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "api/spec.yml"), good);
}

#[test]
fn a_path_that_is_not_conflicted_is_an_error() {
    let w = World::new();
    content_conflict(&w);
    let recorded = fs::read(pending(&w)).unwrap();
    match resolve_with(&w, "notes/keep.md", Choice::Ours) {
        Err(ResolveError::NotConflicted(path)) => assert_eq!(path, "notes/keep.md"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read(pending(&w)).unwrap(), recorded);
}

#[test]
fn no_open_conflict_is_an_error() {
    let w = World::new();
    assert!(matches!(
        resolve_with(&w, "notes/first.md", Choice::Ours),
        Err(ResolveError::NoOpenConflict)
    ));
    assert!(matches!(
        abort(&w.ours, &w.git()),
        Err(ResolveError::NoOpenConflict)
    ));
    assert!(open_conflicts(&w.ours, &w.git()).is_none());
}

#[test]
fn a_stale_conflict_is_refused_and_abort_drops_it() {
    let w = World::new();
    content_conflict(&w);
    let conflicts = open_conflicts(&w.ours, &w.git()).unwrap();
    assert_eq!(conflicts[0].path, "notes/first.md");
    // The user commits meanwhile: HEAD is no longer the conflict's ours.
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.snapshot();
    match resolve_with(&w, "notes/first.md", Choice::Ours) {
        Err(e @ ResolveError::Stale { .. }) => {
            let text = e.to_string();
            assert!(text.contains("kluris resolve --abort"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
    assert_eq!(abort(&w.ours, &w.git()).unwrap(), 1);
    assert!(!pending(&w).exists());
    assert_eq!(
        w.snapshot(),
        before,
        "abort changed more than the pending state"
    );
    // The next sync meets the same conflict again.
    open(&w, &["notes/first.md"]);
}

#[test]
fn abort_changes_nothing_but_the_pending_state() {
    let w = World::new();
    content_conflict(&w);
    let before = w.snapshot();
    assert_eq!(abort(&w.ours, &w.git()).unwrap(), 1);
    assert_eq!(w.snapshot(), before);
    assert!(!pending(&w).exists());
    assert!(open_conflicts(&w.ours, &w.git()).is_none());
}

// --- completing: dirty files, hooks, the push race ----------------------------------

#[test]
fn a_dirty_file_in_the_way_keeps_the_choices_until_it_is_gone() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    let idea = w.read(&w.ours, "ideas/idea.md");
    w.theirs_push(
        &[
            ("notes/first.md", &base.replace("Three.", "Three, theirs.")),
            ("ideas/idea.md", &idea.replace("# Idea", "# Idea, theirs")),
        ],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    open(&w, &["notes/first.md"]);
    w.write(
        &w.ours,
        "ideas/idea.md",
        &idea.replace("# Idea", "# Idea, mine"),
    );
    let head = w.head(&w.ours);
    match resolve_with(&w, "notes/first.md", Choice::Ours) {
        Err(ResolveError::Blocked(paths)) => assert_eq!(paths, ["ideas/idea.md"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.head(&w.ours), head);
    assert!(pending(&w).exists());
    assert!(w.read(&w.ours, "ideas/idea.md").contains("mine"));
    // Put the file back as it was committed, then complete.
    w.write(&w.ours, "ideas/idea.md", &idea);
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert_completed(&w, &r);
    assert!(w.read(&w.ours, "ideas/idea.md").contains("# Idea, theirs"));
}

#[test]
fn a_failing_pre_merge_commit_hook_keeps_the_conflict_open() {
    let w = World::new();
    content_conflict(&w);
    w.hook(&w.ours, "pre-merge-commit", "exit 1");
    let head = w.head(&w.ours);
    match resolve_with(&w, "notes/first.md", Choice::Ours) {
        Err(ResolveError::Sync(SyncError::Hook { name, .. })) => {
            assert_eq!(name, "pre-merge-commit")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(w.head(&w.ours), head);
    assert!(pending(&w).exists());
    let r = resolve_req(&w, "notes/first.md", Choice::Ours, true).unwrap();
    assert_completed(&w, &r);
}

/// Our pre-push hook makes a third clone push `rel` first, once.
fn race(w: &World, rel: &str, text: &str) -> std::path::PathBuf {
    let third = w.dir.path().join("third");
    w.clone_as(&third, "Cy Third", "cy@example.com");
    w.write(&third, rel, text);
    w.run(&third, &["add", "-A"]);
    w.run(&third, &["commit", "-q", "-m", "race"]);
    let marker = w.dir.path().join("raced");
    let body = format!(
        "if [ ! -e '{m}' ]; then touch '{m}'; cd '{t}' && unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE && git push -q >&2; fi\nexit 0",
        m = marker.display().to_string().replace('\\', "/"),
        t = third.display().to_string().replace('\\', "/"),
    );
    w.hook(&w.ours, "pre-push", &body);
    marker
}

#[test]
fn a_push_race_during_resolve_is_merged_and_retried() {
    let w = World::new();
    let (_, ours, _) = content_conflict(&w);
    let marker = race(&w, "ideas/race.md", &neuron("", "# Race\n"));
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert!(marker.exists(), "the race did not happen");
    assert!(r.committed && r.pushed && r.remaining.is_empty(), "{r:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
    assert!(w.read(&w.ours, "ideas/race.md").contains("# Race"));
    assert_eq!(w.read(&w.ours, "notes/first.md"), ours);
    assert!(!pending(&w).exists());
}

#[test]
fn a_race_that_conflicts_again_opens_a_new_conflict() {
    let w = World::new();
    content_conflict(&w);
    let theirs_again = w
        .read(&w.theirs, "notes/first.md")
        .replace("Three, theirs.", "Three, theirs again.");
    let marker = race(&w, "notes/first.md", &theirs_again);
    let r = resolve_with(&w, "notes/first.md", Choice::Ours).unwrap();
    assert!(marker.exists());
    assert!(r.committed && !r.pushed, "{r:?}");
    assert_eq!(r.remaining, ["notes/first.md"]);
    // The resolved merge is in place; the new conflict starts from it.
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(pending(&w)).unwrap()).unwrap();
    assert_eq!(doc["ours"], w.head(&w.ours).as_str());
    assert_eq!(r.commit.as_deref(), Some(w.head(&w.ours).as_str()));
}

// --- names and line endings -----------------------------------------------------------

#[test]
fn a_non_ascii_path_resolves() {
    let w = World::new();
    let path = "notes/café ünï.md";
    let base = neuron("", "# Café\n\nUn.\n");
    w.ours_commit(&[(path, &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(&[(path, &base.replace("Un.", "Un, theirs."))], &[]);
    w.ours_commit(&[(path, &base.replace("Un.", "Un, ours."))], &[]);
    open(&w, &[path]);
    let r = resolve_with(&w, path, Choice::Theirs).unwrap();
    assert_completed(&w, &r);
    assert!(w.read(&w.ours, path).contains("Un, theirs."));
}

#[test]
fn a_crlf_file_is_committed_as_given() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md").replace('\n', "\r\n");
    w.ours_commit(&[("notes/first.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/first.md", &base.replace("Three.", "Three, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    open(&w, &["notes/first.md"]);
    let merged = base.replace("Three.", "Three, both.");
    let r = resolve_with(
        &w,
        "notes/first.md",
        Choice::File(merged.clone().into_bytes()),
    )
    .unwrap();
    assert_completed(&w, &r);
    assert_eq!(w.read(&w.ours, "notes/first.md"), merged);
}

#[test]
fn a_windows_style_path_names_the_conflict() {
    let w = World::new();
    content_conflict(&w);
    let given = if cfg!(windows) {
        "notes\\first.md"
    } else {
        "notes/./first.md"
    };
    let r = resolve_with(&w, given, Choice::Ours).unwrap();
    assert_eq!(r.path, "notes/first.md");
    assert_completed(&w, &r);
}
