//! The session every command that opens a brain syncs through
//! (`kluris_core::sync::session`), with real git: a bare remote and clones
//! in temp folders.
//!
//! - A read (and housekeeping) fetches for 3 s at most, once a minute per
//!   brain, never inside the back-off, and lands only what its own fetch
//!   named; it holds the shared tree claim until it is closed.
//! - Every git a read runs before the land, its merge hooks and the checks
//!   the land makes before it writes included, stops at the read's 3 s:
//!   the brain is read as it is (`busy`). A git that fails on its own is
//!   `failed`, whatever it says, and the land itself is never stopped.
//! - An explicit session always fetches, holds the tree claim until it is
//!   closed, and pushes then.
//! - `open_all` fetches eight brains at a time at most, then lands them one
//!   by one in their order.
//! - A brain with no git, no upstream, nested in a larger repository or
//!   shallow is left alone with no report; an explicit session reports
//!   `no_upstream`.
//! - A read that lands nothing reports the merge that waits for a person,
//!   unless the branch has moved on from it.
//! - Nothing turns a session's network off, and a rewrite of `kluris.yml`
//!   keeps its `sync:` key as written.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{ISOLATED_VARS, SERIAL, World, detail, neuron, options, reason, wait_bounded};
use kluris_core::config::{parse_brain_config, read_brain_config, write_brain_config};
use kluris_core::git::SystemGit;
use kluris_core::sync::claim::{COMMAND_BOUND, READ_BOUND, RemoteClaim, TreeClaim};
use kluris_core::sync::engine;
use kluris_core::sync::fetch::{BACKOFF, STAMP, back_off};
use kluris_core::sync::session::{self, Alone, Brain, Intent, READ_TIME_LIMIT, SessionEnv};
use kluris_core::sync::{NETWORK_TIME_LIMIT, SyncReport, SyncRequest};
use tempfile::TempDir;

/// The longest a test waits for another process to do something.
const PATIENCE: Duration = Duration::from_secs(60);

/// A read's bound with room for a slow machine, never the time a stalled
/// step would take.
const READ_LIMIT: Duration = Duration::from_millis(4500);

/// The intents that read: they share the throttle and the 3 s bound.
const READS: [Intent; 2] = [Intent::Read, Intent::Housekeeping];

/// The clock a session compares with its throttle stamp: `secs` past a
/// fixed instant.
fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_000_000_000 + secs)
}

fn env(w: &World, now: SystemTime) -> SessionEnv {
    SessionEnv {
        git: w.git(),
        no_verify: false,
        now,
        program: None,
    }
}

fn brain(root: &Path) -> Brain<'_> {
    Brain {
        root,
        maps: options(),
    }
}

/// Open the brain at `root` with `intent` at `now`, then close it: the
/// report the command would print.
fn session_at(w: &World, root: &Path, intent: Intent, now: SystemTime) -> Option<SyncReport> {
    session::close(session::open(brain(root), intent, &env(w, now)))
}

/// The teammate pushes a new neuron, `rel`.
fn teammate_pushes(w: &World, rel: &str) {
    w.theirs_push(&[(rel, &neuron("", "# New\n"))], &[]);
}

/// What "ours" has at `refname`.
fn ref_at(w: &World, at: &Path, refname: &str) -> String {
    w.run(at, &["rev-parse", refname]).trim().to_string()
}

/// Polls `done` until it holds, at most [`PATIENCE`].
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < PATIENCE,
            "{what}: not within {PATIENCE:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// A path as git's shell takes it (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Run `script` (a shell script's body) for every upload-pack the clone
/// at `at` starts from `origin`: every `ls-remote` and fetch.
fn upload_pack(w: &World, at: &Path, name: &str, script: &str) {
    let path = w.dir.path().join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
    let pack = format!("sh '{}'", sh_path(&path));
    w.run(at, &["config", "remote.origin.uploadpack", &pack]);
}

/// Count every upload-pack the clone at `at` starts from `origin` (each
/// `ls-remote` and each fetch starts one): their count is
/// [`upload_packs`] of the file this returns.
fn count_upload_packs(w: &World, at: &Path) -> PathBuf {
    let log = w.dir.path().join("upload-packs.log");
    let script = format!(
        "echo run >> '{}'\nexec git upload-pack \"$@\"\n",
        sh_path(&log)
    );
    upload_pack(w, at, "counting-upload-pack.sh", &script);
    log
}

/// How many upload-packs the log of [`count_upload_packs`] holds.
fn upload_packs(log: &Path) -> usize {
    fs::read_to_string(log).map_or(0, |text| text.lines().count())
}

/// The remote's `file://` URL.
fn file_url(w: &World) -> String {
    let remote = sh_path(&w.remote);
    if remote.starts_with('/') {
        format!("file://{remote}")
    } else {
        format!("file:///{remote}")
    }
}

#[test]
fn a_read_fetches_once_a_minute_per_brain() {
    for intent in READS {
        let w = World::new();
        let log = count_upload_packs(&w, &w.ours);
        teammate_pushes(&w, "ideas/first-new.md");
        let report = session_at(&w, &w.ours, intent, at(0)).expect("a report");
        assert_eq!(report.pulled, 1, "{intent:?}: {report:?}");
        assert_eq!(w.head(&w.ours), w.remote_head(), "{intent:?}");
        // The stamp is the remote's, in the common git directory.
        assert!(w.ours.join(".git").join(STAMP).is_file(), "{intent:?}");
        let fetched = upload_packs(&log);
        assert!(fetched > 0, "{intent:?}: no upload-pack ran");

        teammate_pushes(&w, "ideas/second-new.md");
        let before = w.head(&w.ours);
        assert_eq!(session_at(&w, &w.ours, intent, at(30)), None, "{intent:?}");
        assert_eq!(
            upload_packs(&log),
            fetched,
            "{intent:?}: reached the remote inside the minute"
        );
        assert_eq!(
            w.head(&w.ours),
            before,
            "{intent:?}: landed inside the minute"
        );

        let report = session_at(&w, &w.ours, intent, at(61)).expect("a report");
        assert_eq!(report.pulled, 1, "{intent:?}: {report:?}");
        assert_eq!(w.head(&w.ours), w.remote_head(), "{intent:?}");
        assert!(upload_packs(&log) > fetched, "{intent:?}: no fetch past it");
    }
}

#[test]
fn an_explicit_session_always_fetches_and_stamps_the_throttle() {
    let w = World::new();
    for (n, rel) in ["ideas/first-new.md", "ideas/second-new.md"]
        .into_iter()
        .enumerate()
    {
        teammate_pushes(&w, rel);
        let report = session_at(&w, &w.ours, Intent::Explicit, at(0)).expect("a report");
        assert_eq!(report.pulled, 1, "fetch {n}: {report:?}");
        assert_eq!(w.head(&w.ours), w.remote_head(), "fetch {n}");
    }
    // Its fetch counts for the reads' minute.
    teammate_pushes(&w, "ideas/third-new.md");
    let before = w.head(&w.ours);
    assert_eq!(session_at(&w, &w.ours, Intent::Read, at(10)), None);
    assert_eq!(w.head(&w.ours), before);
}

#[test]
fn a_read_inside_the_back_off_does_not_fetch() {
    let w = World::new();
    let log = count_upload_packs(&w, &w.ours);
    back_off(&w.ours.join(".git").join(BACKOFF), at(0));
    teammate_pushes(&w, "ideas/new.md");
    let before = w.head(&w.ours);
    for intent in READS {
        assert_eq!(session_at(&w, &w.ours, intent, at(120)), None, "{intent:?}");
        assert_eq!(w.head(&w.ours), before, "{intent:?}");
        assert_eq!(upload_packs(&log), 0, "{intent:?}: reached the remote");
    }
    // An explicit session fetches through it.
    let report = session_at(&w, &w.ours, Intent::Explicit, at(120)).expect("a report");
    assert_eq!(report.pulled, 1, "{report:?}");
    assert!(upload_packs(&log) > 0, "no fetch");
}

#[test]
fn a_back_off_ends_after_its_window() {
    let w = World::new();
    back_off(&w.ours.join(".git").join(BACKOFF), at(0));
    teammate_pushes(&w, "ideas/new.md");
    let report = session_at(&w, &w.ours, Intent::Read, at(3601)).expect("a report");
    assert_eq!(report.pulled, 1, "{report:?}");
}

#[test]
fn a_fetch_stopped_at_the_reads_bound_is_offline_and_backs_off() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    upload_pack(&w, &w.ours, "stalled-upload-pack.sh", "sleep 30\n");
    let before = w.head(&w.ours);
    let started = Instant::now();
    let report = session_at(&w, &w.ours, Intent::Read, at(0)).expect("a report");
    let took = started.elapsed();
    assert!(took < READ_LIMIT, "{took:?}");
    assert_eq!(reason(&report), Some("offline"), "{report:?}");
    assert_eq!(w.head(&w.ours), before);
    assert!(w.ours.join(".git").join(BACKOFF).is_file(), "no back-off");
    // Past the minute, inside the back-off: no fetch, nothing to say.
    let started = Instant::now();
    assert_eq!(session_at(&w, &w.ours, Intent::Read, at(61)), None);
    assert!(started.elapsed() < Duration::from_secs(2), "it fetched");
}

#[test]
fn a_fetch_through_one_worktree_throttles_a_read_through_another() {
    let w = World::new();
    let log = count_upload_packs(&w, &w.ours);
    let linked = w.dir.path().join("linked");
    let to = linked.to_string_lossy().into_owned();
    w.run(
        &w.ours,
        &["worktree", "add", "-q", "-b", "other", &to, "origin/main"],
    );
    assert_eq!(
        w.run(&linked, &["rev-parse", "--abbrev-ref", "other@{upstream}"])
            .trim(),
        "origin/main"
    );
    teammate_pushes(&w, "ideas/first-new.md");
    let report = session_at(&w, &w.ours, Intent::Read, at(0)).expect("a report");
    assert_eq!(report.pulled, 1, "{report:?}");

    let fetched = upload_packs(&log);
    assert!(fetched > 0, "no upload-pack ran");
    let before = w.head(&linked);
    teammate_pushes(&w, "ideas/second-new.md");
    assert_eq!(session_at(&w, &linked, Intent::Read, at(10)), None);
    assert_eq!(upload_packs(&log), fetched, "the linked worktree fetched");
    assert_eq!(w.head(&linked), before, "the linked worktree landed");
    let own = PathBuf::from(w.run(&linked, &["rev-parse", "--absolute-git-dir"]).trim());
    assert!(!own.join(STAMP).exists(), "a stamp of the worktree's own");

    let report = session_at(&w, &linked, Intent::Read, at(61)).expect("a report");
    assert!(report.pulled > 0, "{report:?}");
    assert_eq!(w.head(&linked), w.remote_head());
    assert!(upload_packs(&log) > fetched, "no fetch past the minute");
}

#[test]
fn a_held_remote_claim_makes_a_session_land_nothing_and_report_busy() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    // The shared tracking ref holds the newer commit; it is never taken.
    w.run(&w.ours, &["fetch", "-q"]);
    assert_eq!(
        ref_at(&w, &w.ours, "refs/remotes/origin/main"),
        w.remote_head()
    );
    let before = w.head(&w.ours);
    let held = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    for intent in [Intent::Read, Intent::Housekeeping, Intent::Explicit] {
        let report = session_at(&w, &w.ours, intent, at(0)).expect("a report");
        assert_eq!(reason(&report), Some("busy"), "{intent:?}: {report:?}");
        assert_eq!(report.pulled, 0, "{intent:?}");
        assert_eq!(w.head(&w.ours), before, "{intent:?}");
    }
    drop(held);
}

#[test]
fn a_throttled_read_lands_nothing_and_reports_nothing_with_a_newer_tracking_ref() {
    let w = World::new();
    let log = count_upload_packs(&w, &w.ours);
    assert_eq!(session_at(&w, &w.ours, Intent::Read, at(0)), None);
    teammate_pushes(&w, "ideas/new.md");
    w.run(&w.ours, &["fetch", "-q"]);
    let fetched = upload_packs(&log);
    let before = w.head(&w.ours);
    for intent in READS {
        assert_eq!(session_at(&w, &w.ours, intent, at(30)), None, "{intent:?}");
        assert_eq!(w.head(&w.ours), before, "{intent:?}");
        assert_eq!(
            upload_packs(&log),
            fetched,
            "{intent:?}: reached the remote"
        );
    }
}

/// A git that runs `action` (a shell script's lines) before the step whose
/// arguments hold `step`, and runs every step.
#[cfg(unix)]
fn git_that(w: &World, step: &str, action: &str) -> SystemGit {
    use std::os::unix::fs::PermissionsExt;
    let script = w.dir.path().join("wrapped-git");
    fs::write(
        &script,
        format!("#!/bin/sh\ncase \" $* \" in\n *'{step}'*)\n{action}\n;;\nesac\nexec git \"$@\"\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    w.git().program(&script)
}

/// Ten seconds of a step that never ends within a read.
const STALL: &str = "n=0; while [ $n -lt 1000 ]; do sleep 0.01; n=$((n+1)); done";

/// What a git that fails on its own prints: the words of a stop.
#[cfg(unix)]
const FAILS_WITH_STOP_WORDS: &str =
    "echo 'the gate hook refused: `git status` was stopped at its time limit' >&2; exit 1";

/// Where "ours" stands against the remote when a read opens it.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// The remote is ahead: the read fast-forwards.
    Behind,
    /// Both moved: the read merges.
    Diverged,
}

impl Shape {
    fn make(self, w: &World) {
        match self {
            Shape::Behind => teammate_pushes(w, "ideas/new.md"),
            Shape::Diverged => diverged(w),
        }
    }
}

/// A read of "ours", shaped as `shape`, with `git` for its gits: what it
/// reported and how long it took, once the brain is as it was before (HEAD,
/// the index, the status and every file) and the tree claim is free.
fn read_with(w: &World, shape: Shape, git: SystemGit, what: &str) -> (SyncReport, Duration) {
    shape.make(w);
    let before = w.snapshot();
    let env = SessionEnv {
        git,
        no_verify: false,
        now: at(0),
        program: None,
    };
    let started = Instant::now();
    let report = session::close(session::open(brain(&w.ours), Intent::Read, &env))
        .unwrap_or_else(|| panic!("{what}: no report"));
    let took = started.elapsed();
    let after = w.snapshot();
    assert_eq!(
        (&after.0, &after.1, &after.2, &after.3),
        (&before.0, &before.1, &before.2, &before.3),
        "{what}: the brain changed: {report:?}"
    );
    let own = w.ours.join(".git");
    assert!(
        TreeClaim::exclusive(&own, Duration::ZERO).is_ok(),
        "{what}: the tree claim is held"
    );
    (report, took)
}

/// The steps a read runs before the land, each in the shape that runs it:
/// the brain's discovery, the upstream query, a pull's local step, and the
/// checks the land makes before it writes (the work tree's status and the
/// branch's lock, on a fast-forward and on a merge).
#[cfg(unix)]
const BEFORE_THE_LAND: [(&str, Shape); 6] = [
    (
        " rev-parse --is-inside-work-tree --show-prefix ",
        Shape::Behind,
    ),
    (
        " for-each-ref --format=%(upstream:remotename)",
        Shape::Diverged,
    ),
    (" merge-base ", Shape::Diverged),
    (" status ", Shape::Behind),
    (" --git-path refs/heads/main.lock ", Shape::Behind),
    (" --git-path refs/heads/main.lock ", Shape::Diverged),
];

/// Every git a read runs before the land stops at its 3 s bound: the
/// brain is read as it is (`busy`).
#[cfg(unix)]
#[test]
fn a_stalled_step_before_the_land_keeps_a_read_within_its_bound() {
    for (step, shape) in BEFORE_THE_LAND {
        let w = World::new();
        let what = format!("{step} ({shape:?})");
        let (report, took) = read_with(&w, shape, git_that(&w, step, STALL), &what);
        assert!(took < READ_LIMIT, "{what}: {took:?}");
        assert_eq!(reason(&report), Some("busy"), "{what}: {report:?}");
    }
}

/// A git that fails at once is `failed`, even when what it says holds the
/// words of a stop: before the land and in it.
#[cfg(unix)]
#[test]
fn a_git_that_fails_on_its_own_is_failed_whatever_it_says() {
    for (step, shape) in [
        (
            " for-each-ref --format=%(upstream:remotename)",
            Shape::Diverged,
        ),
        (" merge --ff-only ", Shape::Behind),
    ] {
        let w = World::new();
        let what = format!("{step} ({shape:?})");
        let git = git_that(&w, step, FAILS_WITH_STOP_WORDS);
        let (report, _) = read_with(&w, shape, git, &what);
        assert_eq!(reason(&report), Some("failed"), "{what}: {report:?}");
        assert!(
            detail(&report).contains("the gate hook refused"),
            "{what}: {report:?}"
        );
    }
}

/// A merge hook still running at the read's 3 s is stopped, with every
/// process it started: nothing is merged or landed, and it writes nothing
/// after that.
#[test]
fn a_stalled_merge_hook_stops_at_the_reads_bound() {
    let w = World::new();
    let pid = w.dir.path().join("hook.pid");
    let late = w.dir.path().join("hook.late");
    w.hook(
        &w.ours,
        "pre-merge-commit",
        &format!(
            "echo $$ > '{}'\n{STALL}\necho late > '{}'",
            sh_path(&pid),
            sh_path(&late)
        ),
    );
    let (report, took) = read_with(&w, Shape::Diverged, w.git(), "pre-merge-commit");
    assert!(took < READ_LIMIT, "{took:?}");
    assert_eq!(reason(&report), Some("busy"), "{report:?}");
    let pid = fs::read_to_string(&pid).expect("the hook ran");
    eventually("the hook's end", || !alive(&w, pid.trim()));
    assert!(!late.exists(), "the hook went on past the bound");
}

/// Whether the process `pid` (as the hooks' shell numbers it) runs.
fn alive(w: &World, pid: &str) -> bool {
    let alias = format!("alias.alive=!kill -0 {pid} 2>/dev/null");
    w.command(&w.ours, &["-c", &alias, "alive"])
        .status()
        .unwrap()
        .success()
}

/// The land is never stopped: a `post-merge` hook that takes longer than
/// the read's 3 s runs to its end, and the read lands.
#[test]
fn a_hook_of_the_land_runs_to_its_end() {
    let w = World::new();
    let done = w.dir.path().join("post-merge.done");
    w.hook(
        &w.ours,
        "post-merge",
        &format!("sleep 4\necho done > '{}'", sh_path(&done)),
    );
    Shape::Behind.make(&w);
    let started = Instant::now();
    let report = session_at(&w, &w.ours, Intent::Read, at(0)).expect("a report");
    assert!(started.elapsed() > READ_TIME_LIMIT, "the hook did not run");
    assert_eq!(report.pulled, 1, "{report:?}");
    assert_eq!(report.left_as_is, None, "{report:?}");
    assert!(done.exists(), "the hook was stopped");
    assert_eq!(w.head(&w.ours), w.remote_head());
}

/// A teammate's commit and one of ours on different files.
fn diverged(w: &World) {
    teammate_pushes(w, "ideas/new.md");
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
}

#[test]
fn a_tree_claim_held_past_the_bound_leaves_a_read_busy_and_the_brain_as_it_is() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    let before = w.snapshot();
    let held = TreeClaim::exclusive(&w.ours.join(".git"), Duration::ZERO).unwrap();
    let started = Instant::now();
    let report = session_at(&w, &w.ours, Intent::Read, at(0)).expect("a report");
    let took = started.elapsed();
    assert_eq!(reason(&report), Some("busy"), "{report:?}");
    assert!(took >= READ_BOUND, "did not wait its bound: {took:?}");
    assert!(took < READ_BOUND + READ_LIMIT, "{took:?}");
    let after = w.snapshot();
    assert_eq!(
        (&after.0, &after.1, &after.3),
        (&before.0, &before.1, &before.3)
    );
    drop(held);
    let report = session_at(&w, &w.ours, Intent::Read, at(61)).expect("a report");
    assert_eq!(report.pulled, 1, "{report:?}");
}

#[test]
fn a_read_holds_the_shared_tree_claim_until_it_is_closed() {
    let w = World::new();
    let own = w.ours.join(".git");
    let reading = session::open(brain(&w.ours), Intent::Read, &env(&w, at(0)));
    assert!(TreeClaim::exclusive(&own, Duration::ZERO).is_err());
    drop(TreeClaim::shared(&own, Duration::ZERO).expect("readers share it"));

    // A land from another process, which says when it takes the tree
    // claim, waits for the read.
    teammate_pushes(&w, "ideas/new.md");
    let before = w.head(&w.ours);
    let out = w.dir.path().join("land.out");
    let mut lander = child(&w, "land", &out);
    eventually("the lander at the tree claim", || {
        assert_running(&mut lander);
        claiming(&out).exists()
    });
    assert_eq!(w.head(&w.ours), before, "landed while the read was open");
    let closed = millis(SystemTime::now());
    assert_eq!(session::close(reading), None);
    assert!(wait_bounded(&mut lander, "the lander", PATIENCE).success());
    let (pulled, landed) = child_result(&out);
    assert_eq!(pulled, 1);
    assert!(landed >= closed, "landed at {landed}, closed at {closed}");
    assert_eq!(w.head(&w.ours), w.remote_head());
}

#[test]
fn an_explicit_session_holds_the_tree_claim_and_pushes_when_closed() {
    let w = World::new();
    let own = w.ours.join(".git");
    teammate_pushes(&w, "ideas/first-new.md");
    let open = session::open(brain(&w.ours), Intent::Explicit, &env(&w, at(0)));
    assert_eq!(open.report().map(|r| r.pulled), Some(1));
    assert!(TreeClaim::shared(&own, Duration::ZERO).is_err());

    // The command commits; meanwhile the remote moves.
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    teammate_pushes(&w, "ideas/second-new.md");
    let started = Instant::now();
    let report = session::close(open).expect("a report");
    assert!(
        started.elapsed() < COMMAND_BOUND,
        "waited for its own claim"
    );
    assert!(report.pushed, "{report:?}");
    assert_eq!(report.to_push, 0, "{report:?}");
    assert_eq!(report.left_as_is, None, "{report:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
    assert_eq!(w.parents(&w.ours, "HEAD"), 2, "a merge of the moved remote");
    drop(TreeClaim::exclusive(&own, Duration::ZERO).expect("released"));
}

#[test]
fn a_read_counts_the_commits_waiting_when_it_is_closed() {
    for intent in READS {
        let w = World::new();
        w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
        w.ours_commit(&[("notes/third.md", &neuron("", "# Third\n"))], &[]);
        let remote = w.remote_head();
        for now in [at(0), at(10)] {
            let report = session_at(&w, &w.ours, intent, now).expect("a report");
            assert_eq!(report.to_push, 2, "{intent:?}: {report:?}");
            assert!(!report.pushed, "{intent:?}");
            assert_eq!(w.remote_head(), remote, "{intent:?}: pushed");
        }
    }
}

/// `count` brains, each a bare remote and a clone with an upstream, the
/// remote holding `index % 3 + 1` commits the clone has not; each clone's
/// `post-merge` hook writes its name into `lands`, and every upload-pack
/// of theirs goes through `upload-pack.sh`.
struct Brains {
    _serial: std::sync::MutexGuard<'static, ()>,
    dir: TempDir,
    roots: Vec<PathBuf>,
}

impl Brains {
    fn new(count: usize, upload_pack_script: &str) -> Brains {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        let script = dir.path().join("upload-pack.sh");
        fs::write(&script, format!("#!/bin/sh\n{upload_pack_script}")).unwrap();
        let mut brains = Brains {
            _serial: serial,
            dir,
            roots: Vec::new(),
        };
        let lands = sh_path(&brains.lands());
        for index in 0..count {
            let name = format!("brain{index}");
            let remote = brains.dir.path().join(format!("{name}.git"));
            let root = brains.dir.path().join(&name);
            let teammate = brains.dir.path().join(format!("{name}-theirs"));
            let top = brains.dir.path().to_path_buf();
            brains.git(
                &top,
                &["init", "-q", "--bare", "-b", "main", &path(&remote)],
            );
            fs::create_dir_all(&root).unwrap();
            brains.git(&root, &["init", "-q", "-b", "main"]);
            fs::write(root.join("brain.md"), "# Brain\n").unwrap();
            brains.git(&root, &["add", "-A"]);
            brains.git(&root, &["commit", "-q", "-m", "init"]);
            brains.git(&root, &["remote", "add", "origin", &path(&remote)]);
            brains.git(&root, &["push", "-q", "-u", "origin", "main"]);
            brains.git(&top, &["clone", "-q", &path(&remote), &path(&teammate)]);
            for n in 0..=index % 3 {
                fs::write(teammate.join(format!("note{n}.md")), "# Note\n").unwrap();
                brains.git(&teammate, &["add", "-A"]);
                brains.git(&teammate, &["commit", "-q", "-m", "note"]);
            }
            brains.git(&teammate, &["push", "-q"]);
            let hook = root.join(".git").join("hooks").join("post-merge");
            fs::create_dir_all(hook.parent().unwrap()).unwrap();
            fs::write(&hook, format!("#!/bin/sh\necho {name} >> '{lands}'\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let pack = format!("sh '{}'", sh_path(&script));
            brains.git(&root, &["config", "remote.origin.uploadpack", &pack]);
            brains.roots.push(root);
        }
        brains
    }

    fn lands(&self) -> PathBuf {
        self.dir.path().join("lands.log")
    }

    fn git(&self, cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path())
            .env("GIT_AUTHOR_NAME", "Ana Tester")
            .env("GIT_AUTHOR_EMAIL", "ana@example.com")
            .env("GIT_COMMITTER_NAME", "Ana Tester")
            .env("GIT_COMMITTER_EMAIL", "ana@example.com");
        for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
            cmd.env_remove(var);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn env(&self) -> SessionEnv {
        let mut git = SystemGit::new()
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path())
            .env("GIT_AUTHOR_NAME", "Ana Tester")
            .env("GIT_AUTHOR_EMAIL", "ana@example.com")
            .env("GIT_COMMITTER_NAME", "Ana Tester")
            .env("GIT_COMMITTER_EMAIL", "ana@example.com");
        for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
            git = git.env_remove(var);
        }
        SessionEnv {
            git,
            no_verify: false,
            now: at(0),
            program: None,
        }
    }
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn open_all_fetches_eight_brains_at_a_time_and_lands_them_in_order() {
    let tmp = TempDir::new().unwrap();
    let active = tmp.path().join("active");
    fs::create_dir_all(&active).unwrap();
    let counts = tmp.path().join("counts.log");
    // A brain's first upload-pack (its `ls-remote`; git starts it in the
    // brain's folder, whose name keys it) marks itself active, then holds
    // until eight are, or every brain has started (1.5 s at most), and
    // writes how many it saw, so fetches that overlap are seen overlapping.
    let script = format!(
        "d='{active}'\nlog='{counts}'\nseen=\"$d.seen-${{PWD##*/}}\"\n[ -e \"$seen\" ] && exec git upload-pack \"$@\"\n: > \"$seen\"\ncount() {{ set -- \"$d\"/*; n=$#; set -- \"$d\".seen-*; s=$#; }}\n: > \"$d/$$\"\ncount; echo \"$n\" >> \"$log\"\ni=0\nwhile [ $i -lt 30 ]; do count; {{ [ \"$n\" -ge 8 ] || [ \"$s\" -ge 10 ]; }} && break; sleep 0.05; i=$((i+1)); done\ncount; echo \"$n\" >> \"$log\"\nrm -f \"$d/$$\"\nexec git upload-pack \"$@\"\n",
        active = sh_path(&active),
        counts = sh_path(&counts),
    );
    let brains = Brains::new(10, &script);
    let list: Vec<Brain> = brains.roots.iter().map(|root| brain(root)).collect();
    // Explicit sessions, whose 60 s fetch bound leaves room for the hold.
    let sessions = session::open_all(&list, Intent::Explicit, &brains.env());
    let reports: Vec<Option<&SyncReport>> = sessions.iter().map(|s| s.report()).collect();
    let pulled: Vec<usize> = reports.iter().map(|r| r.map_or(0, |r| r.pulled)).collect();
    let expected: Vec<usize> = (0..10).map(|index| index % 3 + 1).collect();
    assert_eq!(
        pulled, expected,
        "reports in the brains' order: {reports:?}"
    );
    for session in sessions {
        let report = session::close(session).expect("a report");
        assert_eq!(report.left_as_is, None, "{report:?}");
    }
    let lands = fs::read_to_string(brains.lands()).unwrap();
    let order: Vec<&str> = lands.lines().collect();
    let names: Vec<String> = (0..10).map(|index| format!("brain{index}")).collect();
    assert_eq!(order, names, "lands in the brains' order");
    let seen: Vec<usize> = fs::read_to_string(&counts)
        .unwrap()
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    let most = seen.iter().copied().max().unwrap_or(0);
    assert!(most <= 8, "{most} fetching at once: {seen:?}");
    assert!(most > 1, "the fetches never overlapped: {seen:?}");
}

#[test]
fn open_all_of_no_brains_is_empty() {
    let w = World::new();
    assert!(session::open_all(&[], Intent::Read, &env(&w, at(0))).is_empty());
}

#[test]
fn brains_without_a_remote_to_sync_get_no_report() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    // No git.
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    // Nested in a larger repository.
    let nested = w.ours.join("notes");
    // Shallow.
    let shallow = w.dir.path().join("shallow");
    let url = file_url(&w);
    w.run(
        w.dir.path(),
        &["clone", "-q", "--depth", "1", &url, &path(&shallow)],
    );
    teammate_pushes(&w, "ideas/newer.md");
    let shallow_head = w.head(&shallow);
    let ours_head = w.head(&w.ours);
    for intent in [Intent::Read, Intent::Housekeeping, Intent::Explicit] {
        for root in [&plain, &nested, &shallow] {
            assert_eq!(
                session_at(&w, root, intent, at(0)),
                None,
                "{intent:?}: {}",
                root.display()
            );
        }
        assert_eq!(w.head(&shallow), shallow_head, "{intent:?}");
        assert_eq!(w.head(&w.ours), ours_head, "{intent:?}");
    }

    // No upstream: only an explicit session says so.
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    for intent in READS {
        assert_eq!(session_at(&w, &w.ours, intent, at(0)), None, "{intent:?}");
    }
    let report = session_at(&w, &w.ours, Intent::Explicit, at(0)).expect("a report");
    assert_eq!(reason(&report), Some("no_upstream"), "{report:?}");
    assert_eq!(detail(&report), "set one with git push -u origin main");
    assert_eq!(w.head(&w.ours), ours_head);
}

#[test]
fn a_session_says_why_it_left_a_brain_alone() {
    let w = World::new();
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let nested = w.ours.join("notes");
    let shallow = w.dir.path().join("shallow");
    let url = file_url(&w);
    w.run(
        w.dir.path(),
        &["clone", "-q", "--depth", "1", &url, &path(&shallow)],
    );
    let local = w.dir.path().join("local");
    fs::create_dir_all(&local).unwrap();
    w.run(&local, &["init", "-q", "-b", "main"]);
    w.configure(&local, "Ana Tester", "ana@example.com");
    w.write(&local, "brain.md", "# local\n");
    w.run(&local, &["add", "brain.md"]);
    w.run(&local, &["commit", "-q", "-m", "init"]);
    let unset = w.dir.path().join("unset");
    w.run(w.dir.path(), &["clone", "-q", &url, &path(&unset)]);
    w.run(&unset, &["branch", "--unset-upstream"]);
    let alone = |root: &Path| {
        let env = env(&w, at(0));
        let session = session::open(brain(root), Intent::Read, &env);
        let why = session.alone().cloned();
        assert_eq!(session::close(session), None, "{}", root.display());
        why
    };
    assert_eq!(alone(&plain), Some(Alone::NotARepo));
    assert_eq!(alone(&nested), Some(Alone::Nested));
    assert_eq!(alone(&shallow), Some(Alone::Shallow));
    assert_eq!(alone(&local), Some(Alone::NoRemote));
    let Some(Alone::NoUpstream(left)) = alone(&unset) else {
        panic!("no upstream: not said");
    };
    assert_eq!(left.reason.as_str(), "no_upstream");
    assert_eq!(
        left.detail.as_deref(),
        Some("set one with git push -u origin main")
    );
    assert_eq!(alone(&w.ours), None, "a brain it syncs");
}

#[test]
fn no_setting_turns_a_sessions_network_off() {
    for (n, settings) in [
        "sync:\n  auto_fetch: false\n",
        "sync:\n  auto: false\n",
        "sync: {auto_fetch: false, auto: false}\n",
    ]
    .into_iter()
    .enumerate()
    {
        let w = World::new();
        w.write(
            &w.ours,
            "kluris.yml",
            &format!("name: demo\ndescription: A demo brain\n{settings}"),
        );
        for (i, intent) in [Intent::Read, Intent::Housekeeping, Intent::Explicit]
            .into_iter()
            .enumerate()
        {
            teammate_pushes(&w, &format!("ideas/new-{n}-{i}.md"));
            let now = at(100 * i as u64);
            let report = session_at(&w, &w.ours, intent, now).expect("a report");
            assert_eq!(report.pulled, 1, "{settings}{intent:?}: {report:?}");
        }
    }
}

#[test]
fn no_environment_variable_turns_a_sessions_network_off() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    let out = w.dir.path().join("read.out");
    let mut reader = child(&w, "read", &out);
    assert!(wait_bounded(&mut reader, "the reader", PATIENCE).success());
    let (pulled, _) = child_result(&out);
    assert_eq!(pulled, 1);
    assert_eq!(w.head(&w.ours), w.remote_head());
}

/// The `kluris.yml` texts a rewrite keeps the `sync:` key of, each with
/// the entry as it must come out: as written, after the keys Kluris
/// writes.
const KEPT: [(&str, &str); 13] = [
    (
        "name: alpha\ndescription: ''\ncompanions: []\nsearch:\n  language: ro\nsync:\n  auto_fetch: false\n  other: 1\n",
        "sync:\n  auto_fetch: false\n  other: 1\n",
    ),
    (
        "name: alpha\nsync:\n  auto_fetch: off\n",
        "sync:\n  auto_fetch: off\n",
    ),
    (
        "name: alpha\nsync: {auto_fetch: no}\n",
        "sync: {auto_fetch: no}\n",
    ),
    (
        "name: alpha\nsync: {auto: false}  # paused\ncompanions: []\n",
        "sync: {auto: false}  # paused\n",
    ),
    (
        "name: alpha\nsync:\n  # for now\n  auto: false\n  peers: [a, b]\n\n\ndescription: d\n",
        "sync:\n  # for now\n  auto: false\n  peers: [a, b]\n",
    ),
    ("name: alpha\nsync: 5\n", "sync: 5\n"),
    ("name: alpha\nsync: {}\n", "sync: {}\n"),
    ("name: alpha\nsync:\n", "sync:\n"),
    (
        "name: alpha\nsync:\n  auto: false",
        "sync:\n  auto: false\n",
    ),
    (
        "name: alpha\n'sync': {auto: false}  # keep\ndescription: d\n",
        "'sync': {auto: false}  # keep\n",
    ),
    (
        "name: alpha\n\"sync\": {auto: false}  # keep\n",
        "\"sync\": {auto: false}  # keep\n",
    ),
    (
        "name: alpha\nsync:\n  auto: false\n# why it is off\n  peers: [a, b]\n",
        "sync:\n  auto: false\n# why it is off\n  peers: [a, b]\n",
    ),
    (
        "name: alpha\nsync:\n  auto: false\n# about the description\n\ndescription: d\n",
        "sync:\n  auto: false\n",
    ),
];

/// The top-level `sync` keys of a `kluris.yml` text, in any spelling.
fn sync_keys(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            ["sync:", "'sync':", "\"sync\":"]
                .iter()
                .any(|key| line.starts_with(key))
        })
        .count()
}

#[test]
fn a_rewrite_of_kluris_yml_keeps_its_sync_key_as_written() {
    let tmp = TempDir::new().unwrap();
    for (text, entry) in KEPT {
        fs::write(tmp.path().join("kluris.yml"), text).unwrap();
        let mut config = read_brain_config(tmp.path()).unwrap();
        config.companions = vec!["specmint-core".into()];
        write_brain_config(tmp.path(), &config).unwrap();
        let written = fs::read_to_string(tmp.path().join("kluris.yml")).unwrap();
        assert!(
            written.ends_with(&format!("- specmint-core\n{entry}"))
                || written.ends_with(&format!("  language: ro\n{entry}")),
            "{text:?} became {written:?}"
        );
        assert_eq!(sync_keys(&written), 1, "{written:?}");
        assert_eq!(parse_brain_config(&written).unwrap(), config, "{written:?}");
    }
}

/// The `kluris.yml` texts whose `sync:` entry does not read alone as its
/// value, each with the value as the rewrite writes it: an alias of an
/// anchor elsewhere in the file, which the rewrite does not keep; a `|+`
/// scalar that ends the file, whose last lines are its own; two `sync`
/// keys.
const VALUE_ONLY: [(&str, &str); 4] = [
    (
        "name: alpha\ndescription: &d team\nsync: *d\n",
        "sync: team\n",
    ),
    (
        "name: alpha\nsync: |+\n  team\n\n\n",
        "sync: 'team\n\n\n\n  '\n",
    ),
    (
        "name: alpha\nsync: {auto: true}  # first\nsync: {auto: false}  # second\n",
        "sync:\n  auto: false\n",
    ),
    (
        "name: alpha\nsync: {auto: false}  # first\nsync: {auto: false}  # second\n",
        "sync:\n  auto: false\n",
    ),
];

#[test]
fn a_sync_key_that_cannot_be_moved_as_written_keeps_its_value() {
    let tmp = TempDir::new().unwrap();
    for (text, entry) in VALUE_ONLY {
        fs::write(tmp.path().join("kluris.yml"), text).unwrap();
        let config = read_brain_config(tmp.path()).unwrap();
        write_brain_config(tmp.path(), &config).unwrap();
        let written = fs::read_to_string(tmp.path().join("kluris.yml")).unwrap();
        assert!(
            written.ends_with(&format!("companions: []\n{entry}")),
            "{text:?} became {written:?}"
        );
        assert_eq!(sync_keys(&written), 1, "{written:?}");
        assert_eq!(parse_brain_config(&written).unwrap(), config, "{written:?}");
    }
}

#[test]
fn a_kluris_yml_without_a_sync_key_is_written_from_its_values() {
    let tmp = TempDir::new().unwrap();
    let text = "name: alpha\ndescription: team\nagents:\n  commands_for: [codex, claude]\ncompanions: [specmint-core]\n";
    fs::write(tmp.path().join("kluris.yml"), text).unwrap();
    let config = read_brain_config(tmp.path()).unwrap();
    write_brain_config(tmp.path(), &config).unwrap();
    assert_eq!(
        fs::read_to_string(tmp.path().join("kluris.yml")).unwrap(),
        "name: alpha\ndescription: team\nagents:\n  commands_for:\n  - codex\n  - claude\ncompanions:\n- specmint-core\n"
    );
}

/// The overlapping edit of `notes/first.md` no rule merges: ours
/// committed, theirs pushed.
fn overlapping(w: &World) {
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("Three.", "Three, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
}

/// The waiting merge a read reports: one conflict at `notes/first.md`,
/// with both texts and the merge's id, and nothing pulled.
fn assert_waiting(report: &SyncReport, merge: &str, what: &str) {
    assert_eq!(report.conflicts.len(), 1, "{what}: {report:?}");
    let c = &report.conflicts[0];
    assert_eq!(c.path, "notes/first.md", "{what}");
    assert_eq!(c.merge, merge, "{what}");
    assert!(
        c.ours
            .as_deref()
            .is_some_and(|t| t.contains("Three, ours.")),
        "{what}: {c:?}"
    );
    assert!(
        c.theirs
            .as_deref()
            .is_some_and(|t| t.contains("Three, theirs.")),
        "{what}: {c:?}"
    );
    assert_eq!((report.pulled, report.behind_conflict), (0, 0), "{what}");
}

#[test]
fn a_read_that_lands_nothing_reports_the_waiting_conflict() {
    let w = World::new();
    overlapping(&w);
    let found = session_at(&w, &w.ours, Intent::Read, at(0)).expect("a report");
    let merge = w.merge_id();
    assert!(!merge.is_empty(), "no waiting merge: {found:?}");
    assert_waiting(&found, &merge, "the read that met it");
    let pending = w.ours.join(".git").join(kluris_core::sync::PENDING);
    let recorded = fs::read(&pending).unwrap();
    for intent in READS {
        // Inside the throttle.
        let report = session_at(&w, &w.ours, intent, at(30)).expect("a report");
        assert_eq!(report.left_as_is, None, "{intent:?}");
        assert_waiting(&report, &merge, &format!("{intent:?}, throttled"));
    }
    // No newer commit upstream.
    let report = session_at(&w, &w.ours, Intent::Read, at(100)).expect("a report");
    assert_waiting(&report, &merge, "nothing new");
    // Another command holds the remote claim.
    let held = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let report = session_at(&w, &w.ours, Intent::Read, at(200)).expect("a report");
    assert_eq!(reason(&report), Some("busy"), "{report:?}");
    assert_waiting(&report, &merge, "busy");
    drop(held);
    // The remote is gone.
    let gone = w.dir.path().join("gone.git").display().to_string();
    w.run(&w.ours, &["remote", "set-url", "origin", &gone]);
    let report = session_at(&w, &w.ours, Intent::Read, at(300)).expect("a report");
    assert!(
        matches!(reason(&report), Some("offline" | "refused")),
        "{report:?}"
    );
    assert_waiting(&report, &merge, "unreachable");
    assert_eq!(fs::read(&pending).unwrap(), recorded, "the record changed");
}

#[test]
fn a_stale_waiting_conflict_is_not_reported_nor_touched() {
    let w = World::new();
    overlapping(&w);
    assert!(session_at(&w, &w.ours, Intent::Read, at(0)).is_some());
    let pending = w.ours.join(".git").join(kluris_core::sync::PENDING);
    let recorded = fs::read(&pending).unwrap();
    // HEAD moves on: the merge was found for a commit that is no longer
    // the branch's.
    w.ours_commit(&[("notes/keep.md", &neuron("", "# Keep, again\n"))], &[]);
    for intent in READS {
        let report = session_at(&w, &w.ours, intent, at(30)).expect("commits to push");
        assert_eq!(report.conflicts, [], "{intent:?}");
        assert_eq!(report.to_push, 2, "{intent:?}");
    }
    assert_eq!(fs::read(&pending).unwrap(), recorded, "the record changed");
}

/// The child's environment: what it does, the world's folder, the brain,
/// and where it writes what it did.
const CHILD_ACTION: &str = "KLURIS_TEST_SESSION_ACTION";
const CHILD_WORLD: &str = "KLURIS_TEST_SESSION_WORLD";
const CHILD_BRAIN: &str = "KLURIS_TEST_SESSION_BRAIN";
const CHILD_OUT: &str = "KLURIS_TEST_SESSION_OUT";

/// This test binary run again as a child, without the world's lock, that
/// `read`s the brain through a session (with `KLURIS_NO_AUTO_SYNC=1` in
/// its environment), or `land`s what a pull brings: it fetches, writes
/// [`claiming`] right before it takes the tree claim, then waits for it up
/// to an explicit command's bound; either writes `<pulled> <milliseconds
/// when done>`. Or it opens an `explicit` session, writing [`opening`] right
/// before, closes it and writes `<pulled> <pushed> <milliseconds when it
/// was open>`. Without those variables it does nothing.
#[test]
#[ignore = "run only as the child of the cross-process tests"]
fn child_session() {
    let (Some(action), Some(world), Some(root), Some(out)) = (
        std::env::var(CHILD_ACTION).ok(),
        std::env::var_os(CHILD_WORLD),
        std::env::var_os(CHILD_BRAIN),
        std::env::var_os(CHILD_OUT),
    ) else {
        return;
    };
    let world = PathBuf::from(world);
    let root = PathBuf::from(root);
    let mut git = SystemGit::new()
        .env("GIT_CONFIG_GLOBAL", world.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", &world);
    for var in ISOLATED_VARS {
        git = git.env_remove(var);
    }
    let pulled = match action.as_str() {
        "read" => {
            let env = SessionEnv {
                git,
                no_verify: false,
                now: SystemTime::now(),
                program: None,
            };
            session::close(session::open(brain(&root), Intent::Read, &env))
                .map_or(0, |report| report.pulled)
        }
        "land" => {
            let request = SyncRequest {
                no_verify: false,
                maps: options(),
                network_time_limit: NETWORK_TIME_LIMIT,
                tree_bound: COMMAND_BOUND,
            };
            let ctx = engine::open(&root, &git, &request).unwrap();
            let upstream = engine::ready(&ctx).unwrap();
            let remote = ctx.take_remote().unwrap().expect("the remote claim");
            let theirs = engine::fetch_upstream(&ctx, upstream, &remote).unwrap();
            drop(remote);
            fs::write(claiming(Path::new(&out)), "").unwrap();
            let tree = ctx.claim_tree().unwrap();
            engine::pull_fetched(&ctx, &tree, theirs).pulled
        }
        "explicit" => {
            let env = SessionEnv {
                git,
                no_verify: false,
                now: SystemTime::now(),
                program: None,
            };
            fs::write(opening(Path::new(&out)), "").unwrap();
            let open = session::open(brain(&root), Intent::Explicit, &env);
            let opened = millis(SystemTime::now());
            let pulled = open.report().map_or(0, |report| report.pulled);
            let pushed = session::close(open).is_some_and(|report| report.pushed);
            fs::write(&out, format!("{pulled} {pushed} {opened}\n")).unwrap();
            return;
        }
        other => panic!("no action {other}"),
    };
    let done = millis(SystemTime::now());
    let partial = PathBuf::from(&out).with_extension("partial");
    fs::write(&partial, format!("{pulled} {done}\n")).unwrap();
    fs::rename(&partial, &out).unwrap();
}

/// What the `land` child writes right before it takes the tree claim.
fn claiming(out: &Path) -> PathBuf {
    out.with_extension("claiming")
}

/// What the `explicit` child writes right before it opens its session.
fn opening(out: &Path) -> PathBuf {
    out.with_extension("opening")
}

/// The child doing `action` on "ours", writing to `out`.
fn child(w: &World, action: &str, out: &Path) -> Child {
    child_command(w, action, out).spawn().unwrap()
}

/// The command of [`child`].
fn child_command(w: &World, action: &str, out: &Path) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--ignored", "--exact", "child_session", "--quiet"])
        .env(CHILD_ACTION, action)
        .env(CHILD_WORLD, w.dir.path())
        .env(CHILD_BRAIN, &w.ours)
        .env(CHILD_OUT, out)
        .env("KLURIS_NO_AUTO_SYNC", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// How long the remote claim stays held once the explicit session's child
/// says it opens the session.
const HELD_PAST_OPENING: Duration = Duration::from_secs(1);

/// An explicit session waits for a remote claim held from before it opens
/// until it is let go, then fetches and pushes: one that tried the claim
/// once would be busy.
#[test]
fn an_explicit_session_waits_for_a_remote_claim_let_go_within_its_bound() {
    let w = World::new();
    teammate_pushes(&w, "ideas/first-new.md");
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let held = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let out = w.dir.path().join("explicit.out");
    let mut explicit = child(&w, "explicit", &out);
    eventually("the session's opening", || {
        assert_running(&mut explicit);
        opening(&out).exists()
    });
    thread::sleep(HELD_PAST_OPENING);
    let released = millis(SystemTime::now());
    drop(held);
    assert!(wait_bounded(&mut explicit, "the session", PATIENCE).success());
    let text = fs::read_to_string(&out).unwrap();
    let fields: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(fields[..2], ["1", "true"], "pulled and pushed: {text:?}");
    let opened: u128 = fields[2].parse().unwrap();
    assert!(
        opened >= released,
        "open at {opened}, before the claim was let go at {released}"
    );
    assert_eq!(w.remote_head(), w.head(&w.ours));
    assert_eq!(w.parents(&w.ours, "HEAD"), 2, "a merge of the moved remote");
}

/// How many times the interrupted read runs: the thread the signal lands
/// on is the kernel's choice, so one run proves little.
#[cfg(unix)]
const INTERRUPTED_READS: usize = 20;

/// A read interrupted while a merge hook runs (SIGINT to its command's
/// process group, as Ctrl-C sends it) ends the hook and everything the hook
/// started, then dies of the signal: the tree claim is free only once they
/// are gone, and nothing landed. Every time, wherever the signal lands.
#[cfg(unix)]
#[test]
fn an_interrupted_read_ends_its_hooks_before_it_lets_go_of_the_brain() {
    for read in 1..=INTERRUPTED_READS {
        interrupted_read(&format!("read {read} of {INTERRUPTED_READS}"));
    }
}

/// One read of [`an_interrupted_read_ends_its_hooks_before_it_lets_go_of_the_brain`],
/// in a world of its own.
#[cfg(unix)]
fn interrupted_read(what: &str) {
    use rustix::process::{Pid, Signal, kill_process_group};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    let w = World::new();
    let hook = w.dir.path().join("hook.pid");
    let started = w.dir.path().join("started.pid");
    let late = w.dir.path().join("hook.late");
    w.hook(
        &w.ours,
        "pre-merge-commit",
        &format!(
            "sleep 20 &\necho $! > '{}'\necho $$ > '{}'\n{STALL}\necho late > '{}'",
            sh_path(&started),
            sh_path(&hook),
            sh_path(&late)
        ),
    );
    diverged(&w);
    let before = w.head(&w.ours);
    let out = w.dir.path().join("read.out");
    let mut cmd = child_command(&w, "read", &out);
    cmd.process_group(0);
    let mut reader = cmd.spawn().unwrap();
    let pid = |file: &Path| fs::read_to_string(file).ok().filter(|p| p.ends_with('\n'));
    eventually("the hook", || {
        assert_running(&mut reader);
        pid(&hook).is_some()
    });
    let pids = [pid(&hook).unwrap(), pid(&started).unwrap()];
    let group = Pid::from_raw(i32::try_from(reader.id()).unwrap()).unwrap();
    kill_process_group(group, Signal::INT).unwrap();
    let own = w.ours.join(".git");
    eventually("the tree claim", || {
        TreeClaim::exclusive(&own, Duration::ZERO).is_ok()
    });
    for pid in &pids {
        assert!(gone(pid.trim()), "{what}: {} outlived the read", pid.trim());
    }
    let status = wait_bounded(&mut reader, "the reader", PATIENCE);
    assert_eq!(
        status.signal(),
        Some(Signal::INT.as_raw()),
        "{what}: {status:?}"
    );
    assert_eq!(w.head(&w.ours), before, "{what}");
    assert!(!late.exists(), "{what}: the hook went on");
}

/// Whether the process `pid` is gone: none runs under it, or it is a
/// zombie, ended and waiting to be reaped.
#[cfg(unix)]
fn gone(pid: &str) -> bool {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    stat.trim().is_empty() || stat.trim_start().starts_with('Z')
}

#[test]
fn an_explicit_session_is_busy_once_the_remote_claim_is_held_past_its_bound() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    let before = w.head(&w.ours);
    let held = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    let started = Instant::now();
    let report = session_at(&w, &w.ours, Intent::Explicit, at(0)).expect("a report");
    let took = started.elapsed();
    assert!(took >= COMMAND_BOUND, "did not wait its bound: {took:?}");
    assert_eq!(reason(&report), Some("busy"), "{report:?}");
    assert_eq!(w.head(&w.ours), before);
    drop(held);
}

#[test]
fn a_read_never_waits_for_the_remote_claim() {
    let w = World::new();
    teammate_pushes(&w, "ideas/new.md");
    let held = RemoteClaim::try_take(&w.ours.join(".git")).unwrap();
    for intent in READS {
        let started = Instant::now();
        let report = session_at(&w, &w.ours, intent, at(0)).expect("a report");
        let took = started.elapsed();
        assert_eq!(reason(&report), Some("busy"), "{intent:?}: {report:?}");
        assert!(took < Duration::from_secs(2), "{intent:?} waited {took:?}");
    }
    drop(held);
}

fn assert_running(child: &mut Child) {
    if let Ok(Some(status)) = child.try_wait() {
        panic!("the child ended early: {status}");
    }
}

/// What the child wrote: the commits it pulled and when it was done.
fn child_result(out: &Path) -> (usize, u128) {
    let text = fs::read_to_string(out).unwrap();
    let mut fields = text.split_whitespace();
    let pulled = fields.next().unwrap().parse().unwrap();
    let done = fields.next().unwrap().parse().unwrap();
    (pulled, done)
}

fn millis(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).unwrap().as_millis()
}
