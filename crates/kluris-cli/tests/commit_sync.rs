//! `kluris commit` syncs: it opens an explicit session (teammates' commits
//! are pulled before it commits), commits the given paths with the maps and
//! the reverse links they change, and closes by pulling what came in since
//! and pushing. With a merge waiting, `kluris commit --merge <id>` finishes
//! it from the settled files in the work tree. The commit's exit code is
//! the commit's: the sync's outcome is in the `sync` object.
//!
//! Real git, the built binary under a sandbox HOME and `KLURIS_CONFIG`,
//! and local bare remotes.

mod sync_home;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use std::time::Duration;

use kluris_core::sync::claim::TreeClaim;
use serde_json::{Value, json};
use sync_home::*;

/// The `sync` object's keys, in order.
const SYNC_KEYS: [&str; 8] = [
    "pulled",
    "auto_merged",
    "left_as_is",
    "conflicts",
    "behind_conflict",
    "to_push",
    "pushed",
    "last_push",
];

/// `commit --json`'s keys, in order.
const COMMIT_KEYS: [&str; 9] = [
    "ok",
    "brain",
    "committed",
    "commit",
    "paths",
    "regenerated",
    "linked",
    "left_dirty",
    "sync",
];

/// A path as git's shell takes it (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A file under the brain's git directory.
fn git_file(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.join(".git"), |path, part| path.join(part))
}

/// A home with the brain `alpha`, pushed to its remote, and its teammate.
fn team() -> (Home, PathBuf) {
    let mut h = Home::new();
    let root = h.brain("alpha");
    (h, root)
}

/// `kluris commit --brain alpha --paths <paths> [extra] --json`.
fn commit(h: &Home, paths: &str, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["commit", "--brain", "alpha", "--paths", paths];
    args.extend_from_slice(extra);
    args.push("--json");
    h.json(&args)
}

/// The `sync` object of `doc`, checked to have its eight keys.
fn sync_of(doc: &Value) -> &Value {
    let sync = &doc["sync"];
    assert_eq!(keys(sync), SYNC_KEYS, "{doc}");
    sync
}

fn reason(sync: &Value) -> &Value {
    &sync["left_as_is"]["reason"]
}

fn head(h: &Home, root: &Path) -> String {
    h.git(root, &["rev-parse", "HEAD"]).trim().to_string()
}

/// What the remote's `main` holds.
fn remote_main(h: &Home) -> String {
    let bare = h.dir.path().join("alpha.git");
    h.git(&bare, &["rev-parse", "main"]).trim().to_string()
}

/// The parents of `rev`.
fn parents(h: &Home, root: &Path, rev: &str) -> Vec<String> {
    h.git(root, &["rev-list", "--parents", "-n", "1", rev])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// A file at `rev`, as text.
fn show(h: &Home, root: &Path, rev: &str, rel: &str) -> String {
    h.git(root, &["show", &format!("{rev}:{rel}")])
}

/// The paths `rev` changed, sorted.
fn changed(h: &Home, root: &Path, rev: &str) -> Vec<String> {
    let mut paths: Vec<String> = h
        .git(root, &["show", "--name-only", "--format=", rev])
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths
}

/// `git status --porcelain`, every untracked file listed.
fn status(h: &Home, root: &Path) -> String {
    h.git(root, &["status", "--porcelain", "--untracked-files=all"])
}

/// A neuron about zebras.
fn zebra() -> String {
    neuron("Zebra", "All about zebras.\n")
}

/// A neuron whose `related` lists `links`.
fn linking(title: &str, links: &[&str]) -> String {
    let related: String = links.iter().map(|l| format!("- {l}\n")).collect();
    format!(
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n{related}updated: '2026-01-01'\n---\n\n# {title}\n\n{BODY}"
    )
}

// --- a plain commit ------------------------------------------------------------------

#[test]
fn a_commit_opens_an_explicit_session_commits_then_pushes() {
    let (h, root) = team();
    // A read inside the throttle would fetch nothing: an explicit session
    // always fetches.
    let stamp = git_file(&root, "kluris-sync/fetch-stamp");
    fs::create_dir_all(stamp.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(&stamp, format!("{now}\n")).unwrap();
    h.teammate_pushes("alpha", "notes/zebra.md", &zebra());
    let theirs = remote_main(&h);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), COMMIT_KEYS, "{doc}");
    assert_eq!(doc["committed"], true);
    let sync = sync_of(&doc);
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(sync["to_push"], 0, "{doc}");
    assert_eq!(reason(sync), &Value::Null, "{doc}");
    // Pulled first, then committed on top, then pushed.
    let head = head(&h, &root);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(parents(&h, &root, &head), [theirs]);
    assert_eq!(remote_main(&h), head);
    assert_eq!(
        changed(&h, &root, &head),
        ["notes/map.md", "notes/second.md"]
    );
    assert_eq!(status(&h, &root), "");
}

#[test]
fn a_teammates_commit_pushed_while_it_commits_is_pulled_at_close_and_pushed() {
    let (h, root) = team();
    // The teammate's commit reaches the remote once this commit is made,
    // before the push: the push is turned back, pulled again and retried.
    let mate = h.teammate("alpha");
    h.commit(&mate, "notes/zebra.md", &zebra());
    let hooks = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("post-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nunset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE GIT_PREFIX\ncd '{}' && git push -q\n",
            sh_path(&mate)
        ),
    )
    .unwrap();
    make_executable(&hook);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let sync = sync_of(&doc);
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(sync["to_push"], 0, "{doc}");
    let head = head(&h, &root);
    let ours = doc["commit"].as_str().unwrap().to_string();
    assert_eq!(
        parents(&h, &root, &head)[0],
        ours,
        "a merge on top of the commit"
    );
    assert_eq!(parents(&h, &root, &head).len(), 2);
    assert_eq!(remote_main(&h), head);
    assert!(root.join("notes").join("zebra.md").is_file());
    assert_eq!(status(&h, &root), "");
}

#[test]
fn the_persons_paths_in_the_way_of_the_first_pull_are_committed_then_pulled_and_pushed() {
    let (h, root) = team();
    // The teammate changes the first paragraph; the person, uncommitted,
    // the last one: the open's pull would write over the person's file.
    h.teammate_pushes(
        "alpha",
        "notes/first.md",
        &neuron("First", &BODY.replace("One.", "One, theirs.")),
    );
    let theirs = remote_main(&h);
    h.write(
        &root,
        "notes/first.md",
        &neuron("First", &BODY.replace("Five.", "Five, ours.")),
    );

    let (code, doc) = commit(&h, "notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let sync = sync_of(&doc);
    // Where the brain ended: the open's `uncommitted` was cleared by the
    // close's pull, so it is not said.
    assert_eq!(reason(sync), &Value::Null, "{doc}");
    assert_eq!(sync["left_as_is"]["paths"], json!([]), "{doc}");
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["auto_merged"], json!(["notes/first.md"]), "{doc}");
    assert_eq!(sync["conflicts"], json!([]), "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    let head = head(&h, &root);
    let ours = doc["commit"].as_str().unwrap().to_string();
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    assert_eq!(remote_main(&h), head);
    let first = h.read(&root, "notes/first.md");
    assert!(
        first.contains("One, theirs.") && first.contains("Five, ours."),
        "{first}"
    );
    assert_eq!(status(&h, &root), "");
}

#[test]
fn it_commits_only_the_given_paths_and_the_maps_they_change() {
    let (h, root) = team();
    let before = head(&h, &root);
    let brain_md = h.read(&root, "brain.md");
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    h.write(
        &root,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Changed\n",
    );
    h.write(&root, "ideas/draft.md", &neuron("Draft", BODY));

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), COMMIT_KEYS, "{doc}");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["brain"], "alpha");
    assert_eq!(doc["committed"], true);
    let head = head(&h, &root);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(doc["paths"], json!(["notes/second.md"]));
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["linked"], json!([]));
    assert_eq!(doc["left_dirty"], json!(["glossary.md", "ideas/draft.md"]));
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");

    assert_eq!(
        changed(&h, &root, &head),
        ["notes/map.md", "notes/second.md"]
    );
    assert_eq!(parents(&h, &root, &head), [before]);
    assert!(
        h.read(&root, "notes/map.md")
            .contains("- [second.md](./second.md) — Second")
    );
    // A generated file the paths do not change is not written.
    assert_eq!(h.read(&root, "brain.md"), brain_md);
    assert_eq!(status(&h, &root), " M glossary.md\n?? ideas/draft.md\n");
}

#[test]
fn nothing_to_commit_exits_0_with_committed_false_and_the_sync() {
    let (h, root) = team();
    h.write(
        &root,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Changed\n",
    );
    let before = head(&h, &root);
    let (code, doc) = commit(&h, "notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc,
        json!({
            "ok": true,
            "brain": "alpha",
            "committed": false,
            "commit": null,
            "paths": ["notes/first.md"],
            "regenerated": [],
            "linked": [],
            "left_dirty": ["glossary.md"],
            "sync": {
                "pulled": 0,
                "auto_merged": [],
                "left_as_is": {"reason": null, "paths": [], "detail": null},
                "conflicts": [],
                "behind_conflict": 0,
                "to_push": 0,
                "pushed": false,
                "last_push": null,
            },
        })
    );
    assert_eq!(head(&h, &root), before);
}

// --- the consequences it takes ---------------------------------------------------------

#[test]
fn the_reverse_links_its_paths_cause_are_taken_into_otherwise_clean_neurons() {
    let (h, root) = team();
    // A committed neuron that links to `first.md` one way: its missing
    // link back is not this commit's to take.
    h.commit(&root, "notes/third.md", &linking("Third", &["./first.md"]));
    h.git(&root, &["push", "-q"]);
    h.write(
        &root,
        "notes/second.md",
        &linking("Second", &["./first.md"]),
    );

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!(["notes/second.md"]));
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["linked"], json!(["notes/first.md"]), "{doc}");
    let head = head(&h, &root);
    assert_eq!(
        changed(&h, &root, &head),
        ["notes/first.md", "notes/map.md", "notes/second.md"]
    );
    let first = show(&h, &root, &head, "notes/first.md");
    assert!(first.contains("related:\n- second.md\n"), "{first}");
    assert!(!first.contains("third.md"), "{first}");
    // The work tree holds what was committed: nothing is left changed.
    assert_eq!(h.read(&root, "notes/first.md"), first);
    assert_eq!(status(&h, &root), "");
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
}

#[test]
fn a_neuron_with_edits_of_its_own_never_gets_a_reverse_link() {
    let (h, root) = team();
    let edited = neuron("First", &BODY.replace("Two.", "Two, edited."));
    h.write(&root, "notes/first.md", &edited);
    h.write(
        &root,
        "notes/second.md",
        &linking("Second", &["./first.md"]),
    );

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["linked"], json!([]), "{doc}");
    assert_eq!(doc["left_dirty"], json!(["notes/first.md"]), "{doc}");
    let head = head(&h, &root);
    assert_eq!(
        changed(&h, &root, &head),
        ["notes/map.md", "notes/second.md"]
    );
    assert_eq!(h.read(&root, "notes/first.md"), edited);
}

/// `kluris dream --brain alpha`, then the background push it started when
/// it left commits waiting.
fn dream(h: &Home, root: &Path) {
    let (_, out, err) = h.run(&["dream", "--brain", "alpha", "--json"]);
    let doc: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}{err}"));
    let waits = doc["sync"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|item| item["to_push"].as_u64().is_some_and(|n| n > 0));
    if waits {
        wait_for_background(root);
    }
}

/// `alpha` with `notes/second.md`, `notes/third.md` and `notes/fourth.md`
/// committed, tidied by a dream (so HEAD needs no housekeeping) and pushed.
fn tidy_team() -> (Home, PathBuf) {
    let (h, root) = team();
    for (rel, title) in [
        ("notes/second.md", "Second"),
        ("notes/third.md", "Third"),
        ("notes/fourth.md", "Fourth"),
    ] {
        h.write(&root, rel, &neuron(title, BODY));
    }
    h.git(&root, &["add", "-A"]);
    h.git(&root, &["commit", "-q", "-m", "more"]);
    h.git(&root, &["push", "-q"]);
    dream(&h, &root);
    assert_eq!(status(&h, &root), "");
    (h, root)
}

#[test]
fn a_reverse_link_dream_already_wrote_is_taken() {
    let (h, root) = tidy_team();
    // The write, then dream, then the commit of the write.
    let first = h.read(&root, "notes/first.md").replace(
        "parent: ./map.md\n",
        "parent: ./map.md\nrelated:\n- ./second.md\n",
    );
    h.write(&root, "notes/first.md", &first);
    dream(&h, &root);
    assert!(h.read(&root, "notes/second.md").contains("- first.md\n"));

    let (code, doc) = commit(&h, "notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["linked"], json!(["notes/second.md"]), "{doc}");
    assert_eq!(doc["left_dirty"], json!([]), "{doc}");
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
    assert_eq!(status(&h, &root), "");
    // The teammate reads both links.
    let mate = h.teammate("alpha");
    h.git(&mate, &["pull", "-q"]);
    assert!(h.read(&mate, "notes/first.md").contains("- ./second.md\n"));
    assert!(h.read(&mate, "notes/second.md").contains("- first.md\n"));
}

#[test]
fn a_neuron_with_an_edit_or_a_fix_that_is_not_the_commits_stays_out() {
    let (h, root) = tidy_team();
    // `third.md` has an edit of its own; `fourth.md` gets, beside this
    // commit's reverse link, one an untracked draft calls for.
    let third = h
        .read(&root, "notes/third.md")
        .replace("Two.", "Two, edited.");
    h.write(&root, "notes/third.md", &third);
    h.write(&root, "notes/draft.md", &linking("Draft", &["./fourth.md"]));
    let first = h.read(&root, "notes/first.md").replace(
        "parent: ./map.md\n",
        "parent: ./map.md\nrelated:\n- ./third.md\n- ./fourth.md\n",
    );
    h.write(&root, "notes/first.md", &first);
    dream(&h, &root);
    let fourth = h.read(&root, "notes/fourth.md");
    assert!(
        fourth.contains("- first.md\n") && fourth.contains("- draft.md\n"),
        "{fourth}"
    );
    let third = h.read(&root, "notes/third.md");

    let (code, doc) = commit(&h, "notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["linked"], json!([]), "{doc}");
    let head = head(&h, &root);
    assert!(
        !changed(&h, &root, &head)
            .iter()
            .any(|rel| rel == "notes/third.md" || rel == "notes/fourth.md")
    );
    assert_eq!(h.read(&root, "notes/third.md"), third);
    assert_eq!(h.read(&root, "notes/fourth.md"), fourth);
}

#[test]
fn a_staged_version_of_a_map_the_commit_changes_is_kept() {
    let (h, root) = team();
    let head_map = h.read(&root, "notes/map.md");
    let staged_map = format!("{head_map}\nStaged note.\n");
    h.write(&root, "notes/map.md", &staged_map);
    h.git(&root, &["add", "notes/map.md"]);
    h.write(&root, "notes/map.md", &head_map);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]), "{doc}");
    let head = head(&h, &root);
    let map = show(&h, &root, &head, "notes/map.md");
    assert!(map.contains("second.md"), "{map}");
    assert_eq!(h.git(&root, &["show", ":notes/map.md"]), staged_map);
    assert_eq!(h.read(&root, "notes/map.md"), map);
}

#[test]
fn a_commit_that_landed_exits_0_and_warns_of_what_failed_after() {
    let (h, root) = team();
    // A pre-commit hook that leaves the index locked: the commit lands, the
    // index cannot be updated after it.
    let lock = root.join(".git").join("index.lock");
    let hooks = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", sh_path(&lock))).unwrap();
    make_executable(&hook);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let before = head(&h, &root);

    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["committed"], true);
    let head = head(&h, &root);
    assert_ne!(head, before);
    assert_eq!(doc["commit"], head.as_str());
    let warning = doc["warning"]
        .as_str()
        .unwrap_or_else(|| panic!("no warning: {doc}"));
    assert!(
        warning.contains("git reset -q -- notes/second.md notes/map.md"),
        "{warning}"
    );
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
    assert_eq!(remote_main(&h), head);
    let mut expected: Vec<&str> = COMMIT_KEYS[..8].to_vec();
    expected.extend(["warning", "sync"]);
    assert_eq!(keys(&doc), expected, "{doc}");

    // Text: the warning on stderr.
    fs::remove_file(&lock).unwrap();
    h.write(&root, "notes/third.md", &neuron("Third", BODY));
    let (code, _, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/third.md"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("Warning: "), "{err}");
    fs::remove_file(&lock).unwrap();
}

/// The index flags that hide a path's work-tree copy from `git status`.
const HIDING: [&str; 2] = ["--assume-unchanged", "--skip-worktree"];

#[test]
fn a_neuron_git_status_does_not_show_gets_no_reverse_link() {
    for flag in HIDING {
        let (h, root) = team();
        let committed = h.read(&root, "notes/first.md");
        let edited = format!("{committed}\nHidden paragraph.\n");
        h.write(&root, "notes/first.md", &edited);
        h.git(&root, &["update-index", flag, "notes/first.md"]);
        h.write(
            &root,
            "notes/second.md",
            &linking("Second", &["./first.md"]),
        );
        let (code, doc) = commit(&h, "notes/second.md", &[]);
        assert_eq!(code, 0, "{flag}: {doc}");
        assert_eq!(doc["linked"], json!([]), "{flag}: {doc}");
        let head = head(&h, &root);
        assert_eq!(
            show(&h, &root, &head, "notes/first.md"),
            committed,
            "{flag}"
        );
        assert_eq!(h.read(&root, "notes/first.md"), edited, "{flag}");
    }
}

#[test]
fn a_map_git_status_does_not_show_is_left_out_and_never_written() {
    for flag in HIDING {
        let (h, root) = team();
        let committed = h.read(&root, "notes/map.md");
        let edited = format!("{committed}\nMy own line.\n");
        h.write(&root, "notes/map.md", &edited);
        h.git(&root, &["update-index", flag, "notes/map.md"]);
        h.write(&root, "notes/second.md", &neuron("Second", BODY));
        let (code, doc) = commit(&h, "notes/second.md", &[]);
        assert_eq!(code, 0, "{flag}: {doc}");
        assert_eq!(doc["regenerated"], json!([]), "{flag}: {doc}");
        let head = head(&h, &root);
        assert_eq!(changed(&h, &root, &head), ["notes/second.md"], "{flag}");
        assert_eq!(show(&h, &root, &head, "notes/map.md"), committed, "{flag}");
        assert_eq!(h.read(&root, "notes/map.md"), edited, "{flag}");
    }
}

#[cfg(unix)]
#[test]
fn a_merge_that_landed_exits_0_when_the_check_after_it_fails() {
    let (mut h, root) = team();
    let (merge, ours, theirs) = waiting(&h, &root);
    h.write(&root, "notes/first.md", &settled());
    let failed = h.fail_the_check_after_a_land();
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert!(failed.exists(), "the check never failed: {doc}");
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true, "{doc}");
    let head = head(&h, &root);
    assert_eq!(doc["commit"], head.as_str(), "{doc}");
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    let warning = doc["warning"]
        .as_str()
        .unwrap_or_else(|| panic!("no warning: {doc}"));
    assert!(warning.contains("could not be read"), "{warning}");
    assert!(!git_file(&root, "kluris-sync/pending.json").exists());
    assert_eq!(sync_of(&doc)["conflicts"], json!([]), "{doc}");
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
}

// --- the exit code is the commit's -----------------------------------------------------

#[test]
fn offline_or_refused_the_commit_lands_and_exits_0() {
    // A port nothing listens on: offline.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let (h, root) = team();
    let gone = h.dir.path().join("gone.git").display().to_string();
    let offline = format!("git://127.0.0.1:{closed}/alpha.git");
    for (n, (url, expected)) in [(offline.as_str(), "offline"), (gone.as_str(), "refused")]
        .into_iter()
        .enumerate()
    {
        h.git(&root, &["remote", "set-url", "origin", url]);
        let rel = format!("notes/n{n}.md");
        h.write(&root, &rel, &neuron("N", BODY));
        let before = head(&h, &root);
        let (code, doc) = commit(&h, &rel, &[]);
        assert_eq!(code, 0, "{expected}: {doc}");
        assert_eq!(doc["committed"], true, "{expected}: {doc}");
        let sync = sync_of(&doc);
        assert_eq!(reason(sync), expected, "{doc}");
        assert_eq!(sync["pushed"], false, "{doc}");
        assert_eq!(sync["to_push"], n + 1, "{doc}");
        assert_eq!(parents(&h, &root, "HEAD"), [before]);
    }
}

#[test]
fn a_new_conflict_on_the_close_is_recorded_reported_and_exits_0() {
    let (h, root) = team();
    h.teammate_pushes(
        "alpha",
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, theirs.")),
    );
    // In the way of the open's pull, then committed: the close's pull
    // meets the conflict.
    h.write(
        &root,
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, ours.")),
    );
    let (code, doc) = commit(&h, "notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true);
    let sync = sync_of(&doc);
    let conflicts = sync["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{doc}");
    assert_eq!(conflicts[0]["path"], "notes/first.md");
    assert_eq!(conflicts[0]["merge"], h.merge("alpha").as_str());
    assert!(
        conflicts[0]["ours"]
            .as_str()
            .unwrap()
            .contains("Three, ours.")
    );
    assert_eq!(reason(sync), &Value::Null, "{doc}");
    assert_eq!(sync["pushed"], false, "{doc}");
    assert_eq!(head(&h, &root), doc["commit"].as_str().unwrap());
    assert!(git_file(&root, "kluris-sync/pending.json").is_file());
}

// --- what it prints --------------------------------------------------------------------

#[test]
fn the_text_says_what_was_committed_and_the_shared_line_what_the_sync_did() {
    let (h, root) = team();
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    h.write(
        &root,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Changed\n",
    );
    let (code, out, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/second.md"]);
    assert_eq!(code, 0, "{err}");
    let short = &head(&h, &root)[..7];
    assert_eq!(
        out,
        format!(
            "Committed {short} in alpha: kluris: update notes/second.md\n  notes/second.md\n  notes/map.md (regenerated)\nLeft uncommitted: glossary.md\n"
        )
    );
    assert_eq!(err, "kluris: alpha: pushed\n");
    let (code, out, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/second.md"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "Nothing to commit in alpha.\nLeft uncommitted: glossary.md\n"
    );
    assert_eq!(err, "");
    // A reverse link it took is named.
    h.write(&root, "notes/third.md", &linking("Third", &["./second.md"]));
    let (code, out, _) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/third.md"]);
    assert_eq!(code, 0);
    assert!(out.contains("\n  notes/second.md (linked)\n"), "{out}");
}

#[test]
fn a_brain_without_a_remote_is_local_and_says_nothing_of_a_sync() {
    let (h, root) = team();
    h.git(&root, &["remote", "remove", "origin"]);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), COMMIT_KEYS[..8], "{doc}");
    assert_eq!(doc["committed"], true);
    h.write(&root, "notes/third.md", &neuron("Third", BODY));
    let (code, _, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/third.md"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(err, "");
    // A failure says nothing of a sync either.
    let (code, doc) = commit(&h, "../outside.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "error"], "{doc}");
}

#[test]
fn a_brain_with_a_remote_but_no_upstream_says_how_to_set_one() {
    let (h, root) = team();
    h.git(&root, &["branch", "--unset-upstream"]);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), COMMIT_KEYS, "{doc}");
    let sync = sync_of(&doc);
    assert_eq!(reason(sync), "no_upstream", "{doc}");
    assert_eq!(
        sync["left_as_is"]["detail"],
        "set one with git push -u origin main"
    );
    h.write(&root, "notes/third.md", &neuron("Third", BODY));
    let (code, _, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/third.md"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        err,
        "kluris: alpha: left as is (no_upstream): set one with git push -u origin main\n"
    );
}

// --- a failure after the open says what the sync did ---------------------------------

#[test]
fn a_path_refused_after_the_open_says_what_the_sync_did_and_pushes_what_waited() {
    let (h, root) = team();
    h.teammate_pushes("alpha", "notes/zebra.md", &zebra());
    // A commit of the person's own, waiting to be pushed.
    h.commit(&root, "notes/local.md", &neuron("Local", BODY));
    let (code, doc) = commit(&h, "../outside.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "error", "sync"], "{doc}");
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"].as_str().unwrap().contains("outside the brain"),
        "{doc}"
    );
    let sync = sync_of(&doc);
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(sync["to_push"], 0, "{doc}");
    assert_eq!(remote_main(&h), head(&h, &root));
    assert!(root.join("notes").join("zebra.md").is_file());
}

#[test]
fn a_hook_that_stops_the_commit_still_says_what_the_sync_did() {
    let (h, root) = team();
    h.teammate_pushes("alpha", "notes/zebra.md", &zebra());
    let theirs = remote_main(&h);
    let hooks = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    fs::write(&hook, "#!/bin/sh\necho 'no, thanks' >&2\nexit 1\n").unwrap();
    make_executable(&hook);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert!(
        doc["error"].as_str().unwrap().contains("pre-commit"),
        "{doc}"
    );
    let sync = sync_of(&doc);
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(
        head(&h, &root),
        theirs,
        "the pull landed, nothing was committed"
    );
    // The text: the shared line, then the error.
    h.teammate_pushes("alpha", "notes/yak.md", &neuron("Yak", BODY));
    let (code, _, err) = h.run(&["commit", "--brain", "alpha", "--paths", "notes/second.md"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("kluris: alpha: pulled 1 commit\n"), "{err}");
    assert!(err.contains("Error: the pre-commit hook failed"), "{err}");
}

#[test]
fn a_work_tree_another_command_holds_refuses_the_commit_and_says_so() {
    let (h, root) = team();
    let held = TreeClaim::exclusive(&root.join(".git"), Duration::ZERO).unwrap();
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let before = head(&h, &root);
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    drop(held);
    assert_eq!(code, 1, "{doc}");
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("nothing was committed"),
        "{doc}"
    );
    assert_eq!(reason(sync_of(&doc)), "busy", "{doc}");
    assert_eq!(head(&h, &root), before);
}

// --- finishing a waiting merge ---------------------------------------------------------

/// `alpha` with the waiting merge of an overlapping edit of
/// `notes/first.md` (recorded by `kluris sync`); its id, and `ours` and
/// `theirs`.
fn waiting(h: &Home, root: &Path) -> (String, String, String) {
    h.conflict("alpha", root);
    merge_of(h, root)
}

/// The waiting merge `kluris sync` records: its id, `ours` and `theirs`.
fn merge_of(h: &Home, root: &Path) -> (String, String, String) {
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let merge = doc["brains"][0]["sync"]["conflicts"][0]["merge"]
        .as_str()
        .unwrap_or_else(|| panic!("no conflict: {doc}"))
        .to_string();
    (merge, head(h, root), remote_main(h))
}

/// The settled text of `notes/first.md`.
fn settled() -> String {
    neuron("First", &BODY.replace("Three.", "Three, settled."))
}

#[test]
fn finishing_a_merge_lands_with_the_resolved_blobs_in_the_index() {
    let (h, root) = team();
    let (merge, ours, theirs) = waiting(&h, &root);
    h.write(&root, "notes/first.md", &settled());

    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), COMMIT_KEYS, "{doc}");
    assert_eq!(doc["committed"], true);
    assert_eq!(doc["paths"], json!(["notes/first.md"]));
    let head = head(&h, &root);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    assert_eq!(show(&h, &root, &head, "notes/first.md"), settled());
    // The index and the work tree are the merge's: nothing is left
    // changed or staged.
    assert_eq!(h.read(&root, "notes/first.md"), settled());
    assert_eq!(status(&h, &root), "");
    assert_eq!(h.git(&root, &["diff", "--cached", "--name-only"]), "");
    assert!(!git_file(&root, "kluris-sync/pending.json").exists());
    let sync = sync_of(&doc);
    assert_eq!(sync["conflicts"], json!([]), "{doc}");
    assert_eq!(reason(sync), &Value::Null, "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(remote_main(&h), head);
}

#[test]
fn without_merge_or_with_a_wrong_id_it_is_refused_with_the_current_texts_and_id() {
    let (h, root) = team();
    let (merge, ours, _) = waiting(&h, &root);
    h.write(&root, "notes/first.md", &settled());
    let pending = fs::read(git_file(&root, "kluris-sync/pending.json")).unwrap();
    for extra in [&[][..], &["--merge", "000000000000"][..]] {
        let (code, doc) = commit(&h, "notes/first.md", extra);
        assert_eq!(code, 1, "{extra:?}: {doc}");
        assert_eq!(doc["ok"], false);
        let error = doc["error"].as_str().unwrap();
        assert!(error.contains(&merge), "{error}");
        assert!(error.contains("nothing was committed"), "{error}");
        let sync = sync_of(&doc);
        let conflicts = sync["conflicts"].as_array().unwrap();
        assert_eq!(conflicts.len(), 1, "{doc}");
        assert_eq!(conflicts[0]["merge"], merge.as_str());
        assert!(
            conflicts[0]["ours"]
                .as_str()
                .unwrap()
                .contains("Three, ours.")
        );
        assert!(
            conflicts[0]["theirs"]
                .as_str()
                .unwrap()
                .contains("Three, theirs.")
        );
        assert_eq!(head(&h, &root), ours);
        assert_eq!(
            fs::read(git_file(&root, "kluris-sync/pending.json")).unwrap(),
            pending
        );
        assert_eq!(h.read(&root, "notes/first.md"), settled());
    }
    // An id while no merge waits.
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    assert!(
        doc["error"].as_str().unwrap().contains("no merge waits"),
        "{doc}"
    );
}

/// `alpha` with the waiting merge of overlapping edits of `notes/first.md`
/// and `notes/second.md`; its id.
fn two_waiting(h: &Home, root: &Path) -> String {
    h.commit(root, "notes/second.md", &neuron("Second", BODY));
    h.git(root, &["push", "-q"]);
    let mate = h.teammate("alpha");
    h.git(&mate, &["pull", "-q"]);
    for (rel, title) in [("notes/first.md", "First"), ("notes/second.md", "Second")] {
        h.write(
            &mate,
            rel,
            &neuron(title, &BODY.replace("Three.", "Three, theirs.")),
        );
        h.write(
            root,
            rel,
            &neuron(title, &BODY.replace("Three.", "Three, ours.")),
        );
    }
    h.git(&mate, &["commit", "-q", "-am", "theirs"]);
    h.git(&mate, &["push", "-q"]);
    h.git(root, &["commit", "-q", "-am", "ours"]);
    merge_of(h, root).0
}

#[test]
fn every_conflicted_path_must_be_settled_and_no_other_path_may_come_along() {
    let (h, root) = team();
    let merge = two_waiting(&h, &root);
    let before = head(&h, &root);
    h.write(&root, "notes/first.md", &settled());
    h.write(&root, "notes/other.md", &neuron("Other", BODY));

    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("settle these too: notes/second.md"),
        "{error}"
    );

    let (code, doc) = commit(
        &h,
        "notes/first.md,notes/second.md,notes/other.md",
        &["--merge", &merge],
    );
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("commit the conflicted paths first"),
        "{error}"
    );
    assert!(error.contains("notes/other.md"), "{error}");
    assert_eq!(head(&h, &root), before);
    assert!(git_file(&root, "kluris-sync/pending.json").is_file());

    // The conflicted paths, then the rest.
    let (code, doc) = commit(&h, "notes/first.md,notes/second.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let (code, doc) = commit(&h, "notes/other.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
    assert_eq!(status(&h, &root), "");
}

#[test]
fn a_settled_file_with_conflict_markers_is_refused() {
    let (h, root) = team();
    let (merge, ours, _) = waiting(&h, &root);
    h.write(
        &root,
        "notes/first.md",
        &neuron(
            "First",
            "<<<<<<< ours\nThree, ours.\n=======\nThree, theirs.\n>>>>>>> theirs\n",
        ),
    );
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(error.contains("conflict markers"), "{error}");
    assert_eq!(head(&h, &root), ours);
}

#[test]
fn a_structure_conflict_is_settled_with_resolve() {
    let (h, root) = team();
    h.teammate_pushes("alpha", "notes/topic.md/child.md", &neuron("Child", BODY));
    h.commit(&root, "notes/topic.md", &neuron("Topic", BODY));
    let (merge, ours, _) = merge_of(&h, &root);
    let (code, doc) = commit(&h, "notes/topic.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("settle it with `kluris resolve notes/topic.md --ours|--theirs`"),
        "{error}"
    );
    assert_eq!(head(&h, &root), ours);
}

/// `kluris resolve --brain alpha <rel> <choice> --merge <merge> --json`.
fn resolve(h: &Home, rel: &str, choice: &[&str], merge: &str) -> (i32, Value) {
    let mut args = vec!["resolve", "--brain", "alpha", rel];
    args.extend_from_slice(choice);
    args.extend(["--merge", merge, "--json"]);
    h.json(&args)
}

/// `alpha` with one waiting merge of two conflicts: an overlapping edit of
/// `notes/first.md`, and `notes/topic.md` a file here and a folder (with
/// `child.md`) in the teammate's commit. Its id, `ours` and `theirs`.
fn text_and_structure(h: &Home, root: &Path) -> (String, String, String) {
    let mate = h.teammate("alpha");
    h.write(&mate, "notes/first.md", &neuron("First", &theirs_body()));
    h.write(&mate, "notes/topic.md/child.md", &neuron("Child", BODY));
    h.git(&mate, &["add", "-A"]);
    h.git(&mate, &["commit", "-q", "-m", "theirs"]);
    h.git(&mate, &["push", "-q"]);
    h.write(root, "notes/first.md", &neuron("First", &ours_body()));
    h.write(root, "notes/topic.md", &neuron("Topic", BODY));
    h.git(root, &["add", "-A"]);
    h.git(root, &["commit", "-q", "-m", "ours"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let conflicts = doc["brains"][0]["sync"]["conflicts"].as_array().unwrap();
    let mut found: Vec<(&str, &str)> = conflicts
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["kind"].as_str().unwrap()))
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            ("notes/first.md", "content"),
            ("notes/topic.md", "structure")
        ],
        "{doc}"
    );
    let merge = conflicts[0]["merge"].as_str().unwrap().to_string();
    (merge, head(h, root), remote_main(h))
}

/// The files of `rev` under `notes/topic.md`, the file itself included (a
/// folder has the map the merge builds for it).
fn topic_files(h: &Home, root: &Path, rev: &str) -> Vec<String> {
    h.git(root, &["ls-tree", "-r", "--name-only", rev])
        .lines()
        .filter(|rel| rel.starts_with("notes/topic.md"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_written_text_file_finishes_a_merge_whose_structure_path_resolve_settled() {
    for (choice, topic) in [
        ("--ours", vec!["notes/topic.md"]),
        (
            "--theirs",
            vec!["notes/topic.md/child.md", "notes/topic.md/map.md"],
        ),
    ] {
        let (h, root) = team();
        let (merge, ours, theirs) = text_and_structure(&h, &root);
        let (code, doc) = resolve(&h, "notes/topic.md", &[choice], &merge);
        assert_eq!(code, 0, "{choice}: {doc}");
        assert_eq!(doc["remaining"], json!(["notes/first.md"]), "{doc}");
        h.write(&root, "notes/first.md", &settled());

        let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
        assert_eq!(code, 0, "{choice}: {doc}");
        assert_eq!(keys(&doc), COMMIT_KEYS, "{doc}");
        assert_eq!(doc["committed"], true, "{doc}");
        assert_eq!(doc["paths"], json!(["notes/first.md"]), "{doc}");
        let head = head(&h, &root);
        assert_eq!(doc["commit"], head.as_str());
        assert_eq!(parents(&h, &root, &head), [ours, theirs], "{choice}");
        assert_eq!(show(&h, &root, &head, "notes/first.md"), settled());
        assert_eq!(topic_files(&h, &root, &head), topic, "{choice}");
        assert_eq!(
            h.git(&root, &["log", "-1", "--format=%s", &head]).trim(),
            "kluris: sync (resolved)"
        );
        assert_eq!(h.read(&root, "notes/first.md"), settled());
        assert_eq!(status(&h, &root), "", "{choice}");
        assert!(!git_file(&root, "kluris-sync/pending.json").exists());
        let sync = sync_of(&doc);
        assert_eq!(sync["conflicts"], json!([]), "{doc}");
        assert_eq!(sync["pushed"], true, "{doc}");
        assert_eq!(remote_main(&h), head);
    }
}

#[test]
fn a_merge_whose_structure_path_has_no_choice_is_refused_and_nothing_changes() {
    let (h, root) = team();
    let (merge, ours, _) = text_and_structure(&h, &root);
    h.write(&root, "notes/first.md", &settled());
    let record = git_file(&root, "kluris-sync/pending.json");
    let pending = fs::read(&record).unwrap();
    for paths in ["notes/first.md", "notes/first.md,notes/topic.md"] {
        let (code, doc) = commit(&h, paths, &["--merge", &merge]);
        assert_eq!(code, 1, "{paths}: {doc}");
        let error = doc["error"].as_str().unwrap();
        assert!(
            error.contains("settle it with `kluris resolve notes/topic.md --ours|--theirs`"),
            "{error}"
        );
        assert!(error.contains("nothing was committed"), "{error}");
        assert_eq!(head(&h, &root), ours);
        assert_eq!(fs::read(&record).unwrap(), pending);
        assert_eq!(h.read(&root, "notes/first.md"), settled());
    }
}

#[test]
fn a_structure_path_resolve_settled_cannot_be_given_as_a_file() {
    let (h, root) = team();
    let (merge, ours, _) = text_and_structure(&h, &root);
    let (code, doc) = resolve(&h, "notes/topic.md", &["--ours"], &merge);
    assert_eq!(code, 0, "{doc}");
    h.write(&root, "notes/first.md", &settled());
    let record = git_file(&root, "kluris-sync/pending.json");
    let pending = fs::read(&record).unwrap();
    let (code, doc) = commit(&h, "notes/first.md,notes/topic.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("settle it with `kluris resolve notes/topic.md --ours|--theirs`"),
        "{error}"
    );
    assert_eq!(head(&h, &root), ours);
    assert_eq!(fs::read(&record).unwrap(), pending);
}

#[test]
fn with_every_path_recorded_paths_may_name_none() {
    let (h, root) = team();
    let (merge, ours, theirs) = text_and_structure(&h, &root);
    let (code, doc) = resolve(&h, "notes/topic.md", &["--theirs"], &merge);
    assert_eq!(code, 0, "{doc}");

    // A text path still without a choice must be given.
    let record = git_file(&root, "kluris-sync/pending.json");
    let pending = fs::read(&record).unwrap();
    let (code, doc) = commit(&h, "", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("settle these too: notes/first.md"),
        "{error}"
    );
    assert_eq!(head(&h, &root), ours);
    assert_eq!(fs::read(&record).unwrap(), pending);

    // The last choice cannot land past an uncommitted edit: every path has
    // a choice and the merge still waits.
    let merged = h.dir.path().join("merged.md");
    fs::write(&merged, settled()).unwrap();
    h.write(&root, "notes/first.md", &neuron("First", "A draft.\n"));
    let file = merged.display().to_string();
    let (code, doc) = resolve(&h, "notes/first.md", &["--file", &file], &merge);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(head(&h, &root), ours);
    h.git(&root, &["checkout", "--", "notes/first.md"]);

    let (code, doc) = commit(&h, "", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!([]), "{doc}");
    let head = head(&h, &root);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    assert_eq!(show(&h, &root, &head, "notes/first.md"), settled());
    assert_eq!(
        topic_files(&h, &root, &head),
        ["notes/topic.md/child.md", "notes/topic.md/map.md"]
    );
    assert_eq!(h.read(&root, "notes/first.md"), settled());
    assert_eq!(status(&h, &root), "");
    assert!(!record.exists());
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
    assert_eq!(remote_main(&h), head);
}

/// The settled text of `notes/second.md`.
fn second_settled() -> String {
    neuron("Second", &BODY.replace("Three.", "Three, settled."))
}

#[test]
fn a_recorded_text_choice_holds_for_a_path_not_given() {
    let (h, root) = team();
    let merge = two_waiting(&h, &root);
    let (code, doc) = resolve(&h, "notes/first.md", &["--theirs"], &merge);
    assert_eq!(code, 0, "{doc}");
    let second = second_settled();
    h.write(&root, "notes/second.md", &second);
    let (code, doc) = commit(&h, "notes/second.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let head_rev = head(&h, &root);
    assert_eq!(
        show(&h, &root, &head_rev, "notes/first.md"),
        neuron("First", &theirs_body())
    );
    assert_eq!(show(&h, &root, &head_rev, "notes/second.md"), second);
    assert_eq!(status(&h, &root), "");
    assert_eq!(remote_main(&h), head_rev);
}

#[test]
fn the_file_of_a_given_path_replaces_the_choice_recorded_for_it() {
    let (h, root) = team();
    let merge = two_waiting(&h, &root);
    let second = second_settled();
    let merged = h.dir.path().join("merged.md");
    fs::write(&merged, neuron("First", "Recorded.\n")).unwrap();
    let file = merged.display().to_string();
    let (code, doc) = resolve(&h, "notes/first.md", &["--file", &file], &merge);
    assert_eq!(code, 0, "{doc}");
    h.write(&root, "notes/first.md", &settled());
    h.write(&root, "notes/second.md", &second);
    let (code, doc) = commit(&h, "notes/first.md,notes/second.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let head_rev = head(&h, &root);
    assert_eq!(show(&h, &root, &head_rev, "notes/first.md"), settled());
    assert_eq!(show(&h, &root, &head_rev, "notes/second.md"), second);
    assert_eq!(status(&h, &root), "");
    assert_eq!(remote_main(&h), head_rev);
}

#[test]
fn a_conflicted_file_gone_from_disk_is_a_deletion() {
    let (h, root) = team();
    let (merge, ours, theirs) = waiting(&h, &root);
    fs::remove_file(root.join("notes").join("first.md")).unwrap();
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let head = head(&h, &root);
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    let files = h.git(&root, &["ls-tree", "-r", "--name-only", &head]);
    assert!(!files.lines().any(|l| l == "notes/first.md"), "{files}");
    // The merge's maps are rebuilt from the merged files.
    let map = show(&h, &root, &head, "notes/map.md");
    assert!(!map.contains("first.md"), "{map}");
    assert_eq!(status(&h, &root), "");
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
}

#[test]
fn the_merge_commit_has_both_parents_its_message_and_every_trailer() {
    for message in [None, Some("Settle the first note")] {
        let (mut h, root) = team();
        h.env
            .push(("KLURIS_AGENT".into(), "Claude <c@example.com>".into()));
        let (merge, ours, theirs) = waiting(&h, &root);
        h.write(&root, "notes/first.md", &settled());
        let mut extra = vec!["--merge", merge.as_str()];
        if let Some(message) = message {
            extra.extend(["-m", message]);
        }
        let (code, doc) = commit(&h, "notes/first.md", &extra);
        assert_eq!(code, 0, "{doc}");
        let head = head(&h, &root);
        assert_eq!(parents(&h, &root, &head), [ours, theirs]);
        let subject = message.unwrap_or("kluris: sync (resolved)");
        assert_eq!(
            h.git(&root, &["log", "-1", "--format=%B", &head]),
            format!(
                "{subject}\n\nKluris-Sync: resolved\nKluris-Resolved-By: Ana Tester\nKluris-Approved-By: Ana Tester\nCo-Authored-By: Claude <c@example.com>\n\n"
            )
        );
    }
}

#[test]
fn the_text_of_a_finished_merge_names_its_subject_and_paths() {
    let (h, root) = team();
    let (merge, _, _) = waiting(&h, &root);
    h.write(&root, "notes/first.md", &settled());
    let (code, out, err) = h.run(&[
        "commit",
        "--brain",
        "alpha",
        "--paths",
        "notes/first.md",
        "--merge",
        &merge,
    ]);
    assert_eq!(code, 0, "{err}");
    let short = &head(&h, &root)[..7];
    assert!(
        out.starts_with(&format!(
            "Committed {short} in alpha: kluris: sync (resolved)\n  notes/first.md\n"
        )),
        "{out}"
    );
    assert_eq!(err, "kluris: alpha: pushed\n");
}

#[test]
fn a_map_dream_rewrote_keeps_its_bytes_and_the_merge_lands() {
    let (h, root) = team();
    h.conflict("alpha", &root);
    // The person's side dated as its `updated` says: dream has nothing of
    // HEAD's to commit, and only rewrites the map from the work tree.
    h.git(
        &root,
        &[
            "commit",
            "-q",
            "--amend",
            "--no-edit",
            "--date=2026-01-01T12:00:00",
        ],
    );
    let (merge, ours, theirs) = merge_of(&h, &root);
    // The settled file retitles the neuron, and an uncommitted draft is
    // there too: dream's map lists both, the merge's only the first.
    h.write(
        &root,
        "notes/first.md",
        &neuron("First, settled", &BODY.replace("Three.", "Three, settled.")),
    );
    h.write(&root, "notes/draft.md", &neuron("Draft", BODY));
    let (_, out, err) = h.run(&["dream", "--brain", "alpha", "--json"]);
    assert_eq!(head(&h, &root), ours, "{out}{err}");
    let dreamt = h.read(&root, "notes/map.md");
    assert!(
        dreamt.contains("draft.md") && dreamt.contains("First, settled"),
        "{dreamt}"
    );

    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let head = head(&h, &root);
    assert_eq!(parents(&h, &root, &head), [ours, theirs]);
    let committed = show(&h, &root, &head, "notes/map.md");
    assert!(committed.contains("First, settled"), "{committed}");
    assert!(!committed.contains("draft.md"), "{committed}");
    assert_eq!(h.read(&root, "notes/map.md"), dreamt);
    assert_eq!(
        doc["left_dirty"],
        json!(["notes/draft.md", "notes/map.md"]),
        "{doc}"
    );
    assert!(!git_file(&root, "kluris-sync/pending.json").exists());
    assert_eq!(sync_of(&doc)["pushed"], true, "{doc}");
}

#[test]
fn a_neuron_in_the_way_of_the_merge_is_refused_and_says_what_the_sync_did() {
    let (h, root) = team();
    h.commit(&root, "notes/second.md", &neuron("Second", BODY));
    h.git(&root, &["push", "-q"]);
    let mate = h.teammate("alpha");
    h.git(&mate, &["pull", "-q"]);
    h.write(
        &mate,
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, theirs.")),
    );
    h.write(
        &mate,
        "notes/second.md",
        &neuron("Second", &BODY.replace("One.", "One, theirs.")),
    );
    h.git(&mate, &["commit", "-q", "-am", "theirs"]);
    h.git(&mate, &["push", "-q"]);
    h.commit(
        &root,
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, ours.")),
    );
    let (merge, ours, _) = merge_of(&h, &root);
    // The teammate's clean change to second.md would land on the person's
    // own edit of it.
    let mine = neuron("Second", &BODY.replace("Five.", "Five, mine."));
    h.write(&root, "notes/second.md", &mine);
    h.write(&root, "notes/first.md", &settled());
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(error.contains("notes/second.md"), "{error}");
    assert!(error.contains("nothing was committed"), "{error}");
    let sync = sync_of(&doc);
    assert_eq!(sync["conflicts"][0]["merge"], merge.as_str(), "{doc}");
    assert_eq!(head(&h, &root), ours);
    assert_eq!(h.read(&root, "notes/second.md"), mine);
    assert_eq!(h.read(&root, "notes/first.md"), settled());
    // The index is as it was: nothing staged.
    assert_eq!(h.git(&root, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn something_newer_is_pulled_after_the_merge_then_pushed() {
    let (h, root) = team();
    let (merge, ours, theirs) = waiting(&h, &root);
    h.teammate_pushes("alpha", "notes/zebra.md", &zebra());
    let newer = remote_main(&h);
    h.write(&root, "notes/first.md", &settled());
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    let finished = doc["commit"].as_str().unwrap().to_string();
    assert_eq!(parents(&h, &root, &finished), [ours, theirs]);
    let head = head(&h, &root);
    assert_eq!(parents(&h, &root, &head), [finished, newer]);
    let sync = sync_of(&doc);
    assert_eq!(sync["conflicts"], json!([]), "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    assert_eq!(remote_main(&h), head);
    assert!(root.join("notes").join("zebra.md").is_file());
    assert_eq!(h.read(&root, "notes/first.md"), settled());
    assert_eq!(status(&h, &root), "");
}

#[test]
fn a_new_conflict_after_the_merge_is_a_new_record_and_exits_0() {
    let (h, root) = team();
    let (merge, _, _) = waiting(&h, &root);
    h.teammate_pushes(
        "alpha",
        "notes/first.md",
        &neuron("First", &BODY.replace("Three.", "Three, theirs again.")),
    );
    h.write(&root, "notes/first.md", &settled());
    let (code, doc) = commit(&h, "notes/first.md", &["--merge", &merge]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true);
    let finished = doc["commit"].as_str().unwrap();
    assert_eq!(head(&h, &root), finished);
    let sync = sync_of(&doc);
    let conflicts = sync["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{doc}");
    assert_ne!(conflicts[0]["merge"], merge.as_str());
    assert!(
        conflicts[0]["theirs"]
            .as_str()
            .unwrap()
            .contains("Three, theirs again."),
        "{doc}"
    );
    assert_eq!(sync["pushed"], false, "{doc}");
    let record = fs::read_to_string(git_file(&root, "kluris-sync/pending.json")).unwrap();
    assert!(record.contains(finished), "{record}");
}

// --- hooks -----------------------------------------------------------------------------

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

/// The hooks that write each run, with its arguments, to `<home>/hooks.log`
/// (a message file as `file`): every hook a commit, a merge or a push runs.
fn logging_hooks(h: &Home, root: &Path) -> PathBuf {
    let log = h.dir.path().join("hooks.log");
    let hooks = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let log_s = sh_path(&log);
    let file = "$( [ -f \"$1\" ] && echo file )";
    for (name, args) in [
        ("pre-commit", "$#".to_string()),
        ("pre-merge-commit", "$#".to_string()),
        ("prepare-commit-msg", format!("$# {file} $2")),
        ("commit-msg", format!("$# {file}")),
        ("post-commit", "$#".to_string()),
        ("post-merge", "$# $1".to_string()),
        ("pre-push", "$# $1".to_string()),
    ] {
        let hook = hooks.join(name);
        fs::write(
            &hook,
            format!("#!/bin/sh\necho \"{name} {args}\" >> '{log_s}'\nexit 0\n"),
        )
        .unwrap();
        make_executable(&hook);
    }
    log
}

fn logged(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn a_commit_runs_each_hook_with_the_arguments_git_gives_it() {
    let (h, root) = team();
    h.teammate_pushes("alpha", "notes/zebra.md", &zebra());
    let log = logging_hooks(&h, &root);
    h.write(&root, "notes/second.md", &neuron("Second", BODY));
    let (code, doc) = commit(&h, "notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        logged(&log),
        [
            // The open's pull lands the teammate's commit.
            "post-merge 1 0",
            "pre-commit 0",
            "prepare-commit-msg 2 file message",
            "commit-msg 1 file",
            "post-commit 0",
            "pre-push 2 origin",
        ]
    );

    fs::remove_file(&log).unwrap();
    h.write(&root, "notes/third.md", &neuron("Third", BODY));
    let (code, doc) = commit(&h, "notes/third.md", &["--no-verify"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        logged(&log),
        [
            "prepare-commit-msg 2 file message",
            "post-commit 0",
            "pre-push 2 origin",
        ]
    );
}

#[test]
fn finishing_a_merge_runs_the_hooks_git_commit_runs_for_a_merge() {
    for no_verify in [false, true] {
        let (h, root) = team();
        let (merge, _, _) = waiting(&h, &root);
        let log = logging_hooks(&h, &root);
        h.write(&root, "notes/first.md", &settled());
        let mut extra = vec!["--merge", merge.as_str()];
        let mut expected = vec![
            "pre-commit 0",
            "prepare-commit-msg 2 file merge",
            "commit-msg 1 file",
            // The land.
            "post-merge 1 0",
            "post-commit 0",
            "pre-push 2 origin",
        ];
        if no_verify {
            extra.push("--no-verify");
            expected.retain(|run| !run.starts_with("pre-commit") && !run.starts_with("commit-msg"));
        }
        let (code, doc) = commit(&h, "notes/first.md", &extra);
        assert_eq!(code, 0, "{doc}");
        assert_eq!(logged(&log), expected, "--no-verify {no_verify}");
    }
}
