//! `dream` and `mri` sync: each brings a teammate's commits in first, does
//! its own work in the work tree, then commits the brain's own
//! housekeeping, built from HEAD's files and never the work tree: dates
//! from git, a missing `parent`, reverse `related` links, every `map.md`
//! and `brain.md`, as one `kluris: dream` commit (`Kluris-Sync: dream`)
//! through the commit hooks. The fixes an uncommitted file causes stay in
//! the work tree. A brain the session leaves alone (no git, no remote)
//! behaves as it always has. Neither command's exit code is its sync's.
//!
//! Also here: the stderr line every command prints says what the last
//! background push did.
//!
//! Real git, the built binary under a sandbox HOME and `KLURIS_CONFIG`,
//! and local bare remotes.

mod sync_home;

use std::fs;
#[cfg(unix)]
use std::net::TcpListener;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kluris_core::sync::background::{self, LAST_PUSH, LastPush, PushResult};
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

/// `dream --json`'s keys when it has nothing to commit or say.
const DREAM_KEYS: [&str; 11] = [
    "ok",
    "healthy",
    "broken_synapses",
    "one_way_synapses",
    "orphans",
    "frontmatter_issues",
    "dates_updated",
    "deprecation_issues",
    "deprecation",
    "broken_synapses_detail",
    "fixes",
];

/// An `mri --json` entry's keys when it has nothing to commit or say.
const MRI_KEYS: [&str; 5] = ["name", "output_path", "preflight_fixes", "nodes", "edges"];

/// The housekeeping commit's message.
const MESSAGE: &str = "kluris: dream\n\nKluris-Sync: dream";

/// The hooks a commit runs, in the order git runs them.
const COMMIT_HOOKS: [&str; 4] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

/// The longest a test waits for a process to do something.
#[cfg(unix)]
const PATIENCE: Duration = Duration::from_secs(60);

/// Polls `done` until it holds, at most [`PATIENCE`].
#[cfg(unix)]
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < PATIENCE,
            "{what}: not within {PATIENCE:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// A path as git's shell takes it (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A file under the brain's git directory.
fn git_file(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.join(".git"), |path, part| path.join(part))
}

/// A hook of the brain's: `body` after a shebang.
fn hook(root: &Path, name: &str, body: &str) {
    let path = git_file(root, &format!("hooks/{name}"));
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The brain's fetch stamp, `age` seconds old.
fn stamp_age(root: &Path, age: u64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let stamp = git_file(root, "kluris-sync/fetch-stamp");
    fs::create_dir_all(stamp.parent().unwrap()).unwrap();
    fs::write(stamp, format!("{}\n", now - age)).unwrap();
}

/// A home with the brain `alpha`, pushed to its remote, and its teammate.
fn team() -> (Home, PathBuf) {
    let mut h = Home::new();
    let root = h.brain("alpha");
    (h, root)
}

/// A neuron in a new lobe with every housekeeping fix to make: no parent,
/// no dates, and a `related` link `notes/first.md` does not return.
const SECOND: &str = "---\nrelated:\n- ../notes/first.md\n---\n\n# Second\n\nAbout the second.\n";

/// The teammate pushes `ideas/second.md` ([`SECOND`]).
fn teammate_pushes_second(h: &Home) {
    h.teammate_pushes("alpha", "ideas/second.md", SECOND);
}

/// What HEAD's housekeeping commit after the teammate's `ideas/second.md`
/// holds: the new neuron's parent and dates, the reverse link and the
/// date of `notes/first.md`, the new lobe's map, the map beside it, and
/// `brain.md`.
const SECOND_FILES: [&str; 5] = [
    "brain.md",
    "ideas/map.md",
    "ideas/second.md",
    "notes/first.md",
    "notes/map.md",
];

impl Home {
    fn head(&self, root: &Path) -> String {
        self.git(root, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The last commit's message, without git's final newline.
    fn message(&self, root: &Path) -> String {
        self.git(root, &["log", "-1", "--format=%B"])
            .trim_end()
            .to_string()
    }

    /// The files HEAD changes against its first parent, sorted.
    fn changed(&self, root: &Path) -> Vec<String> {
        let out = self.git(root, &["diff", "--name-only", "HEAD^", "HEAD"]);
        let mut files: Vec<String> = out.lines().map(str::to_string).collect();
        files.sort();
        files
    }

    /// `rel` as HEAD has it.
    fn committed(&self, root: &Path, rel: &str) -> String {
        self.git(root, &["show", &format!("HEAD:{rel}")])
    }

    fn status(&self, root: &Path) -> String {
        self.git(root, &["status", "--porcelain"])
    }

    /// The remote's `main`.
    fn remote_head(&self, name: &str) -> String {
        let bare = self.dir.path().join(format!("{name}.git"));
        self.git(&bare, &["rev-parse", "main"]).trim().to_string()
    }

    /// `kluris <args> --json`: the exit code, the document and stderr.
    fn json_run(&self, args: &[&str]) -> (i32, Value, String) {
        let mut args = args.to_vec();
        args.push("--json");
        let (code, out, err) = self.run(&args);
        let doc =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}: {out}{err}"));
        (code, doc, err)
    }
}

/// The `sync` object, checked to have its eight keys (and `brain` first
/// when it is one of `dream`'s items).
fn sync_of(item: &Value) -> &Value {
    let mut expected: Vec<&str> = SYNC_KEYS.to_vec();
    if item.get("brain").is_some() {
        expected.insert(0, "brain");
    }
    assert_eq!(keys(item), expected, "{item}");
    item
}

/// `dream --json`'s `sync` item of `brain`.
fn dream_sync<'d>(doc: &'d Value, brain: &str) -> &'d Value {
    let items = doc["sync"]
        .as_array()
        .unwrap_or_else(|| panic!("no sync array: {doc}"));
    let item = items
        .iter()
        .find(|i| i["brain"] == brain)
        .unwrap_or_else(|| panic!("no sync item for {brain}: {doc}"));
    sync_of(item)
}

/// `dream --json`'s `committed` files of `brain`, in order.
fn dream_committed(doc: &Value, brain: &str) -> Vec<String> {
    doc["committed"]
        .as_array()
        .unwrap_or_else(|| panic!("no committed array: {doc}"))
        .iter()
        .inspect(|item| assert_eq!(keys(item), ["brain", "file"], "{item}"))
        .filter(|item| item["brain"] == brain)
        .map(|item| item["file"].as_str().unwrap().to_string())
        .collect()
}

/// Waits for the background push the command just started when it left
/// commits waiting, so the next command finds the remote free.
fn settle(sync: &Value, root: &Path) {
    if sync["to_push"].as_u64().is_some_and(|n| n > 0) {
        wait_for_background(root);
    }
}

// --- the pull first ------------------------------------------------------------

#[test]
fn dream_and_mri_pull_a_teammates_commit_first() {
    for command in ["dream", "mri"] {
        let (h, root) = team();
        h.teammate_pushes("alpha", "notes/zebra.md", &neuron("Zebra", "Stripes.\n"));
        let (code, doc, err) = h.json_run(&[command, "--brain", "alpha"]);
        assert_eq!(code, 0, "{command}: {doc}{err}");
        let sync = match command {
            "dream" => dream_sync(&doc, "alpha"),
            _ => sync_of(&doc["brains"][0]["sync"]),
        };
        assert_eq!(sync["pulled"], 1, "{command}: {doc}");
        assert!(root.join("notes").join("zebra.md").is_file(), "{command}");
        // The command's own work saw what was pulled.
        assert!(
            h.read(&root, "notes/map.md").contains("zebra.md"),
            "{command}: the map was made before the pull"
        );
        settle(sync, &root);
    }
}

// --- the housekeeping commit ---------------------------------------------------

#[test]
fn dream_commits_what_its_steps_change_over_head_through_the_hooks() {
    let (h, root) = team();
    let log = h.dir.path().join("hooks.log");
    for name in COMMIT_HOOKS {
        hook(
            &root,
            name,
            &format!("echo {name} >> '{}'\n", sh_path(&log)),
        );
    }
    teammate_pushes_second(&h);
    let theirs = h.head(&h.teammate("alpha"));
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");

    // One commit on the teammate's, as the person, with the trailer.
    assert_eq!(h.message(&root), MESSAGE);
    assert_eq!(h.git(&root, &["rev-parse", "HEAD^"]).trim(), theirs);
    assert_eq!(
        h.git(&root, &["log", "-1", "--format=%an <%ae>|%P"])
            .trim()
            .split('|')
            .collect::<Vec<_>>(),
        ["Ana Tester <ana@example.com>", theirs.as_str()]
    );
    assert_eq!(h.changed(&root), SECOND_FILES);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "pre-commit\nprepare-commit-msg\ncommit-msg\npost-commit\n"
    );

    // Each step, as HEAD has it now.
    let day = h.git(&root, &["log", "-1", "--format=%aI", &theirs])[..10].to_string();
    let second = h.committed(&root, "ideas/second.md");
    assert!(second.contains("parent: ./map.md\n"), "{second}");
    assert!(second.contains(&format!("updated: '{day}'\n")), "{second}");
    assert!(second.contains(&format!("created: '{day}'\n")), "{second}");
    let first = h.committed(&root, "notes/first.md");
    assert!(first.contains("- ../ideas/second.md\n"), "{first}");
    assert!(!first.contains("updated: '2026-01-01'"), "{first}");
    assert!(h.committed(&root, "ideas/map.md").contains("second.md"));
    assert!(h.committed(&root, "brain.md").contains("ideas"));

    // The work tree holds what was committed.
    assert_eq!(h.status(&root), "");

    // JSON: `committed`, then `sync`, after dream's own keys.
    let mut expected: Vec<&str> = DREAM_KEYS.to_vec();
    expected.extend(["committed", "sync"]);
    assert_eq!(keys(&doc), expected, "{doc}");
    assert_eq!(dream_committed(&doc, "alpha"), SECOND_FILES);
    let sync = dream_sync(&doc, "alpha");
    assert_eq!(
        sync,
        &json!({
            "brain": "alpha", "pulled": 1, "auto_merged": [],
            "left_as_is": {"reason": null, "paths": [], "detail": null},
            "conflicts": [], "behind_conflict": 0, "to_push": 1, "pushed": false,
            "last_push": null,
        })
    );
    settle(sync, &root);

    // Nothing left to commit: no commit, no `committed`.
    let before = h.head(&root);
    stamp_age(&root, 0);
    h.run(&["status", "--brain", "alpha", "--json"]);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.head(&root), before);
    // Only the commit waiting may still be said (on Windows the runner's
    // job may keep the background push from starting).
    let mut quiet: Vec<String> = keys(&doc);
    quiet.retain(|key| key != "sync");
    assert_eq!(quiet, DREAM_KEYS, "{doc}");
}

#[test]
fn text_says_what_was_committed_on_stdout_and_what_the_sync_did_on_stderr() {
    for command in ["dream", "mri"] {
        let (h, root) = team();
        teammate_pushes_second(&h);
        let (code, out, err) = h.run(&[command, "--brain", "alpha"]);
        assert_eq!(code, 0, "{command}: {out}{err}");
        assert!(
            out.contains("\nCommitted 5 housekeeping files in alpha\n"),
            "{command}: {out}"
        );
        assert_eq!(
            err, "kluris: alpha: pulled 1 commit; 1 commit to push\n",
            "{command}"
        );
        assert_eq!(h.message(&root), MESSAGE, "{command}");
        wait_for_background(&root);
    }
}

#[test]
fn mri_entries_carry_their_committed_files_and_sync() {
    let (h, root) = team();
    teammate_pushes_second(&h);
    let (code, doc, err) = h.json_run(&["mri", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    let entry = &doc["brains"][0];
    let mut expected: Vec<&str> = MRI_KEYS.to_vec();
    expected.extend(["committed", "sync"]);
    assert_eq!(keys(entry), expected, "{doc}");
    assert_eq!(entry["committed"], json!(SECOND_FILES));
    assert_eq!(h.changed(&root), SECOND_FILES);
    assert_eq!(h.message(&root), MESSAGE);
    let sync = sync_of(&entry["sync"]);
    assert_eq!((&sync["pulled"], &sync["to_push"]), (&json!(1), &json!(1)));
    // The page stays out of the commit.
    assert!(!h.changed(&root).iter().any(|f| f.contains("mri")));
    settle(sync, &root);
}

#[test]
fn dream_names_each_brain_in_its_committed_and_sync_items() {
    let (mut h, root) = team();
    // A brain outside git beside it: nothing to commit or say.
    let local = h.dir.path().join("local");
    h.write(&local, "kluris.yml", "name: local\ndescription: d\n");
    h.write(
        &local,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
    );
    h.write(&local, "notes/one.md", &neuron("One", BODY));
    h.register("local", &local);
    teammate_pushes_second(&h);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "all"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(dream_committed(&doc, "alpha"), SECOND_FILES);
    assert_eq!(dream_committed(&doc, "local"), Vec::<String>::new());
    let items = doc["sync"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{doc}");
    let sync = dream_sync(&doc, "alpha");
    settle(sync, &root);
}

// --- what stays in the work tree -------------------------------------------------

#[test]
fn fixes_an_uncommitted_neuron_causes_stay_in_the_work_tree() {
    let (h, root) = team();
    // An untracked neuron that links to `first.md`, and an uncommitted new
    // title of a tracked one.
    h.write(
        &root,
        "notes/draft.md",
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n- ./first.md\nupdated: '2026-01-01'\n---\n\n# Secret Draft\n\nNot yet.\n",
    );
    h.write(&root, "ideas/second.md", &neuron("Shown Title", BODY));
    h.git(&root, &["add", "ideas/second.md"]);
    h.git(&root, &["commit", "-q", "-m", "second"]);
    h.write(&root, "ideas/second.md", &neuron("Unapproved Title", BODY));
    let base = h.head(&root);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.message(&root), MESSAGE);
    assert_eq!(h.git(&root, &["rev-parse", "HEAD^"]).trim(), base);

    // Neither file's text reached the commit.
    let committed: Vec<String> = h
        .git(&root, &["show", "--format=", "--patch", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    let patch = committed.join("\n");
    for secret in ["Secret Draft", "draft.md", "Unapproved Title"] {
        assert!(!patch.contains(secret), "{secret} was committed:\n{patch}");
    }
    assert!(!h.committed(&root, "notes/map.md").contains("Secret Draft"));
    assert!(
        !h.committed(&root, "ideas/map.md")
            .contains("Unapproved Title")
    );
    assert!(h.committed(&root, "ideas/map.md").contains("Shown Title"));
    assert!(!h.committed(&root, "notes/first.md").contains("draft.md"));
    assert!(!dream_committed(&doc, "alpha").contains(&"notes/draft.md".to_string()));

    // The work tree keeps every fix dream made for them.
    assert!(h.read(&root, "notes/map.md").contains("Secret Draft"));
    assert!(h.read(&root, "ideas/map.md").contains("Unapproved Title"));
    assert!(h.read(&root, "notes/first.md").contains("draft.md"));
    let mut status: Vec<String> = h.status(&root).lines().map(str::to_string).collect();
    status.sort();
    assert_eq!(
        status,
        [
            " M ideas/map.md",
            " M ideas/second.md",
            " M notes/first.md",
            " M notes/map.md",
            "?? notes/draft.md",
        ],
        "{doc}"
    );
    settle(dream_sync(&doc, "alpha"), &root);
}

// --- what leaves the commit undone ------------------------------------------------

#[test]
fn a_gate_hook_that_stops_the_commit_leaves_the_fixes_in_the_work_tree() {
    for name in ["pre-commit", "prepare-commit-msg", "commit-msg"] {
        let (h, root) = team();
        hook(&root, name, "echo 'no housekeeping today' >&2\nexit 1\n");
        teammate_pushes_second(&h);
        let theirs = h.head(&h.teammate("alpha"));
        let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
        assert_eq!(code, 0, "{name}: {doc}{err}");
        assert!(err.contains("no housekeeping today"), "{name}: {err}");
        assert_eq!(h.head(&root), theirs, "{name}: committed");
        assert_eq!(keys(&doc).last().map(String::as_str), Some("sync"), "{doc}");
        assert!(doc.get("committed").is_none(), "{name}: {doc}");
        let sync = dream_sync(&doc, "alpha");
        assert_eq!(sync["pulled"], 1, "{name}");
        assert_eq!(sync["left_as_is"]["reason"], "hook", "{name}: {doc}");
        let detail = sync["left_as_is"]["detail"].as_str().unwrap();
        assert!(detail.contains(name), "{name}: {detail}");
        // The fixes, uncommitted.
        assert!(
            h.read(&root, "ideas/second.md")
                .contains("parent: ./map.md")
        );
        assert!(
            h.read(&root, "notes/first.md")
                .contains("- ../ideas/second.md")
        );
        assert!(root.join("ideas").join("map.md").is_file(), "{name}");

        // mri too, with its own exit code.
        let (code, doc, _) = h.json_run(&["mri", "--brain", "alpha"]);
        assert_eq!(code, 0, "{name}: {doc}");
        assert_eq!(h.head(&root), theirs, "{name}: mri committed");
        let sync = sync_of(&doc["brains"][0]["sync"]);
        assert_eq!(sync["left_as_is"]["reason"], "hook", "{name}: {doc}");
    }
}

#[test]
fn a_tree_claim_held_past_the_bound_leaves_the_commit_undone() {
    let (h, root) = team();
    // Another reader holds the work tree: the read goes on beside it, the
    // commit cannot.
    let held = TreeClaim::shared(&root.join(".git"), Duration::ZERO).unwrap();
    let before = h.head(&root);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    drop(held);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.head(&root), before);
    assert!(doc.get("committed").is_none(), "{doc}");
    let sync = dream_sync(&doc, "alpha");
    assert_eq!(sync["left_as_is"]["reason"], "busy", "{doc}");
    // The date fix, uncommitted.
    assert_eq!(h.status(&root), " M notes/first.md\n");
}

#[test]
fn a_pull_left_as_it_is_keeps_its_reason_over_the_commits() {
    let (h, root) = team();
    hook(&root, "pre-commit", "exit 1\n");
    let gone = h.dir.path().join("gone.git").display().to_string();
    h.git(&root, &["remote", "set-url", "origin", &gone]);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(dream_sync(&doc, "alpha")["left_as_is"]["reason"], "refused");
    assert_eq!(h.status(&root), " M notes/first.md\n");
}

// --- the commit meets teammates --------------------------------------------------

#[test]
fn two_clones_dreaming_on_one_history_commit_the_same_blobs_and_merge_cleanly() {
    let (mut h, root) = team();
    let mate = h.teammate("alpha");
    h.write(&mate, "kluris.yml", "name: alpha\ndescription: d\n");
    h.register("mate", &mate);
    // A third person pushes the neuron both clones tidy.
    let carol = h.dir.path().join("carol");
    let bare = h.dir.path().join("alpha.git").display().to_string();
    h.git(
        h.dir.path(),
        &["clone", "-q", &bare, &carol.display().to_string()],
    );
    h.configure(&carol, "Carol Third", "carol@example.com");
    h.commit(&carol, "ideas/second.md", SECOND);
    h.git(&carol, &["push", "-q"]);

    // alpha's push is turned back until both have dreamed.
    hook(&root, "pre-push", "exit 1\n");
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    settle(dream_sync(&doc, "alpha"), &root);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "mate"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(dream_sync(&doc, "mate")["pulled"], 1, "{doc}");
    settle(dream_sync(&doc, "mate"), &mate);
    // The mate's commit reaches the remote, whether or not its background
    // push could start.
    let (code, doc, err) = h.json_run(&["sync", "--brain", "mate"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.message(&root), MESSAGE);
    assert_eq!(h.message(&mate), MESSAGE);
    assert_ne!(h.head(&root), h.head(&mate));
    for rel in [
        "ideas/second.md",
        "notes/first.md",
        "ideas/map.md",
        "brain.md",
    ] {
        let blob = |at: &Path| h.git(at, &["rev-parse", &format!("HEAD:{rel}")]);
        assert_eq!(blob(&root), blob(&mate), "{rel}");
    }

    // alpha syncs: the two commits merge with no conflict.
    fs::remove_file(git_file(&root, "hooks/pre-push")).unwrap();
    let (code, doc, err) = h.json_run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    let sync = sync_of(&doc["brains"][0]["sync"]);
    assert_eq!(sync["pulled"], 1, "{doc}");
    assert_eq!(sync["conflicts"], json!([]), "{doc}");
    assert_eq!(sync["pushed"], true, "{doc}");
    let parents = h.git(&root, &["log", "-1", "--format=%P"]);
    assert_eq!(parents.split_whitespace().count(), 2, "{parents}");
    assert_eq!(h.remote_head("alpha"), h.head(&root));
}

#[test]
fn while_a_merge_waits_nothing_is_committed_and_the_id_dream_says_finishes_it() {
    let (h, root) = team();
    h.conflict("alpha", &root);
    // Our side also has a neuron to tidy outside the conflict.
    h.commit(
        &root,
        "notes/third.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Third\n\nThe third.\n",
    );
    let ours = h.head(&root);
    // A read says the conflict and the waiting merge's id.
    let (code, read, err) = h.json_run(&["status", "--brain", "alpha"]);
    assert_eq!(code, 0, "{read}{err}");
    let said = &read["brains"][0]["sync"];
    let merge = said["conflicts"][0]["merge"].as_str().unwrap().to_string();
    settle(said, &root);

    // The person settles the file, retitling the neuron, and dream runs:
    // no commit, the fixes and the rebuilt map stay in the work tree, and
    // the conflict is said with the same id.
    h.write(
        &root,
        "notes/first.md",
        &neuron("First, settled", &BODY.replace("Three.", "Three, settled.")),
    );
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.head(&root), ours);
    assert!(doc.get("committed").is_none(), "{doc}");
    let sync = dream_sync(&doc, "alpha");
    assert_eq!(sync["left_as_is"]["reason"], Value::Null, "{doc}");
    let conflicts = sync["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{doc}");
    assert_eq!(conflicts[0]["path"], "notes/first.md");
    assert_eq!(conflicts[0]["merge"], merge.as_str(), "{doc}");
    let dreamt = h.read(&root, "notes/map.md");
    assert!(dreamt.contains("First, settled"), "{dreamt}");
    assert!(h.read(&root, "notes/third.md").contains("parent: ./map.md"));
    assert!(git_file(&root, "kluris-sync/pending.json").is_file());
    settle(sync, &root);

    // That id finishes the merge, the map dream rewrote kept as it is.
    let (code, done, err) = h.json_run(&[
        "commit",
        "--brain",
        "alpha",
        "--merge",
        &merge,
        "--paths",
        "notes/first.md",
    ]);
    assert_eq!(code, 0, "{done}{err}");
    let parents = h.git(&root, &["log", "-1", "--format=%P"]);
    assert_eq!(parents.split_whitespace().count(), 2, "{parents}");
    assert_eq!(parents.split_whitespace().next(), Some(ours.as_str()));
    assert!(
        h.committed(&root, "notes/map.md")
            .contains("First, settled")
    );
    assert_eq!(h.read(&root, "notes/map.md"), dreamt);
    // The map dream rebuilt is the merge's own; the neuron it tidied stays
    // changed in the work tree.
    assert_eq!(done["left_dirty"], json!(["notes/third.md"]), "{done}");
    assert!(!git_file(&root, "kluris-sync/pending.json").exists());
    assert!(!h.committed(&root, "notes/third.md").contains("parent:"));
    let merged = h.head(&root);

    // The next dream commits the housekeeping on the merge.
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    assert_eq!(h.message(&root), MESSAGE, "{doc}{err}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD^"]).trim(), merged);
    assert_eq!(dream_committed(&doc, "alpha"), ["notes/third.md"], "{doc}");
    assert!(
        h.committed(&root, "notes/third.md")
            .contains("parent: ./map.md")
    );
    assert_eq!(h.status(&root), "");
    settle(dream_sync(&doc, "alpha"), &root);
}

#[cfg(unix)]
#[test]
fn offline_the_commit_lands_here_and_a_background_push_sends_it_later() {
    let (h, root) = team();
    teammate_pushes_second(&h);
    h.run(&["status", "--brain", "alpha", "--json"]);
    // A port nothing listens on.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let origin = h.dir.path().join("alpha.git").display().to_string();
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "origin",
            &format!("git://127.0.0.1:{closed}/alpha.git"),
        ],
    );
    stamp_age(&root, 61);
    let (code, doc, err) = h.json_run(&["dream", "--brain", "alpha"]);
    assert_eq!(code, 0, "{doc}{err}");
    let sync = dream_sync(&doc, "alpha");
    assert_eq!(sync["left_as_is"]["reason"], "offline", "{doc}");
    assert_eq!(sync["to_push"], 1, "{doc}");
    assert_eq!(h.message(&root), MESSAGE);
    // Its own background push found the remote offline too.
    let record = git_file(&root, LAST_PUSH);
    eventually("the offline record", || {
        LastPush::read(&record).is_some_and(|r| r.result == PushResult::Offline)
    });
    wait_for_background(&root);

    // Online again, past the hold-off: the next command's background push
    // sends it.
    h.git(&root, &["remote", "set-url", "origin", &origin]);
    let mut last = LastPush::read(&record).unwrap();
    last.at = background::timestamp(SystemTime::now() - Duration::from_secs(11 * 60));
    last.write(&record).unwrap();
    let local = h.head(&root);
    let (code, _, err) = h.json_run(&["status", "--brain", "alpha"]);
    assert_eq!(code, 0, "{err}");
    eventually("the push", || {
        LastPush::read(&record).is_some_and(|r| r.result == PushResult::Pushed)
    });
    assert_eq!(h.remote_head("alpha"), local);
}

// --- brains the session leaves alone ----------------------------------------------

#[test]
fn a_brain_with_no_remote_or_no_git_is_dreamed_as_it_always_was() {
    for git in [true, false] {
        let mut h = Home::new();
        let root = h.dir.path().join("solo");
        h.write(&root, "kluris.yml", "name: solo\ndescription: d\n");
        h.write(
            &root,
            "glossary.md",
            "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
        );
        h.write(&root, "notes/first.md", &neuron("First", BODY));
        h.register("solo", &root);
        if git {
            h.git(&root, &["init", "-q", "-b", "main"]);
            h.configure(&root, "Ana Tester", "ana@example.com");
            h.git(&root, &["add", "-A"]);
            h.git(&root, &["commit", "-q", "-m", "init"]);
        }
        let before = git.then(|| h.head(&root));
        let (code, doc, err) = h.json_run(&["dream", "--brain", "solo"]);
        assert_eq!((code, err.as_str()), (0, ""), "git {git}: {doc}");
        assert_eq!(keys(&doc), DREAM_KEYS, "git {git}");
        let (code, doc, err) = h.json_run(&["mri", "--brain", "solo"]);
        assert_eq!((code, err.as_str()), (0, ""), "git {git}: {doc}");
        assert_eq!(keys(&doc["brains"][0]), MRI_KEYS, "git {git}");
        let (code, out, err) = h.run(&["dream", "--brain", "solo"]);
        assert_eq!((code, err.as_str()), (0, ""), "git {git}");
        assert!(!out.contains("Committed"), "git {git}: {out}");
        if let Some(before) = before {
            assert_eq!(h.head(&root), before);
            // dream's fixes stay in the work tree, as always.
            assert!(
                h.status(&root).contains("notes/map.md"),
                "{}",
                h.status(&root)
            );
        }
    }
}

// --- the last background push on the stderr line ----------------------------------

#[test]
fn the_stderr_line_says_what_the_last_background_push_did() {
    let (h, root) = team();
    let (code, _, err) = h.run(&["status", "--brain", "alpha"]);
    assert_eq!((code, err.as_str()), (0, ""));
    let record = git_file(&root, LAST_PUSH);
    let write = |result: PushResult, detail: Option<&str>, commits: usize| {
        LastPush {
            at: background::timestamp(SystemTime::now()),
            head: h.head(&root),
            result,
            detail: detail.map(str::to_string),
            commits,
        }
        .write(&record)
        .unwrap();
    };
    for (result, detail, commits, line) in [
        (
            PushResult::Refused,
            Some("the pre-push hook refused it"),
            1,
            "kluris: alpha: last push refused: the pre-push hook refused it\n",
        ),
        (
            PushResult::NeedsPull,
            Some("the remote has commits this brain does not"),
            1,
            "kluris: alpha: last push needs_pull\n",
        ),
        (
            PushResult::Pushed,
            None,
            2,
            "kluris: alpha: pushed 2 commits in the background\n",
        ),
    ] {
        write(result, detail, commits);
        let (code, _, err) = h.run(&["status", "--brain", "alpha"]);
        assert_eq!((code, err.as_str()), (0, line), "{result:?}");
    }
    // A success is said once.
    let (code, _, err) = h.run(&["status", "--brain", "alpha"]);
    assert_eq!((code, err.as_str()), (0, ""));
}
