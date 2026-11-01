//! The reads sync: `wake-up`, `search`, `status`, `register` and `doctor`
//! bring a teammate's commits in before they read, through the session
//! (`kluris_core::sync::session`), and say what the sync did in a `sync`
//! object (all eight keys, absent when there is nothing to say) and, in
//! text, in one stderr line per brain. A read's exit code is never its
//! sync's. `list`, `companion` and `remove` reach no remote.
//!
//! Real git, the built binary under a sandbox HOME and `KLURIS_CONFIG`,
//! and local bare remotes.

mod sync_home;

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kluris_core::sync::claim::RemoteClaim;
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

/// A conflict's keys, in order.
const CONFLICT_KEYS: [&str; 7] = ["path", "kind", "merge", "ours", "theirs", "base", "binary"];

/// The longest a test waits for a process to do something.
const PATIENCE: Duration = Duration::from_secs(60);

/// Polls `done` until it holds, at most [`PATIENCE`].
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

// --- the five reads -------------------------------------------------------------

/// A command that reads one brain, `alpha`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    WakeUp,
    Search,
    Status,
    Register,
    Doctor,
}

const READS: [Read; 5] = [
    Read::WakeUp,
    Read::Search,
    Read::Status,
    Read::Register,
    Read::Doctor,
];

impl Read {
    /// Its arguments for the brain at `root`.
    fn args(self, root: &Path, json: bool) -> Vec<String> {
        let mut args: Vec<String> = match self {
            Read::WakeUp => vec!["wake-up".into(), "--brain".into(), "alpha".into()],
            Read::Search => vec![
                "search".into(),
                "zebras".into(),
                "--brain".into(),
                "alpha".into(),
            ],
            Read::Status => vec!["status".into(), "--brain".into(), "alpha".into()],
            Read::Register => vec!["register".into(), root.display().to_string()],
            Read::Doctor => vec!["doctor".into()],
        };
        if json {
            args.push("--json".into());
        }
        args
    }

    /// The object of its JSON that holds the brain's `sync` key.
    fn entry(self, doc: &Value) -> &Value {
        match self {
            Read::WakeUp | Read::Search | Read::Register => doc,
            Read::Status => &doc["brains"][0],
            Read::Doctor => doc["checks"]
                .as_array()
                .and_then(|checks| checks.iter().find(|c| c["name"] == "brain:alpha"))
                .unwrap_or_else(|| panic!("no brain check: {doc}")),
        }
    }

    /// The keys of its entry, in order, when the brain has nothing to say.
    fn quiet_keys(self) -> &'static [&'static str] {
        match self {
            Read::WakeUp => &[
                "ok",
                "name",
                "path",
                "description",
                "brain_md",
                "lobes",
                "total_neurons",
                "total_yaml_neurons",
                "recent",
                "glossary",
                "deprecation_count",
                "deprecation",
                "parse_errors",
            ],
            Read::Search => &["ok", "brain", "query", "total", "results", "engine"],
            Read::Status => &[
                "name",
                "lobes",
                "neurons",
                "uncommitted",
                "git_enabled",
                "recent_commits",
            ],
            Read::Register => &["ok", "name", "path", "already_registered"],
            Read::Doctor => &["name", "passed", "detail"],
        }
    }

    /// The keys of its entry, in order, with a `sync` object that is not a
    /// warning.
    fn keys_with_sync(self) -> Vec<&'static str> {
        let mut keys = self.quiet_keys().to_vec();
        keys.push("sync");
        keys
    }

    /// Whether its output shows the teammate's `notes/zebra.md`; `None`
    /// for a read whose output does not list the brain's files.
    fn shows_zebra(self, doc: &Value) -> Option<bool> {
        let has = |list: &Value, key: &str| {
            list.as_array()
                .is_some_and(|items| items.iter().any(|i| i[key] == "notes/zebra.md"))
        };
        match self {
            Read::WakeUp => Some(has(&doc["recent"], "path")),
            Read::Search => Some(has(&doc["results"], "file")),
            Read::Status => Some(
                doc["brains"][0]["recent_commits"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|s| s == "notes/zebra.md")),
            ),
            Read::Register | Read::Doctor => None,
        }
    }
}

impl Home {
    /// `kluris <args>` with every variable [`Home::run`] sets.
    fn run_args(&self, args: &[String]) -> (i32, String, String) {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&args)
    }

    fn read_json(&self, read: Read, root: &Path) -> (i32, Value, String) {
        let (code, out, err) = self.run_args(&read.args(root, true));
        let doc =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{read:?}: {e}: {out}{err}"));
        (code, doc, err)
    }

    /// The command as a process of its own, with [`Home::run`]'s variables.
    fn spawn(&self, args: &[&str]) -> Child {
        let mut cmd = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.hermetic(&mut cmd);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.spawn().unwrap()
    }
}

/// A home with the brain `alpha`, pushed to its remote, and its teammate.
fn team() -> (Home, PathBuf) {
    let mut h = Home::new();
    let root = h.brain("alpha");
    (h, root)
}

/// The teammate pushes `notes/<name>.md`, a neuron about zebras.
fn teammate_pushes_zebra(h: &Home, name: &str) {
    h.teammate_pushes(
        "alpha",
        &format!("notes/{name}.md"),
        &neuron("Zebra", "All about zebras.\n"),
    );
}

/// The brain's fetch stamp, `age` seconds old (negative: from the future).
fn stamp_age(root: &Path, age: i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let stamp = git_file(root, "kluris-sync/fetch-stamp");
    fs::create_dir_all(stamp.parent().unwrap()).unwrap();
    fs::write(stamp, format!("{}\n", now - age)).unwrap();
}

/// What the stderr line adds for the record the background push of the
/// brain at `root` left: a remote that moved while a conflict waits is
/// `needs_pull`; nothing where no push ran (on Windows the runner's job
/// may keep it from starting).
fn last_push_said(root: &Path) -> &'static str {
    if git_file(root, "kluris-sync/last-push.json").is_file() {
        "; last push needs_pull"
    } else {
        ""
    }
}

/// The `sync` object of `entry`, checked to have its eight keys.
fn sync_of(entry: &Value) -> &Value {
    let sync = &entry["sync"];
    assert_eq!(keys(sync), SYNC_KEYS, "{entry}");
    sync
}

fn reason(sync: &Value) -> &Value {
    &sync["left_as_is"]["reason"]
}

#[test]
fn each_read_pulls_a_teammates_commit_first_and_says_so() {
    for read in READS {
        let (h, root) = team();
        // Nothing to say: no key, and nothing on stderr.
        let (code, doc, err) = h.read_json(read, &root);
        assert_eq!(code, 0, "{read:?}: {doc}{err}");
        let entry = read.entry(&doc);
        assert_eq!(keys(entry), read.quiet_keys(), "{read:?}: {doc}");
        let (code, _, err) = h.run_args(&read.args(&root, false));
        assert_eq!((code, err.as_str()), (0, ""), "{read:?}");

        // Text: one stderr line.
        teammate_pushes_zebra(&h, "zebra");
        stamp_age(&root, 61);
        let (code, _, err) = h.run_args(&read.args(&root, false));
        assert_eq!(code, 0, "{read:?}: {err}");
        assert_eq!(err, "kluris: alpha: pulled 1 commit\n", "{read:?}");
        assert!(root.join("notes").join("zebra.md").is_file(), "{read:?}");

        // JSON: the object after the command's own keys; no stderr line.
        teammate_pushes_zebra(&h, "zebra-two");
        stamp_age(&root, 61);
        let (code, doc, err) = h.read_json(read, &root);
        assert_eq!((code, err.as_str()), (0, ""), "{read:?}: {doc}");
        let entry = read.entry(&doc);
        assert_eq!(keys(entry), read.keys_with_sync(), "{read:?}: {doc}");
        let sync = sync_of(entry);
        assert_eq!(
            sync,
            &json!({
                "pulled": 1, "auto_merged": [],
                "left_as_is": {"reason": null, "paths": [], "detail": null},
                "conflicts": [], "behind_conflict": 0, "to_push": 0, "pushed": false,
                "last_push": null,
            }),
            "{read:?}"
        );
        if let Some(shown) = read.shows_zebra(&doc) {
            assert!(shown, "{read:?} read the brain before the pull: {doc}");
        }
        assert_eq!(h.git(&root, &["status", "--porcelain"]), "", "{read:?}");
    }
}

#[test]
fn wake_up_reads_what_it_pulled_and_no_tag() {
    let (h, root) = team();
    let (_, before, _) = h.read_json(Read::WakeUp, &root);
    let neurons = before["total_neurons"].as_u64().unwrap();
    teammate_pushes_zebra(&h, "zebra");
    let mate = h.teammate("alpha");
    h.git(&mate, &["tag", "v9"]);
    h.git(&mate, &["push", "-q", "origin", "v9"]);
    stamp_age(&root, 61);
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["total_neurons"].as_u64(), Some(neurons + 1), "{doc}");
    assert_eq!(h.git(&root, &["tag"]), "", "a tag came with the pull");
}

#[test]
fn a_reads_exit_code_is_never_its_syncs() {
    // A port nothing listens on: offline.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    for read in READS {
        let (h, root) = team();
        let (quiet, _, _) = h.read_json(read, &root);
        teammate_pushes_zebra(&h, "zebra");
        let gone = h.dir.path().join("gone.git").display().to_string();
        let offline = format!("git://127.0.0.1:{closed}/alpha.git");
        for (url, expected) in [(gone.as_str(), "refused"), (offline.as_str(), "offline")] {
            h.git(&root, &["remote", "set-url", "origin", url]);
            stamp_age(&root, 61);
            let (code, doc, _) = h.read_json(read, &root);
            assert_eq!(code, quiet, "{read:?} {expected}: {doc}");
            assert_eq!(reason(sync_of(read.entry(&doc))), expected, "{read:?}");
            let _ = fs::remove_file(git_file(&root, "kluris-sync/fetch-backoff"));
        }
        // Another command holds the remote claim: busy.
        let origin = h.dir.path().join("alpha.git").display().to_string();
        h.git(&root, &["remote", "set-url", "origin", &origin]);
        let held = RemoteClaim::try_take(&root.join(".git")).unwrap();
        stamp_age(&root, 61);
        let (code, doc, _) = h.read_json(read, &root);
        assert_eq!(code, quiet, "{read:?} busy: {doc}");
        assert_eq!(reason(sync_of(read.entry(&doc))), "busy", "{read:?}");
        drop(held);
        assert!(!root.join("notes").join("zebra.md").exists(), "{read:?}");
    }
}

// --- conflicts --------------------------------------------------------------------

/// The waiting merge's id, from its record.
fn recorded_merge(h: &Home) -> String {
    h.merge("alpha")
}

#[test]
fn a_conflict_a_read_meets_carries_both_texts_and_its_merge() {
    let (h, root) = team();
    h.conflict("alpha", &root);
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    let sync = sync_of(&doc);
    let conflicts = sync["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{doc}");
    let c = &conflicts[0];
    assert_eq!(keys(c), CONFLICT_KEYS);
    assert_eq!(
        (&c["path"], &c["kind"]),
        (&json!("notes/first.md"), &json!("content"))
    );
    assert!(c["ours"].as_str().unwrap().contains("Three, ours."), "{c}");
    assert!(
        c["theirs"].as_str().unwrap().contains("Three, theirs."),
        "{c}"
    );
    assert!(c["base"].as_str().unwrap().contains("Three.\n"), "{c}");
    assert_eq!(c["binary"], false);
    assert_eq!(c["merge"], recorded_merge(&h).as_str());
    assert_eq!(sync["to_push"], 1);
    assert_eq!(sync["pulled"], 0);
    // The text says it on stderr, with what the background push of the
    // commit waiting found, when one ran.
    let last = last_push_said(&root);
    let (code, _, err) = h.run_args(&Read::WakeUp.args(&root, false));
    assert_eq!(code, 0);
    assert_eq!(
        err,
        format!("kluris: alpha: 1 conflict (notes/first.md); 1 commit to push{last}\n")
    );
    wait_for_background(&root);
}

#[test]
fn status_shows_a_waiting_conflict_and_only_then() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    let (_, clean) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(keys(&clean["brains"][0]), Read::Status.quiet_keys());
    h.conflict("alpha", &root);
    let (code, synced) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{synced}");
    // Inside the throttle the sync just stamped: the record says it.
    let (code, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), Read::Status.keys_with_sync());
    let sync = sync_of(entry);
    assert_eq!(sync["conflicts"], synced["brains"][0]["sync"]["conflicts"]);
    assert_eq!(sync["behind_conflict"], 0);
    // The text says it too, with what the background push of the commit
    // waiting found, once it is done.
    wait_for_background(&root);
    let last = last_push_said(&root);
    let (code, out, err) = h.run(&["status", "--brain", "alpha"]);
    assert_eq!(code, 0);
    assert!(!out.contains("Sync:"), "{out}");
    assert_eq!(
        err,
        format!("kluris: alpha: 1 conflict (notes/first.md); 1 commit to push{last}\n")
    );
    // Once settled and pushed, no conflict and nothing to push is said: no
    // key, or (where the background push ran and found the remote ahead)
    // its record alone, which stays until a push of a session succeeds.
    wait_for_background(&root);
    let merge = h.merge("alpha");
    let (code, done) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--ours",
        "--merge",
        &merge,
        "--json",
    ]);
    assert_eq!(code, 0, "{done}");
    let (_, doc) = h.json(&["status", "--brain", "alpha", "--json"]);
    let entry = &doc["brains"][0];
    if git_file(&root, "kluris-sync/last-push.json").exists() {
        let sync = sync_of(entry);
        assert!(sync["last_push"].is_object(), "{doc}");
        let mut said = sync.clone();
        said["last_push"] = Value::Null;
        assert_eq!(
            said,
            json!({
                "pulled": 0, "auto_merged": [],
                "left_as_is": {"reason": null, "paths": [], "detail": null},
                "conflicts": [], "behind_conflict": 0, "to_push": 0, "pushed": false,
                "last_push": null,
            }),
            "{doc}"
        );
    } else {
        assert_eq!(keys(entry), Read::Status.quiet_keys(), "{doc}");
    }
}

#[test]
fn a_waiting_merge_record_of_a_brain_without_an_upstream_is_not_said() {
    // The session leaves such a brain alone: nothing of it is read.
    let mut h = Home::new();
    let root = local_brain(&h, "solo");
    h.register("solo", &root);
    let pending = git_file(&root, "kluris-sync/pending.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    let (a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
    fs::write(
        &pending,
        format!(
            "{{\"base\": \"{a}\", \"ours\": \"{b}\", \"theirs\": \"{c}\", \"conflicts\": [{{\"path\": \"lobe/a.md\", \"kind\": \"content\", \"base\": \"{a}\", \"ours\": \"{b}\", \"theirs\": \"{c}\"}}], \"created\": \"2026-01-01T00:00:00Z\"}}"
        ),
    )
    .unwrap();
    let (code, doc) = h.json(&["status", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc["brains"][0]), Read::Status.quiet_keys(), "{doc}");
    let (code, out, err) = h.run(&["status"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(!out.contains("Sync:"), "{out}");
}

#[test]
fn status_of_two_brains_reports_each_in_its_entry() {
    let mut h = Home::new();
    let alpha = h.brain("alpha");
    h.brain("beta");
    h.conflict("alpha", &alpha);
    let (code, doc) = h.json(&["status", "--brain", "all", "--json"]);
    assert_eq!(code, 0, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    assert_eq!(brains[0]["name"], "alpha");
    assert_eq!(
        sync_of(&brains[0])["conflicts"][0]["path"],
        "notes/first.md"
    );
    assert_eq!(brains[1]["name"], "beta");
    assert_eq!(keys(&brains[1]), Read::Status.quiet_keys(), "{doc}");
    wait_for_background(&alpha);
}

// --- doctor -------------------------------------------------------------------

#[test]
fn doctor_reads_the_brain_it_pulled() {
    let (h, root) = team();
    // Without its kluris.yml the brain's check fails.
    fs::remove_file(root.join("kluris.yml")).unwrap();
    let (code, doc, _) = h.read_json(Read::Doctor, &root);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(Read::Doctor.entry(&doc)["passed"], false, "{doc}");
    // The teammate commits it.
    let mate = h.teammate("alpha");
    h.write(&mate, ".gitignore", "");
    h.commit(&mate, "kluris.yml", "name: alpha\ndescription: d\n");
    h.git(&mate, &["push", "-q"]);
    stamp_age(&root, 61);
    let (code, doc, _) = h.read_json(Read::Doctor, &root);
    assert_eq!(code, 0, "{doc}");
    let check = Read::Doctor.entry(&doc);
    assert_eq!(check["passed"], true, "{doc}");
    assert_eq!(check["detail"], root.display().to_string());
    assert_eq!(sync_of(check)["pulled"], 1);
}

#[test]
fn doctor_warns_for_a_brain_offline_or_in_conflict_and_never_fails() {
    let mut h = Home::new();
    let alpha = h.brain("alpha");
    let beta = h.brain("beta");
    let (quiet, _) = h.json(&["doctor", "--json"]);
    h.conflict("alpha", &alpha);
    let gone = h.dir.path().join("gone.git").display().to_string();
    h.git(&beta, &["remote", "set-url", "origin", &gone]);
    stamp_age(&alpha, 61);
    stamp_age(&beta, 61);
    let (code, doc) = h.json(&["doctor", "--json"]);
    assert_eq!(code, quiet, "{doc}");
    let check = |name: &str| {
        doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == format!("brain:{name}"))
            .unwrap()
            .clone()
    };
    let (a, b) = (check("alpha"), check("beta"));
    for c in [&a, &b] {
        assert_eq!(
            keys(c),
            ["name", "passed", "detail", "sync", "warning"],
            "{c}"
        );
        assert_eq!(
            (&c["passed"], &c["warning"]),
            (&json!(true), &json!(true)),
            "{c}"
        );
    }
    assert_eq!(
        a["detail"],
        format!(
            "{}; 1 conflict (notes/first.md); 1 commit to push",
            alpha.display()
        )
    );
    assert_eq!(reason(sync_of(&b)), "refused");
    let detail = b["detail"].as_str().unwrap();
    assert!(
        detail.starts_with(&format!("{}; left as is (refused): ", beta.display())),
        "{detail}"
    );
    // In text: WARN, and each brain's line on stderr, once the background
    // push of alpha's commit lets its remote go.
    wait_for_background(&alpha);
    stamp_age(&alpha, 61);
    stamp_age(&beta, 61);
    let last = last_push_said(&alpha);
    let (code, out, err) = h.run(&["doctor"]);
    assert_eq!(code, quiet);
    assert!(out.contains("  WARN brain:alpha: "), "{out}");
    assert!(out.contains("  WARN brain:beta: "), "{out}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 2, "{err}");
    assert_eq!(
        lines[0],
        format!("kluris: alpha: 1 conflict (notes/first.md); 1 commit to push{last}")
    );
    assert!(
        lines[1].starts_with("kluris: beta: left as is (refused): "),
        "{err}"
    );
    wait_for_background(&alpha);
}

#[test]
fn two_names_on_one_brain_open_it_once_and_share_its_report() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.register("again", &root);
    teammate_pushes_zebra(&h, "zebra");
    stamp_age(&root, 61);
    let (code, doc) = h.json(&["doctor", "--no-refresh", "--json"]);
    assert_eq!(code, 0, "{doc}");
    for name in ["brain:alpha", "brain:again"] {
        let check = doc["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        assert_eq!(sync_of(check)["pulled"], 1, "{name}: {doc}");
    }
}

// --- register -------------------------------------------------------------------

#[test]
fn register_reads_the_identity_it_pulled() {
    let (h, root) = team();
    let (code, _, err) = h.run(&["remove", "alpha", "--json"]);
    assert_eq!(code, 0, "{err}");
    let mate = h.teammate("alpha");
    let brain_md = h.read(&mate, "brain.md");
    assert!(brain_md.contains("# alpha\n"), "{brain_md}");
    h.commit(
        &mate,
        "brain.md",
        &brain_md.replacen("# alpha\n", "# beta\n", 1),
    );
    h.git(&mate, &["push", "-q"]);
    stamp_age(&root, 61);
    let (code, doc) = h.json(&["register", &root.display().to_string(), "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["name"], "beta", "registered before the pull: {doc}");
    assert_eq!(keys(&doc), ["ok", "name", "path", "companions", "sync"]);
    assert_eq!(sync_of(&doc)["pulled"], 1);
}

/// A brain named `dir` at `<home>/<dir>` with one commit and no remote.
fn local_brain(h: &Home, dir: &str) -> PathBuf {
    let root = h.dir.path().join(dir);
    h.write(&root, "brain.md", &format!("# {dir}\n"));
    h.git(&root, &["init", "-q", "-b", "main"]);
    h.configure(&root, "Ana Tester", "ana@example.com");
    h.git(&root, &["add", "-A"]);
    h.git(&root, &["commit", "-q", "-m", "init"]);
    root
}

#[test]
fn register_says_a_brain_has_no_upstream_and_sets_none() {
    let h = Home::new();
    let root = local_brain(&h, "solo");
    let bare = h.dir.path().join("solo.git").display().to_string();
    h.git(h.dir.path(), &["init", "-q", "--bare", "-b", "main", &bare]);
    h.git(&root, &["remote", "add", "origin", &bare]);
    let (code, doc) = h.json(&["register", &root.display().to_string(), "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["sync"]["left_as_is"],
        json!({"reason": "no_upstream", "paths": [],
               "detail": "set one with git push -u origin main"}),
        "{doc}"
    );
    sync_of(&doc);
    let mut cmd = StdCommand::new("git");
    h.hermetic(&mut cmd);
    let upstream = cmd
        .args(["rev-parse", "--abbrev-ref", "main@{upstream}"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!upstream.status.success(), "an upstream was set");
    let (code, _, err) = h.run(&["register", &root.display().to_string()]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        err,
        "kluris: solo: left as is (no_upstream): set one with git push -u origin main\n"
    );
}

#[test]
fn register_of_a_brain_outside_git_nested_or_without_a_remote_says_nothing() {
    let h = Home::new();
    let plain = h.dir.path().join("plain");
    h.write(&plain, "brain.md", "# plain\n");
    let solo = local_brain(&h, "solo");
    let project = local_brain(&h, "project");
    let nested = project.join("kb");
    h.write(&nested, "brain.md", "# nested\n");
    h.git(&project, &["add", "-A"]);
    h.git(&project, &["commit", "-q", "-m", "kb"]);
    for (root, name) in [(&plain, "plain"), (&nested, "nested"), (&solo, "solo")] {
        let (code, doc) = h.json(&["register", &root.display().to_string(), "--json"]);
        assert_eq!(code, 0, "{doc}");
        assert_eq!(doc["name"], name);
        assert_eq!(keys(&doc), ["ok", "name", "path", "companions"], "{doc}");
    }
}

// --- the background push ------------------------------------------------------------

#[cfg(unix)]
#[test]
fn commits_a_read_leaves_waiting_go_to_one_background_push() {
    let (h, root) = team();
    h.commit(&root, "notes/mine.md", &neuron("Mine", "Mine.\n"));
    teammate_pushes_zebra(&h, "zebra");
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    let sync = sync_of(&doc);
    // Mine and the merge commit wait.
    assert_eq!(
        (&sync["pulled"], &sync["to_push"]),
        (&json!(1), &json!(2)),
        "{doc}"
    );
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    let remote = h.dir.path().join("alpha.git");
    eventually("the merge reaches the remote", || {
        h.git(&remote, &["rev-parse", "main"]) == head
    });
    wait_for_background(&root);
    let record: Value = serde_json::from_str(
        &fs::read_to_string(git_file(&root, "kluris-sync/last-push.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["result"], "pushed", "{record}");
}

// --- no off switch, no network for the others ----------------------------------------

#[test]
fn no_variable_or_setting_turns_a_reads_sync_off() {
    let (mut h, root) = team();
    h.env.push(("KLURIS_NO_AUTO_SYNC".into(), "1".into()));
    teammate_pushes_zebra(&h, "zebra");
    let (_, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(sync_of(&doc)["pulled"], 1, "{doc}");
    h.env.clear();
    for setting in ["sync:\n  auto_fetch: false\n", "sync: {auto: false}\n"] {
        h.write(
            &root,
            "kluris.yml",
            &format!("name: alpha\ndescription: d\n{setting}"),
        );
        teammate_pushes_zebra(&h, &format!("zebra-{}", setting.len()));
        stamp_age(&root, 61);
        let (_, doc, _) = h.read_json(Read::Search, &root);
        assert_eq!(sync_of(&doc)["pulled"], 1, "{setting}: {doc}");
    }
}

#[test]
fn list_companion_and_remove_reach_no_remote() {
    let (h, root) = team();
    let marker = h.dir.path().join("reached");
    let script = h.dir.path().join("marking-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ntouch '{}'\nexec git upload-pack \"$@\"\n",
            sh_path(&marker)
        ),
    )
    .unwrap();
    let pack = format!("sh '{}'", sh_path(&script));
    h.git(&root, &["config", "remote.origin.uploadpack", &pack]);
    teammate_pushes_zebra(&h, "zebra");
    stamp_age(&root, 61);
    for args in [
        &["list", "--json"][..],
        &["companion", "list", "--brain", "alpha", "--json"],
        &[
            "companion",
            "add",
            "specmint-core",
            "--brain",
            "alpha",
            "--json",
        ],
        &[
            "companion",
            "remove",
            "specmint-core",
            "--brain",
            "alpha",
            "--json",
        ],
        &["remove", "alpha", "--json"],
    ] {
        let (code, out, err) = h.run(args);
        assert_eq!(code, 0, "{args:?}: {out}{err}");
        assert!(!marker.exists(), "{args:?} reached the remote");
    }
    // The marker works: a read reaches it.
    let (code, doc) = h.json(&["register", &root.display().to_string(), "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert!(marker.exists(), "the read did not reach the remote");
}

// --- what a read meets in the work tree --------------------------------------------

/// What a read must leave exactly as it was.
#[derive(Debug, PartialEq, Eq)]
struct Exact {
    head: String,
    /// The SHA-256 of `.git/index`'s raw bytes.
    index: String,
    index_lock: bool,
    /// Each path outside `.git`: its permission bits, and its bytes, link
    /// target or `<dir>`.
    paths: Vec<(PathBuf, u32, Vec<u8>)>,
}

/// The whole mode on Unix; read-only or not elsewhere.
#[cfg(unix)]
fn permission_bits(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn permission_bits(meta: &fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Home {
    /// HEAD, the raw index, `index.lock` and every path of the clone at
    /// `root`, read without running anything that could write the index.
    fn exact(&self, root: &Path) -> Exact {
        let mut paths = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).unwrap();
                let content = if meta.file_type().is_symlink() {
                    let target = fs::read_link(&path).unwrap();
                    format!("<link {}>", target.display()).into_bytes()
                } else if meta.is_dir() {
                    stack.push(path.clone());
                    b"<dir>".to_vec()
                } else {
                    fs::read(&path).unwrap()
                };
                paths.push((path, permission_bits(&meta), content));
            }
        }
        paths.sort();
        let git_dir = root.join(".git");
        Exact {
            head: self.git(root, &["rev-parse", "HEAD"]),
            index: sha256_hex(&fs::read(git_dir.join("index")).unwrap()),
            index_lock: fs::symlink_metadata(git_dir.join("index.lock")).is_ok(),
            paths,
        }
    }

    /// The teammate rewrites `notes/first.md` and pushes.
    fn teammate_changes_first(&self) {
        self.teammate_pushes(
            "alpha",
            "notes/first.md",
            &neuron("First", &BODY.replace("Two.", "Two, rotated.")),
        );
    }
}

/// The brain's state before an incoming change the read must not land
/// over: what it sets up, and the path in the way.
type InTheWay = (&'static str, fn(&Home, &Path), &'static str);

const IN_THE_WAY: [InTheWay; 3] = [
    (
        "a new untracked file at an incoming path",
        |h, root| {
            teammate_pushes_zebra(h, "zebra");
            h.write(root, "notes/zebra.md", "# my own zebra draft\n");
        },
        "notes/zebra.md",
    ),
    (
        "a new ignored file at an incoming path",
        |h, root| {
            teammate_pushes_zebra(h, "zebra");
            fs::write(git_file(root, "info/exclude"), "notes/zebra.md\n").unwrap();
            h.write(root, "notes/zebra.md", "# my ignored zebra draft\n");
            h.git(root, &["check-ignore", "-q", "notes/zebra.md"]);
        },
        "notes/zebra.md",
    ),
    (
        "an uncommitted edit to an incoming file",
        |h, root| {
            h.teammate_changes_first();
            h.write(root, "notes/first.md", &neuron("First", "my own edit\n"));
        },
        "notes/first.md",
    ),
];

#[test]
fn an_uncommitted_file_in_the_way_is_kept_and_the_brain_read_as_it_is() {
    for (what, set_up, path) in IN_THE_WAY {
        let (h, root) = team();
        set_up(&h, &root);
        let before = h.exact(&root);
        for read in [Read::WakeUp, Read::Search] {
            stamp_age(&root, 61);
            let (code, doc, _) = h.read_json(read, &root);
            assert_eq!(code, 0, "{what}, {read:?}: {doc}");
            let left = &sync_of(read.entry(&doc))["left_as_is"];
            assert_eq!(left["reason"], "uncommitted", "{what}, {read:?}: {doc}");
            assert_eq!(left["paths"], json!([path]), "{what}, {read:?}");
        }
        assert_eq!(h.exact(&root), before, "{what}");
    }
}

#[test]
fn a_live_git_holding_index_lock_keeps_it_and_finishes() {
    let (h, root) = team();
    teammate_pushes_zebra(&h, "zebra");
    let note = h.dir.path().join("held.md");
    fs::write(&note, "# held\n").unwrap();
    let blob = h
        .git(&root, &["hash-object", "-w", &note.display().to_string()])
        .trim()
        .to_string();
    let mut cmd = StdCommand::new("git");
    h.hermetic(&mut cmd);
    cmd.args(["update-index", "--index-info"])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut writer = cmd.spawn().unwrap();
    let lock = git_file(&root, "index.lock");
    eventually("the writer takes index.lock", || lock.exists());
    let before = h.exact(&root);

    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(reason(sync_of(&doc)), "busy", "{doc}");
    assert_eq!(h.exact(&root), before);

    let mut stdin = writer.stdin.take().unwrap();
    stdin
        .write_all(format!("100644 {blob}\tnotes/held.md\n").as_bytes())
        .unwrap();
    drop(stdin);
    let out = writer.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "the writer failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!lock.exists());
    assert_eq!(
        h.git(&root, &["ls-files", "notes/held.md"]).trim(),
        "notes/held.md"
    );
}

/// A clean tree in a state git treats specially, which the pull lands
/// into like any other: what it sets up.
type Clean = (&'static str, fn(&Home, &Path));

const CLEAN: [Clean; 5] = [
    ("a plain clean tree", |_, _| {}),
    ("an LF file under core.autocrlf", |h, root| {
        h.git(root, &["config", "core.autocrlf", "true"]);
    }),
    ("an assume-unchanged entry", |h, root| {
        h.git(
            root,
            &["update-index", "--assume-unchanged", "notes/first.md"],
        );
    }),
    ("an owner-only file", |_, root| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join("notes").join("first.md"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        let _ = root;
    }),
    ("an executable file under core.filemode false", |h, root| {
        h.git(root, &["config", "core.filemode", "false"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join("notes").join("first.md"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
    }),
];

#[test]
fn a_clean_tree_is_pulled_into() {
    for (what, set_up) in CLEAN {
        let (h, root) = team();
        set_up(&h, &root);
        h.teammate_changes_first();
        let (code, doc, _) = h.read_json(Read::WakeUp, &root);
        assert_eq!(code, 0, "{what}: {doc}");
        assert_eq!(sync_of(&doc)["pulled"], 1, "{what}: {doc}");
        let text = h.read(&root, "notes/first.md").replace("\r\n", "\n");
        assert!(text.contains("Two, rotated."), "{what}: {text}");
        let remote = h.dir.path().join("alpha.git");
        assert_eq!(
            h.git(&root, &["rev-parse", "HEAD"]),
            h.git(&remote, &["rev-parse", "main"]),
            "{what}"
        );
    }
}

#[test]
fn a_skip_worktree_edit_is_never_overwritten() {
    let (h, root) = team();
    h.teammate_changes_first();
    h.git(
        &root,
        &["update-index", "--skip-worktree", "notes/first.md"],
    );
    let mine = neuron("First", "my edit, hidden from git\n");
    h.write(&root, "notes/first.md", &mine);
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    // git's own land refuses to write over it: the brain is left as it is.
    let sync = sync_of(&doc);
    assert_eq!(reason(sync), "failed", "{doc}");
    let detail = sync["left_as_is"]["detail"].as_str().unwrap();
    assert!(detail.contains("would be overwritten"), "{detail}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(h.read(&root, "notes/first.md"), mine, "{doc}");
}

#[test]
fn an_incoming_case_alias_of_a_tracked_file_is_kept() {
    // On a case-insensitive file system (macOS, Windows) the incoming
    // `notes/A.md` is the same file as the tracked `notes/a.md`.
    let (h, root) = team();
    let mate = h.teammate("alpha");
    h.commit(&mate, "notes/a.md", "# a\n\nlower case\n");
    h.git(&mate, &["push", "-q"]);
    h.git(&root, &["pull", "-q", "--ff-only"]);
    // A case-insensitive work tree cannot hold both: the teammate's commit
    // is made with plumbing.
    let upper = h.dir.path().join("upper.md");
    fs::write(&upper, "# A\n\nupper case, a different file\n").unwrap();
    let blob = h
        .git(&mate, &["hash-object", "-w", &upper.display().to_string()])
        .trim()
        .to_string();
    h.git(
        &mate,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},notes/A.md"),
        ],
    );
    h.git(&mate, &["commit", "-q", "-m", "A"]);
    h.git(&mate, &["push", "-q"]);
    let lower = h.read(&root, "notes/a.md");
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    let insensitive = root.join("notes").join("A.MD").exists();
    if insensitive {
        assert_eq!(reason(sync_of(&doc)), "uncommitted", "{doc}");
        assert_eq!(h.read(&root, "notes/a.md"), lower);
    } else {
        assert_eq!(sync_of(&doc)["pulled"], 1, "{doc}");
    }
}

#[test]
fn nothing_new_upstream_leaves_the_brain_and_its_index_as_they_were() {
    // Nothing writes the index, and no ref is updated: the fetch is git's
    // own, so `reference-transaction` runs for its empty transactions.
    let (h, root) = team();
    let log = h.dir.path().join("hooks.log");
    for hook in ["post-index-change", "reference-transaction"] {
        let path = git_file(&root, &format!("hooks/{hook}"));
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho {hook} \"$@\" >> '{0}'\ncat >> '{0}'\n",
                sh_path(&log)
            ),
        )
        .unwrap();
        #[cfg(unix)]
        make_executable(&path);
    }
    let before = h.exact(&root);
    let refs = h.git(&root, &["for-each-ref"]);
    for read in [Read::WakeUp, Read::Search] {
        stamp_age(&root, 61);
        let (code, doc, _) = h.read_json(read, &root);
        assert_eq!(code, 0, "{read:?}: {doc}");
        assert_eq!(keys(read.entry(&doc)), read.quiet_keys(), "{read:?}");
    }
    assert_eq!(h.exact(&root), before);
    assert_eq!(h.git(&root, &["for-each-ref"]), refs);
    // Each hook's line, then the ref updates it was given: none.
    let ran = fs::read_to_string(&log).unwrap_or_default();
    assert!(!ran.contains("post-index-change"), "{ran}");
    assert!(
        ran.lines().all(|l| l.starts_with("reference-transaction ")),
        "{ran}"
    );
}

/// The brain's git state a read must leave as it is.
type InProgress = (&'static str, fn(&Home, &Path));

const IN_PROGRESS: [InProgress; 3] = [
    ("a git merge stopped before its commit", |h, root| {
        h.git(root, &["checkout", "-q", "-b", "side"]);
        h.commit(root, "notes/side.md", "# side\n");
        h.git(root, &["checkout", "-q", "main"]);
        h.git(
            root,
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
    }),
    ("a conflicted git merge", |h, root| {
        h.conflict("alpha", root);
        h.git(root, &["fetch", "-q"]);
        let mut cmd = StdCommand::new("git");
        h.hermetic(&mut cmd);
        let merged = cmd
            .args(["merge", "-q", "origin/main"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(!merged.status.success(), "a conflict");
    }),
    ("a rebase stopped partway", |_, root| {
        // git's own marker of a stopped rebase (as `git rebase -i` leaves it).
        fs::create_dir_all(git_file(root, "rebase-merge")).unwrap();
    }),
];

#[test]
fn a_git_operation_in_progress_leaves_the_brain_as_it_is() {
    for (what, set_up) in IN_PROGRESS {
        let (h, root) = team();
        set_up(&h, &root);
        teammate_pushes_zebra(&h, "zebra");
        let before = h.exact(&root);
        for read in [Read::WakeUp, Read::Search] {
            stamp_age(&root, 61);
            let (code, doc, _) = h.read_json(read, &root);
            assert_eq!(code, 0, "{what}, {read:?}: {doc}");
            let sync = sync_of(&doc);
            assert_eq!(reason(sync), "in_progress", "{what}, {read:?}: {doc}");
            if sync["to_push"] != 0 {
                // Its own commit goes to the background push.
                wait_for_background(&root);
            }
        }
        assert_eq!(h.exact(&root), before, "{what}");
    }
}

#[test]
fn an_unreadable_waiting_merge_record_goes_with_the_pull() {
    let (h, root) = team();
    let pending = git_file(&root, "kluris-sync/pending.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    fs::write(&pending, "{}").unwrap();
    teammate_pushes_zebra(&h, "zebra");
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(sync_of(&doc)["pulled"], 1, "{doc}");
    assert!(!pending.exists());
}

#[test]
fn a_brain_without_an_upstream_or_on_a_detached_head_says_nothing() {
    type Cut = fn(&Home, &Path);
    let cuts: [(&str, Cut); 3] = [
        ("no upstream", |h, root| {
            h.git(root, &["branch", "--unset-upstream"]);
        }),
        ("no remote", |h, root| {
            h.git(root, &["remote", "remove", "origin"]);
        }),
        ("a detached HEAD", |h, root| {
            h.git(root, &["checkout", "-q", "--detach"]);
        }),
    ];
    for (what, cut) in cuts {
        let (h, root) = team();
        teammate_pushes_zebra(&h, "zebra");
        cut(&h, &root);
        let before = h.exact(&root);
        for read in [Read::WakeUp, Read::Search, Read::Status] {
            let (code, doc, err) = h.read_json(read, &root);
            assert_eq!((code, err.as_str()), (0, ""), "{what}, {read:?}");
            assert_eq!(
                keys(read.entry(&doc)),
                read.quiet_keys(),
                "{what}, {read:?}"
            );
        }
        assert_eq!(h.exact(&root), before, "{what}");
    }
}

#[cfg(unix)]
#[test]
fn a_claim_that_cannot_be_taken_fetches_nothing_and_the_read_goes_on() {
    use std::os::unix::fs::PermissionsExt;
    // `kluris-sync/` is there but cannot be written: no claim can be made.
    let (h, root) = team();
    teammate_pushes_zebra(&h, "zebra");
    let folder = git_file(&root, "kluris-sync");
    fs::create_dir_all(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(code, 0, "{doc}");
    assert_eq!(reason(sync_of(&doc)), "failed", "{doc}");
    assert!(!h.has_remote_main(&root), "fetched");
    assert!(!root.join("notes").join("zebra.md").exists());
}

#[test]
fn a_shallow_brain_is_not_fetched() {
    // Deepening a shallow clone writes `.git/shallow` under `shallow.lock`.
    let (h, root) = team();
    h.git(&root, &["fetch", "-q", "--depth=1"]);
    assert_eq!(
        h.git(&root, &["rev-parse", "--is-shallow-repository"])
            .trim(),
        "true"
    );
    teammate_pushes_zebra(&h, "zebra");
    let shallow = fs::read(git_file(&root, "shallow")).unwrap();
    let refs = h.git(&root, &["for-each-ref"]);
    for read in [Read::WakeUp, Read::Search] {
        let (code, doc, _) = h.read_json(read, &root);
        assert_eq!(code, 0, "{read:?}");
        assert_eq!(keys(read.entry(&doc)), read.quiet_keys(), "{read:?}");
    }
    assert_eq!(fs::read(git_file(&root, "shallow")).unwrap(), shallow);
    assert_eq!(h.git(&root, &["for-each-ref"]), refs);
    assert!(!git_file(&root, "kluris-sync/fetch-stamp").exists());
}

#[test]
fn a_brain_nested_in_a_larger_repo_is_never_moved() {
    // A brain inside a project repo: a read never moves a repository the
    // user did not ask Kluris to manage.
    let mut h = Home::new();
    let dir = h.dir.path().to_path_buf();
    h.git(&dir, &["init", "-q", "--bare", "-b", "main", "outer.git"]);
    h.git(&dir, &["clone", "-q", "outer.git", "project"]);
    let theirs = dir.join("project");
    h.configure(&theirs, "Bob Teammate", "bob@example.com");
    h.write(&theirs, "README.md", "# project\n");
    h.write(&theirs, ".gitignore", "kluris.yml\n");
    h.write(&theirs, "kb/brain.md", "# team\n");
    h.write(
        &theirs,
        "kb/notes/jwt.md",
        &neuron("JWT", "We sign every JWT.\n"),
    );
    h.git(&theirs, &["add", "-A"]);
    h.git(&theirs, &["commit", "-q", "-m", "seed"]);
    h.git(&theirs, &["push", "-q", "-u", "origin", "main"]);
    h.git(&dir, &["clone", "-q", "outer.git", "project-mine"]);
    let mine = dir.join("project-mine");
    let brain = mine.join("kb");
    h.write(&brain, "kluris.yml", "name: alpha\n");
    h.register("alpha", &brain);
    h.commit(
        &theirs,
        "kb/notes/zebra.md",
        &neuron("Zebra", "All about zebras.\n"),
    );
    h.git(&theirs, &["push", "-q"]);

    let state = || {
        (
            h.git(&mine, &["rev-parse", "HEAD"]),
            h.git(&mine, &["rev-parse", "refs/remotes/origin/main"]),
            h.git(&mine, &["status", "--porcelain", "--untracked-files=all"]),
        )
    };
    let before = state();
    for read in [Read::WakeUp, Read::Search, Read::Status] {
        let (code, doc, err) = h.read_json(read, &brain);
        assert_eq!((code, err.as_str()), (0, ""), "{read:?}");
        assert_eq!(keys(read.entry(&doc)), read.quiet_keys(), "{read:?}");
    }
    assert_eq!(state(), before, "nothing fetched, nothing moved");
    assert!(!brain.join("notes").join("zebra.md").exists());
    assert!(!mine.join(".git").join("kluris-sync").exists());
}

// --- the remote's time and prompts ---------------------------------------------------

impl Home {
    /// Run `script` (a shell script's body) for every upload-pack the
    /// brain at `root` starts from `origin`.
    fn upload_pack(&self, root: &Path, name: &str, script: &str) {
        let path = self.dir.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
        let pack = format!("sh '{}'", sh_path(&path));
        self.git(root, &["config", "remote.origin.uploadpack", &pack]);
    }

    /// Whether the brain at `root` has the remote's `main` among its
    /// objects.
    fn has_remote_main(&self, root: &Path) -> bool {
        let remote = self.dir.path().join("alpha.git");
        let main = self.git(&remote, &["rev-parse", "main"]);
        let mut cmd = StdCommand::new("git");
        self.hermetic(&mut cmd);
        cmd.args(["cat-file", "-e", &format!("{}^{{commit}}", main.trim())])
            .current_dir(root)
            .output()
            .unwrap()
            .status
            .success()
    }
}

/// Every `*.lock` file under `dir`.
fn lock_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lock") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn a_brain_is_fetched_at_most_once_a_minute() {
    let (h, root) = team();
    h.read_json(Read::WakeUp, &root);
    teammate_pushes_zebra(&h, "zebra");
    for read in [Read::WakeUp, Read::Search] {
        let (_, doc, _) = h.read_json(read, &root);
        assert_eq!(
            keys(read.entry(&doc)),
            read.quiet_keys(),
            "fetched seconds ago"
        );
    }
    assert!(!h.has_remote_main(&root));
    // Well inside the minute: kluris's clock runs a little after the test's.
    stamp_age(&root, 50);
    h.read_json(Read::WakeUp, &root);
    assert!(!h.has_remote_main(&root));
    stamp_age(&root, 61);
    let (_, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(sync_of(&doc)["pulled"], 1);
    teammate_pushes_zebra(&h, "okapi");
    stamp_age(&root, -3600);
    let (_, doc, _) = h.read_json(Read::Search, &root);
    assert_eq!(
        sync_of(&doc)["pulled"],
        1,
        "a stamp from the future does not throttle"
    );
}

#[test]
fn a_fetch_stopped_at_3_seconds_leaves_no_lock_and_backs_off() {
    // An upload-pack that starts late keeps the fetch past the 3 s.
    let (h, root) = team();
    teammate_pushes_zebra(&h, "zebra");
    let calls = h.dir.path().join("upload-pack-calls");
    h.upload_pack(
        &root,
        "slow-upload-pack.sh",
        &format!(
            "echo call >> '{}'\nsleep 10\nexec git upload-pack \"$@\"\n",
            sh_path(&calls)
        ),
    );
    let started = Instant::now();
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    assert!(took < Duration::from_secs(6), "{took:?}");
    assert_eq!(reason(sync_of(&doc)), "offline", "{doc}");
    assert_eq!(lock_files(&root.join(".git")), Vec::<PathBuf>::new());
    let backoff = git_file(&root, "kluris-sync/fetch-backoff");
    assert!(backoff.is_file());
    // Inside the back-off the reads do not fetch, and say nothing.
    for read in [Read::Search, Read::WakeUp] {
        stamp_age(&root, 61);
        let started = Instant::now();
        let (_, doc, _) = h.read_json(read, &root);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(keys(read.entry(&doc)), read.quiet_keys(), "{read:?}: {doc}");
    }
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 1);
    // A sync that fetches ends the back-off; the next read fetches.
    h.git(&root, &["config", "--unset", "remote.origin.uploadpack"]);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["brains"][0]["sync"]["pulled"], 1, "{doc}");
    assert!(!backoff.exists(), "sync did not end the back-off");
    teammate_pushes_zebra(&h, "okapi");
    stamp_age(&root, 61);
    let (_, doc, _) = h.read_json(Read::Search, &root);
    assert_eq!(sync_of(&doc)["pulled"], 1, "{doc}");
}

/// A local server that accepts connections and never answers.
fn silent_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    port
}

/// A local HTTP server that answers every request with 401 and a Basic
/// challenge, as a host asking for credentials does.
fn auth_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"kluris\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    port
}

#[test]
fn a_remote_that_hangs_is_given_up_after_3_seconds() {
    let (h, root) = team();
    let port = silent_server();
    let url = format!("git://127.0.0.1:{port}/alpha.git");
    h.git(&root, &["remote", "set-url", "origin", &url]);
    let started = Instant::now();
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    assert!(took < Duration::from_secs(6), "{took:?}");
    assert_eq!(reason(sync_of(&doc)), "offline", "{doc}");
    // The back-off: the next read does not wait again.
    stamp_age(&root, 61);
    let started = Instant::now();
    h.read_json(Read::Search, &root);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn a_stopped_fetch_leaves_no_process_behind() {
    let (h, root) = team();
    let pids = h.dir.path().join("pids");
    h.upload_pack(
        &root,
        "hang.sh",
        &format!(
            "echo $$ >> '{0}'\nsleep 30 &\necho $! >> '{0}'\nwait\n",
            sh_path(&pids)
        ),
    );
    let started = Instant::now();
    h.read_json(Read::WakeUp, &root);
    assert!(started.elapsed() < Duration::from_secs(6));
    let text = fs::read_to_string(&pids).unwrap();
    let pids: Vec<&str> = text.lines().collect();
    assert_eq!(pids.len(), 2, "{text}");
    let alive = |pid: &str| {
        StdCommand::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while pids.iter().any(|p| alive(p)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    for pid in &pids {
        if alive(pid) {
            let _ = StdCommand::new("kill").args(["-9", pid]).status();
            panic!("process {pid} outlived kluris");
        }
    }
}

#[test]
fn a_read_killed_while_it_fetches_never_blocks_the_next() {
    // The remote claim is an OS lock: it goes with the process.
    let (h, root) = team();
    teammate_pushes_zebra(&h, "zebra");
    let pids = h.dir.path().join("pids");
    h.upload_pack(
        &root,
        "hang.sh",
        &format!(
            "echo $$ >> '{0}'\nsleep 30 &\necho $! >> '{0}'\nwait\n",
            sh_path(&pids)
        ),
    );
    let mut read = h.spawn(&["wake-up", "--brain", "alpha", "--json"]);
    eventually("the read fetches", || {
        fs::read_to_string(&pids).map_or(0, |t| t.lines().count()) >= 2
    });
    // SIGKILL on Unix, TerminateProcess on Windows: no cleanup runs.
    read.kill().unwrap();
    read.wait().unwrap();
    // Unix: the fetch's own process group outlives kluris; end it.
    #[cfg(unix)]
    for pid in fs::read_to_string(&pids).unwrap().lines() {
        let _ = StdCommand::new("kill").args(["-9", pid]).status();
    }
    h.git(&root, &["config", "--unset", "remote.origin.uploadpack"]);
    stamp_age(&root, 61);
    let (_, doc, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(sync_of(&doc)["pulled"], 1, "{doc}");
}

#[test]
fn two_reads_at_once_fetch_once_per_back_off_window() {
    // Agents starting together over a link too slow for the 3 s.
    let (h, root) = team();
    let calls = h.dir.path().join("upload-pack-calls");
    h.upload_pack(
        &root,
        "slow-upload-pack.sh",
        &format!(
            "echo call >> '{}'\nsleep 10\nexec git upload-pack \"$@\"\n",
            sh_path(&calls)
        ),
    );
    teammate_pushes_zebra(&h, "zebra");
    let backoff = git_file(&root, "kluris-sync/fetch-backoff");
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    for (round, window) in [3600u64, 7200, 14400].into_iter().enumerate() {
        if round > 0 {
            // The last window has run out (as an hour later).
            fs::write(&backoff, format!("{} {}\n", now() - 1, window / 2)).unwrap();
        }
        stamp_age(&root, 61);
        let started = Instant::now();
        let mut reads = [
            h.spawn(&["wake-up", "--brain", "alpha", "--json"]),
            h.spawn(&["search", "zebras", "--brain", "alpha", "--json"]),
        ];
        for read in &mut reads {
            assert!(read.wait().unwrap().success());
        }
        assert!(started.elapsed() < Duration::from_secs(8));
        let attempts = fs::read_to_string(&calls).unwrap().lines().count();
        assert_eq!(attempts, round + 1, "one fetch per window");
        let state = fs::read_to_string(&backoff).unwrap();
        let mut fields = state.split_whitespace();
        let until: u64 = fields.next().unwrap().parse().unwrap();
        assert_eq!(fields.next(), Some(window.to_string().as_str()), "{state}");
        assert!(
            until >= now() + window - 30 && until <= now() + window,
            "{state}"
        );
    }
    assert!(!root.join("notes").join("zebra.md").exists());
}

#[test]
fn a_credential_prompt_never_blocks() {
    let (h, root) = team();
    let port = auth_server();
    let url = format!("http://127.0.0.1:{port}/alpha.git");
    h.git(&root, &["remote", "set-url", "origin", &url]);
    let started = Instant::now();
    let (code, doc, _) = h.read_json(Read::WakeUp, &root);
    let took = started.elapsed();
    assert_eq!(code, 0);
    assert_eq!(reason(sync_of(&doc)), "refused", "{doc}");
    assert!(
        took < Duration::from_millis(2500),
        "refused at once, not cut off at 3 s: {took:?}"
    );
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn an_askpass_helper_is_never_asked() {
    // Editors set GIT_ASKPASS to a helper that shows a dialog; one that
    // hangs here must not be run.
    let (mut h, root) = team();
    let port = auth_server();
    let url = format!("http://127.0.0.1:{port}/alpha.git");
    h.git(&root, &["remote", "set-url", "origin", &url]);
    let asked = h.dir.path().join("asked");
    let askpass = h.dir.path().join("askpass.sh");
    fs::write(
        &askpass,
        format!("#!/bin/sh\ntouch '{}'\nsleep 30\n", asked.display()),
    )
    .unwrap();
    make_executable(&askpass);
    let helper = askpass.display().to_string();
    h.env.push(("GIT_ASKPASS".into(), helper.clone()));
    h.env.push(("SSH_ASKPASS".into(), helper));
    let started = Instant::now();
    let (code, _, _) = h.read_json(Read::WakeUp, &root);
    assert_eq!(code, 0);
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert!(!asked.exists(), "the askpass helper ran");
}

#[cfg(unix)]
#[test]
fn ssh_runs_in_batch_mode_unless_the_user_chose_an_ssh_command() {
    let (mut h, root) = team();
    let bin = h.dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = h.dir.path().join("ssh.log");
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nexit 255\n",
            log.display()
        ),
    )
    .unwrap();
    make_executable(&ssh);
    h.git(
        &root,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.invalid/alpha.git",
        ],
    );
    let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let base = h.env.clone();
    let run = |h: &mut Home, extra: &[(&str, String)]| {
        let _ = fs::remove_file(&log);
        stamp_age(&root, 61);
        h.env = base.clone();
        h.env
            .push(("PATH".into(), path.to_string_lossy().into_owned()));
        for (k, v) in extra {
            h.env.push(((*k).into(), v.clone()));
        }
        let (code, _, _) = h.read_json(Read::WakeUp, &root);
        assert_eq!(code, 0);
        fs::read_to_string(&log).unwrap_or_default()
    };
    let logged = run(&mut h, &[]);
    assert!(logged.contains("BatchMode=yes"), "{logged}");
    // The user's own GIT_SSH_COMMAND is used as it is.
    let logged = run(
        &mut h,
        &[("GIT_SSH_COMMAND", format!("'{}' -o User=me", ssh.display()))],
    );
    assert!(logged.contains("User=me"), "{logged}");
    assert!(!logged.contains("BatchMode"), "{logged}");
    // So is core.sshCommand.
    let command = format!("'{}' -o User=cfg", ssh.display());
    h.git(&root, &["config", "core.sshCommand", &command]);
    let logged = run(&mut h, &[]);
    assert!(logged.contains("User=cfg"), "{logged}");
    assert!(!logged.contains("BatchMode"), "{logged}");
}
