//! `kluris sync` (TEST-K3-45), with real git: a bare remote and two clones
//! in temp dirs, "ours" (the brain being synced) and "theirs" (a teammate).

use std::fs;

use kluris_core::sync::merge::merge_markdown;
use kluris_core::sync::{ConflictKind, PENDING, SyncError, SyncRequest, sync};

mod common;

use common::*;

// --- fetch, fast-forward, push, offline ------------------------------------------------

#[test]
fn nothing_to_do() {
    let w = World::new();
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed && !r.offline, "{r:?}");
    assert!(r.auto_merged.is_empty() && r.conflicts.is_empty());
    assert!(r.skipped_dirty.is_empty() && r.blocked.is_empty());
    assert_eq!(w.snapshot(), before);
}

#[test]
fn behind_only_fast_forwards() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.fetched && r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.head(&w.ours), w.remote_head());
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "ideas/map.md").contains("new.md"));
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn ahead_only_pushes() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
}

#[test]
fn no_upstream_is_offline_and_keeps_local_commits() {
    let w = World::new();
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let head = w.head(&w.ours);
    let remote = w.remote_head();
    let r = w.sync().unwrap();
    assert!(r.offline && !r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.head(&w.ours), head);
    assert_eq!(w.remote_head(), remote);
}

#[test]
fn an_unreachable_remote_is_offline_with_nothing_changed() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let gone = w.dir.path().join("gone.git").to_string_lossy().into_owned();
    w.run(&w.ours, &["remote", "set-url", "origin", &gone]);
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert!(r.offline && !r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.snapshot(), before);
}

// --- auto-merged -------------------------------------------------------------------------

#[test]
fn different_files_merge_into_a_sync_commit_and_push() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let ours_before = w.head(&w.ours);
    let r = w.sync().unwrap();
    assert!(r.fetched && r.merged && r.pushed, "{r:?}");
    assert!(
        r.auto_merged.is_empty(),
        "different files are not listed: {r:?}"
    );
    let head = w.head(&w.ours);
    assert_eq!(w.parents(&w.ours, &head), 2);
    assert_eq!(w.run(&w.ours, &["rev-parse", "HEAD^1"]).trim(), ours_before);
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        "kluris: sync\n\nKluris-Sync: auto\n\n"
    );
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Ana Tester <ana@example.com>"
    );
    assert_eq!(w.remote_head(), head);
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "notes/second.md").contains("# Second"));
    assert_eq!(w.status(&w.ours), "");
    no_markers_anywhere(&w, &w.ours);
}

#[test]
fn generated_maps_are_rebuilt_from_the_merged_tree() {
    let w = World::new();
    // Both sides add a neuron to the same lobe: both regenerate notes/map.md.
    w.theirs_push(&[("notes/theirs.md", &neuron("", "# Theirs\n"))], &[]);
    w.ours_commit(&[("notes/ours.md", &neuron("", "# Ours\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    let map = w.show(&w.ours, "HEAD", "notes/map.md");
    no_markers(&map);
    assert!(map.contains("- [ours.md](./ours.md) — Ours"), "{map}");
    assert!(map.contains("- [theirs.md](./theirs.md) — Theirs"), "{map}");
    assert_eq!(w.read(&w.ours, "notes/map.md"), map);
    assert!(
        !r.auto_merged.contains(&"notes/map.md".to_string()),
        "{r:?}"
    );
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn a_new_lobe_on_each_side_rebuilds_brain_md() {
    let w = World::new();
    w.theirs_push(&[("theirs-lobe/t.md", &neuron("", "# T\n"))], &[]);
    w.ours_commit(&[("ours-lobe/o.md", &neuron("", "# O\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.conflicts.is_empty(), "{r:?}");
    let brain = w.show(&w.ours, "HEAD", "brain.md");
    no_markers(&brain);
    assert!(
        brain.contains("[ours-lobe/]") && brain.contains("[theirs-lobe/]"),
        "{brain}"
    );
}

#[test]
fn non_overlapping_body_edits_merge_and_are_listed() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/first.md"]);
    let text = w.read(&w.ours, "notes/first.md");
    no_markers(&text);
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn dates_take_the_later_updated_and_the_earlier_created() {
    let w = World::new();
    let theirs = "---\ncreated: '2025-12-01'\nparent: ./map.md\nupdated: '2026-03-01'\n---\n\n# First\n\nOne, theirs.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";
    let ours = "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-01'\n---\n\n# First\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive, ours.\n";
    w.theirs_push(&[("notes/first.md", theirs)], &[]);
    w.ours_commit(&[("notes/first.md", ours)], &[]);
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty(), "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(text.contains("created: '2025-12-01'\n"), "{text}");
    assert!(text.contains("updated: '2026-03-01'\n"), "{text}");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn related_and_tags_are_the_union_ours_first() {
    let w = World::new();
    let with = |tags: &str, related: &str| {
        format!(
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n{related}tags:\n{tags}updated: '2026-01-01'\n---\n\n# Keep\n"
        )
    };
    let base = with("- api\n", "- ../ideas/idea.md\n");
    w.ours_commit(&[("notes/keep.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[(
            "notes/keep.md",
            &with(
                "- api\n- zeta\n- auth\n",
                "- ../ideas/idea.md\n- first.md\n",
            ),
        )],
        &[],
    );
    w.ours_commit(
        &[(
            "notes/keep.md",
            &with("- api\n- beta\n", "- ../ideas/idea.md\n"),
        )],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/keep.md"]);
    let text = w.read(&w.ours, "notes/keep.md");
    assert!(
        text.contains("tags:\n- api\n- beta\n- zeta\n- auth\n"),
        "{text}"
    );
    assert!(
        text.contains("related:\n- ../ideas/idea.md\n- first.md\n"),
        "{text}"
    );
}

#[test]
fn crlf_neurons_merge_and_keep_their_line_endings() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md").replace('\n', "\r\n");
    w.ours_commit(&[("notes/first.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty() && r.merged, "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text:?}"
    );
    assert_eq!(
        text.matches('\n').count(),
        text.matches("\r\n").count(),
        "{text:?}"
    );
}

#[test]
fn non_ascii_names_sync() {
    let w = World::new();
    w.theirs_push(&[("été/naïve.md", &neuron("", "# Naïve\n"))], &[]);
    w.ours_commit(&[("notes/café ünï.md", &neuron("", "# Café\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(w.read(&w.ours, "été/naïve.md").contains("# Naïve"));
    assert!(w.read(&w.ours, "brain.md").contains("été"));
    assert_eq!(w.status(&w.ours), "");
}

// --- sent to a person --------------------------------------------------------------------

/// Sync ends in exactly one conflict of `kind` at `path`, and nothing
/// changed: HEAD, index, status, files and the remote are as they were.
fn assert_conflict(w: &World, path: &str, kind: ConflictKind) {
    let before = w.snapshot();
    let ours = w.head(&w.ours);
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    let c = &r.conflicts[0];
    assert_eq!((c.path.as_str(), c.kind), (path, kind), "{r:?}");
    assert_eq!(w.snapshot(), before, "a conflict changed something");
    no_markers_anywhere(w, &w.ours);

    let pending = w.ours.join(".git").join(PENDING);
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&pending).unwrap()).unwrap();
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

    // The next sync reports the open conflict in its entry (decision of
    // T-146): the conflicts from pending.json, `refused` pointing at
    // `kluris resolve`, nothing fetched or changed.
    let again = w.sync().unwrap();
    assert_eq!(again.conflicts, r.conflicts);
    assert!(
        !again.fetched && !again.merged && !again.pushed,
        "{again:?}"
    );
    let why = again.refused.as_deref().unwrap_or_default();
    assert_eq!(
        why,
        "a conflict is open: settle it with kluris resolve (or kluris resolve --abort)"
    );
    assert_eq!(w.snapshot(), before);
}

#[test]
fn overlapping_body_edits_are_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("Three.", "Three, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    assert_conflict(&w, "notes/first.md", ConflictKind::Content);
}

#[test]
fn delete_vs_edit_is_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(&[], &["notes/first.md"]);
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    assert_conflict(&w, "notes/first.md", ConflictKind::DeleteEdit);
    let r_conflict = fs::read_to_string(w.ours.join(".git").join(PENDING)).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&r_conflict).unwrap();
    assert!(doc["conflicts"][0]["theirs"].is_null(), "{doc}");
    assert!(doc["conflicts"][0]["base"].is_string() && doc["conflicts"][0]["ours"].is_string());
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

#[test]
fn the_same_change_on_both_sides_is_no_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    let same = base.replace("Three.", "Three, both.");
    w.theirs_push(&[("notes/first.md", &same)], &[]);
    w.ours_commit(&[("notes/first.md", &same)], &[]);
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty() && r.merged, "{r:?}");
    assert!(w.read(&w.ours, "notes/first.md").contains("Three, both."));
}

// --- dirty files ---------------------------------------------------------------------------

#[test]
fn incoming_changes_to_a_dirty_file_stop_the_sync() {
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
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(r.blocked, ["notes/first.md"]);
    assert_eq!(r.skipped_dirty, ["glossary.md", "notes/first.md"]);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_untracked_file_in_the_way_stops_the_sync() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.write(&w.ours, "ideas/new.md", "# my own draft\n");
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["ideas/new.md"]);
    assert!(!r.merged);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_ignored_file_in_the_way_stops_the_sync() {
    let w = World::new();
    w.theirs_push(&[(".gitignore", "kluris.yml\nnotes/local.md\n")], &[]);
    w.run(&w.ours, &["pull", "-q"]);
    // The teammate stops ignoring it and commits theirs; ours is ignored here.
    w.write(&w.ours, "notes/local.md", "# my local note\n");
    let gi = "kluris.yml\n";
    w.write(&w.theirs, ".gitignore", gi);
    w.write(&w.theirs, "notes/local.md", &neuron("", "# Local\n"));
    w.run(&w.theirs, &["add", "-A"]);
    w.run(&w.theirs, &["commit", "-q", "-m", "unignore"]);
    w.run(&w.theirs, &["push", "-q"]);
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["notes/local.md"], "{r:?}");
    assert_eq!(w.snapshot(), before);
}

#[test]
fn dirty_files_the_changes_do_not_touch_stay_and_are_listed() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let mine = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n- mine\n";
    w.write(&w.ours, "glossary.md", mine);
    w.write(&w.ours, "notes/draft.md", "# draft\n");
    let r = w.sync().unwrap();
    assert!(r.merged && r.blocked.is_empty(), "{r:?}");
    assert_eq!(r.skipped_dirty, ["glossary.md", "notes/draft.md"]);
    assert_eq!(w.read(&w.ours, "glossary.md"), mine);
    assert_eq!(w.read(&w.ours, "notes/draft.md"), "# draft\n");
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
}

// --- the push race, hooks ----------------------------------------------------------------

#[test]
fn a_push_rejected_because_the_remote_moved_is_merged_again_and_retried() {
    let w = World::new();
    let third = w.dir.path().join("third");
    w.clone_as(&third, "Cy Third", "cy@example.com");
    w.write(&third, "ideas/race.md", &neuron("", "# Race\n"));
    w.commit(&third, &["ideas/race.md"]);
    // Our pre-push hook makes the third clone push first, once.
    let marker = w.dir.path().join("raced");
    let body = format!(
        "if [ ! -e '{m}' ]; then touch '{m}'; cd '{t}' && unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE && git push -q >&2; fi\nexit 0",
        m = marker.display().to_string().replace('\\', "/"),
        t = third.display().to_string().replace('\\', "/"),
    );
    w.hook(&w.ours, "pre-push", &body);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(marker.exists(), "the race did not happen");
    assert!(r.pushed && r.merged, "{r:?}");
    let remote = w.remote_head();
    assert_eq!(remote, w.head(&w.ours));
    assert!(w.read(&w.ours, "ideas/race.md").contains("# Race"));
    assert!(
        w.show(&w.ours, &remote, "notes/second.md")
            .contains("# Second")
    );
}

#[test]
fn a_failing_pre_merge_commit_hook_stops_the_merge() {
    let w = World::new();
    w.hook(&w.ours, "pre-merge-commit", "echo 'not now' >&2\nexit 1");
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.snapshot();
    match w.sync() {
        Err(SyncError::Hook { name, code }) => assert_eq!((name, code), ("pre-merge-commit", 1)),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
    // --no-verify skips it; post-merge runs once the work tree is updated.
    let ran = w.dir.path().join("post-merge-ran");
    w.hook(
        &w.ours,
        "post-merge",
        &format!("touch '{}'", ran.display().to_string().replace('\\', "/")),
    );
    let r = sync(
        &w.ours,
        &w.git(),
        &SyncRequest {
            no_verify: true,
            ..w.request()
        },
    )
    .unwrap();
    assert!(r.merged && r.pushed, "{r:?}");
    assert!(ran.exists(), "post-merge did not run");
}

// --- refusals ------------------------------------------------------------------------------

#[test]
fn a_brain_nested_in_a_larger_repo_is_refused() {
    let w = World::new();
    fs::create_dir_all(w.ours.join("sub")).unwrap();
    match sync(&w.ours.join("sub"), &w.git(), &w.request()) {
        Err(SyncError::Nested { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_brain_without_git_is_refused() {
    let w = World::new();
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        sync(&plain, &w.git(), &w.request()),
        Err(SyncError::NotARepo)
    ));
}

#[test]
fn a_merge_in_progress_is_refused() {
    let w = World::new();
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
    assert!(matches!(w.sync(), Err(SyncError::InProgress(_))));
    assert_eq!(w.snapshot(), before);
}

#[cfg(unix)]
#[test]
fn an_older_git_is_a_clean_error() {
    use kluris_core::git::SystemGit;
    use kluris_core::sync::check_git;
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

// --- criss-cross ---------------------------------------------------------------------------

/// Two clones that each merged the other: `git merge-base --all` lists two
/// bases. `base` is the first neuron text both sides then change.
fn criss_cross(w: &World) -> (String, String, String) {
    w.ours_commit(
        &[(
            "notes/first.md",
            &w.read(&w.ours, "notes/first.md")
                .replace("One.", "P_APPROVED"),
        )],
        &[],
    );
    let p = w.head(&w.ours);
    let q_text = w
        .read(&w.theirs, "notes/first.md")
        .replace("Five.", "Q_APPROVED");
    w.theirs_push(&[("notes/first.md", &q_text)], &[]);
    let q = w.remote_head();
    w.run(&w.ours, &["fetch", "-q"]);
    let ours_path = w.ours.to_string_lossy().into_owned();
    w.run(&w.theirs, &["fetch", "-q", &ours_path, "main:refs/ours"]);
    w.run(&w.ours, &["merge", "-q", "--no-edit", &q]);
    w.run(&w.theirs, &["merge", "-q", "--no-edit", "refs/ours"]);
    w.run(&w.theirs, &["push", "-q"]);
    w.run(&w.ours, &["fetch", "-q"]);
    let bases = w.run(&w.ours, &["merge-base", "--all", "HEAD", "origin/main"]);
    assert_eq!(bases.lines().count(), 2, "{bases}");
    let first = bases.lines().next().unwrap_or_default().to_string();
    (p, q, first)
}

/// With several merge bases nothing is decided field by field: a neuron
/// changed on both sides goes to a person (`ambiguous-base`), even when the
/// edits do not overlap (decision of T-146, replacing the first-base rule).
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

/// The review's case: ours reverts one side's approved change, theirs edits
/// that same text. The first base equals ours' revert; trusting it dropped
/// the revert.
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

/// A file changed on one side only still merges as git's merge-tree says.
#[test]
fn a_criss_cross_history_merges_one_sided_changes() {
    let w = World::new();
    criss_cross(&w);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "notes/second.md").contains("# Second"));
}

// --- structural conflicts --------------------------------------------------------------

#[test]
fn a_file_where_the_teammate_made_a_directory_goes_to_a_person() {
    let w = World::new();
    w.theirs_push(
        &[("notes/topic.md/child.md", &neuron("", "# Child\n"))],
        &[],
    );
    w.ours_commit(&[("notes/topic.md", &neuron("", "# Topic\n"))], &[]);
    let oid = w
        .run(&w.ours, &["rev-parse", "HEAD:notes/topic.md"])
        .trim()
        .to_string();
    assert_conflict(&w, "notes/topic.md", ConflictKind::Structure);
    let r = w.sync().unwrap();
    assert_eq!(r.conflicts[0].ours.as_deref(), Some(oid.as_str()));
    assert_eq!(r.conflicts[0].theirs, None);
}

#[test]
fn a_directory_where_the_teammate_made_a_file_goes_to_a_person() {
    let w = World::new();
    w.theirs_push(&[("notes/topic.md", &neuron("", "# Topic, theirs\n"))], &[]);
    let oid = w
        .run(&w.theirs, &["rev-parse", "HEAD:notes/topic.md"])
        .trim()
        .to_string();
    w.ours_commit(
        &[("notes/topic.md/child.md", &neuron("", "# Child\n"))],
        &[],
    );
    assert_conflict(&w, "notes/topic.md", ConflictKind::Structure);
    let r = w.sync().unwrap();
    assert_eq!(r.conflicts[0].theirs.as_deref(), Some(oid.as_str()));
}

#[test]
fn a_rename_on_one_side_and_an_edit_on_the_other_merge_as_git_does() {
    let w = World::new();
    w.run(&w.ours, &["mv", "notes/keep.md", "notes/renamed.md"]);
    w.commit(&w.ours, &["notes/keep.md", "notes/renamed.md"]);
    let edited = w
        .read(&w.theirs, "notes/keep.md")
        .replace("# Keep", "# Keep\n\nTheir approved line.");
    w.theirs_push(&[("notes/keep.md", &edited)], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(
        w.read(&w.ours, "notes/renamed.md")
            .contains("Their approved line.")
    );
    assert!(!w.ours.join("notes").join("keep.md").exists());
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn a_rename_on_one_side_and_a_delete_on_the_other_goes_to_a_person() {
    let w = World::new();
    w.run(&w.ours, &["mv", "notes/keep.md", "notes/renamed.md"]);
    w.commit(&w.ours, &["notes/keep.md", "notes/renamed.md"]);
    w.theirs_push(&[], &["notes/keep.md"]);
    assert_conflict(&w, "notes/renamed.md", ConflictKind::Structure);
}

// --- ignored files, autostash, a late edit ---------------------------------------------

/// Ours ignores `private/` locally; the teammate starts tracking a file there.
fn ignored_in_the_way(w: &World) {
    fs::write(
        w.ours.join(".git").join("info").join("exclude"),
        "private/\n",
    )
    .unwrap();
    w.theirs_push(&[("private/draft.md", "TEAM VERSION\n")], &[]);
}

#[test]
fn an_ignored_file_written_by_a_hook_before_the_fast_forward_blocks() {
    let w = World::new();
    ignored_in_the_way(&w);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.hook(
        &w.ours,
        "pre-merge-commit",
        "mkdir -p private; printf 'MINE\\n' > private/draft.md",
    );
    let (head, remote) = (w.head(&w.ours), w.remote_head());
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["private/draft.md"], "{r:?}");
    assert!(!r.merged && !r.pushed);
    assert_eq!(w.read(&w.ours, "private/draft.md"), "MINE\n");
    assert_eq!((w.head(&w.ours), w.remote_head()), (head, remote));
}

#[test]
fn a_tracked_edit_made_before_the_fast_forward_blocks_even_with_autostash() {
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
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["ideas/idea.md"], "{r:?}");
    assert_eq!(w.read(&w.ours, "ideas/idea.md"), "# MINE, UNCOMMITTED\n");
    assert_eq!(w.run(&w.ours, &["stash", "list"]), "");
    assert_eq!((w.head(&w.ours), w.remote_head()), (head, remote));
    no_markers_anywhere(&w, &w.ours);
}

/// The fast-forward itself: an ignored file in the way and a local edit
/// both stop it, whatever merge.autostash says; nothing is stashed.
#[test]
fn the_fast_forward_never_overwrites_an_ignored_file_or_stashes() {
    use kluris_core::git::Git;
    use kluris_core::sync::plumbing::SyncGit;
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

// --- merge message hooks -----------------------------------------------------------------

fn diverged(w: &World) {
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
}

#[test]
fn a_failing_commit_msg_hook_stops_the_merge() {
    let w = World::new();
    diverged(&w);
    w.hook(&w.ours, "commit-msg", "exit 23");
    let before = w.snapshot();
    match w.sync() {
        Err(SyncError::Hook { name, code }) => assert_eq!((name, code), ("commit-msg", 23)),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
}

#[test]
fn the_message_hooks_shape_the_merge_message() {
    let w = World::new();
    diverged(&w);
    let seen = w.dir.path().join("prepare-args");
    w.hook(
        &w.ours,
        "prepare-commit-msg",
        &format!(
            "echo \"$2\" > '{}'\necho 'Prepared: yes' >> \"$1\"",
            seen.display().to_string().replace('\\', "/")
        ),
    );
    w.hook(
        &w.ours,
        "commit-msg",
        "sed 's/^kluris: sync$/kluris: sync (checked)/' \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"",
    );
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed, "{r:?}");
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        "kluris: sync (checked)\n\nKluris-Sync: auto\nPrepared: yes\n\n"
    );
    assert_eq!(fs::read_to_string(&seen).unwrap().trim(), "merge");
}

#[test]
fn no_verify_skips_commit_msg_but_not_prepare_commit_msg() {
    let w = World::new();
    diverged(&w);
    w.hook(&w.ours, "commit-msg", "exit 23");
    w.hook(&w.ours, "pre-merge-commit", "exit 1");
    w.hook(
        &w.ours,
        "prepare-commit-msg",
        "echo 'Prepared: yes' >> \"$1\"",
    );
    let r = sync(
        &w.ours,
        &w.git(),
        &SyncRequest {
            no_verify: true,
            ..w.request()
        },
    )
    .unwrap();
    assert!(r.merged && r.pushed, "{r:?}");
    assert!(
        w.run(&w.ours, &["log", "-1", "--format=%B"])
            .contains("\nPrepared: yes\n")
    );
}

#[test]
fn a_failing_prepare_commit_msg_hook_stops_the_merge() {
    let w = World::new();
    diverged(&w);
    w.hook(&w.ours, "prepare-commit-msg", "exit 4");
    let before = w.snapshot();
    match w.sync() {
        Err(SyncError::Hook { name, code }) => assert_eq!((name, code), ("prepare-commit-msg", 4)),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
}

// --- the merge never writes markers, never loses a change ----------------------------------

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn file(created: &str, updated: &str, tags: &[String], body: &[String]) -> String {
    let mut text = format!("---\ncreated: '{created}'\nparent: ./map.md\n");
    if !tags.is_empty() {
        text.push_str("tags:\n");
        for t in tags {
            text.push_str(&format!("- {t}\n"));
        }
    }
    text.push_str(&format!("updated: '{updated}'\n---\n\n"));
    text.push_str(&body.join("\n"));
    text.push('\n');
    text
}

#[test]
fn auto_merges_never_write_markers_and_keep_every_change() {
    let w = World::new();
    let git = w.git();
    let cases: usize = std::env::var("KLURIS_SYNC_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..cases {
        let n = 8 + rng.below(20);
        let base_body: Vec<String> = (0..n).map(|i| format!("line {i}")).collect();
        let base_tags: Vec<String> = (0..rng.below(3)).map(|i| format!("t{i}")).collect();
        // Ours edits in the first half, theirs in the second, at least two
        // unchanged lines apart: never overlapping.
        let i = rng.below(n / 2 - 1);
        let j = n / 2 + 1 + rng.below(n - n / 2 - 1);
        let mut ours_body = base_body.clone();
        ours_body[i] = format!("ours {case}");
        if rng.below(2) == 0 {
            ours_body.insert(i + 1, format!("ours added {case}"));
        }
        let mut theirs_body = base_body.clone();
        theirs_body[j] = format!("theirs {case}");
        let mut ours_tags = base_tags.clone();
        ours_tags.push(format!("o{}", rng.below(5)));
        let mut theirs_tags = base_tags.clone();
        theirs_tags.push(format!("h{}", rng.below(5)));
        let day = |r: &mut Rng| format!("2026-0{}-1{}", 1 + r.below(9), r.below(10));
        let (oc, ou, tc, tu) = (day(&mut rng), day(&mut rng), day(&mut rng), day(&mut rng));
        let base = file("2026-01-01", "2026-01-01", &base_tags, &base_body);
        let ours = file(&oc, &ou, &ours_tags, &ours_body);
        let theirs = file(&tc, &tu, &theirs_tags, &theirs_body);
        let merged = merge_markdown(
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &git,
        )
        .unwrap_or_else(|kind| panic!("case {case}: {kind:?}\n{ours}\n{theirs}"));
        let text = String::from_utf8(merged.bytes).unwrap();
        no_markers(&text);
        for line in ours_body
            .iter()
            .chain(&theirs_body)
            .filter(|l| !l.starts_with("line "))
        {
            assert!(
                text.contains(line.as_str()),
                "case {case} lost {line:?}:\n{text}"
            );
        }
        for tag in ours_tags.iter().chain(&theirs_tags) {
            assert!(
                text.contains(&format!("- {tag}\n")),
                "case {case} lost tag {tag}:\n{text}"
            );
        }
        let (created, updated) = (oc.clone().min(tc.clone()), ou.clone().max(tu.clone()));
        assert!(
            text.contains(&format!("created: '{created}'\n")),
            "case {case}:\n{text}"
        );
        assert!(
            text.contains(&format!("updated: '{updated}'\n")),
            "case {case}:\n{text}"
        );
    }
}
