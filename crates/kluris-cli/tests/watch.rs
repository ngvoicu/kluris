//! `kluris sync --watch` and the per-user sync services (TEST-K3-51).
//!
//! - The watch loop (`kluris_core::sync::watch`) driven by a fake clock and
//!   a fake git: a change on either side runs one sync, offline is quiet, a
//!   conflict, a block or a refusal is reported once and the brain is left
//!   alone until a head moves, a stop ends the loop.
//! - The binary over real git: the interval bounds, one line (or one JSON
//!   document) per sync, and a clean exit 0 on SIGINT/SIGTERM sent to its
//!   whole process group, a sync in progress finishing first (Unix), or on
//!   CTRL_BREAK (Windows).
//! - The services (`kluris_core::service`) through a fake that records each
//!   command: the launchd plist, the systemd unit and the Task Scheduler
//!   XML byte for byte, the exact commands, idempotent install and
//!   uninstall, the listing `kluris doctor` shows (through a fake
//!   `launchctl`/`systemctl` first on PATH, Unix).
//!
//! `self-uninstall` and the services are in `self_uninstall.rs`, beside the
//! installed HOME they need.
//!
//! The opt-in `real_service_round_trip` installs a real service for a
//! throwaway brain, checks it runs and syncs, uninstalls it and checks that
//! nothing is left; it runs only with `--ignored`.
//!
//! Every process a test starts and every service it installs has a guard
//! ([`Owned`], [`TestService`]) that stops it (by its own process group or
//! job, by the PIDs the test recorded, by the service name the test made)
//! and waits, on every unwind path, so a failing test leaves nothing
//! running. A guard cannot run when the test process is itself killed; each
//! guard says what then remains.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use assert_fs::TempDir;
use kluris_core::service::{
    self, Installed, Manager, Ran, Runner, ServiceEnv, ServiceError, ServiceState,
};
use kluris_core::sync::SyncReport;
use kluris_core::sync::watch::{
    self, Clock, Heads, MIN_INTERVAL_SECS, Probe, RETRY_FIRST, RETRY_MAX, Report, WatchGit,
    WatchRequest, Watched,
};
use serde_json::Value;

// ---------------------------------------------------------------------------
// The loop, with a fake clock and a fake git.
// ---------------------------------------------------------------------------

fn heads(local: &str, remote: &str) -> Heads {
    Heads {
        local: local.into(),
        remote: remote.into(),
    }
}

/// Answers each probe from a script (the last answer repeats), `settled`
/// with what the last sync left, and `local_state` (what `git status` and
/// the operation markers say) as a function of the tick (the probes made
/// so far), counting its calls.
struct FakeGit {
    probes: RefCell<VecDeque<Probe>>,
    last: RefCell<Probe>,
    settled: RefCell<Option<Heads>>,
    probed: RefCell<usize>,
    local: Box<dyn Fn(usize) -> Option<String>>,
    local_calls: RefCell<usize>,
}

impl FakeGit {
    fn new(probes: Vec<Probe>) -> FakeGit {
        FakeGit {
            probes: RefCell::new(probes.into()),
            last: RefCell::new(Probe::Offline),
            settled: RefCell::new(None),
            probed: RefCell::new(0),
            local: Box::new(|_| Some(String::new())),
            local_calls: RefCell::new(0),
        }
    }

    /// The local state at each tick.
    fn with_local(mut self, local: impl Fn(usize) -> Option<String> + 'static) -> FakeGit {
        self.local = Box::new(local);
        self
    }
}

impl WatchGit for FakeGit {
    fn probe(&self, _root: &Path, _deadline: Instant) -> Probe {
        *self.probed.borrow_mut() += 1;
        if let Some(next) = self.probes.borrow_mut().pop_front() {
            *self.last.borrow_mut() = next;
        }
        self.last.borrow().clone()
    }

    fn settled(&self, _root: &Path) -> Option<Heads> {
        self.settled.borrow().clone()
    }

    fn local_state(&self, _root: &Path, _deadline: Instant) -> Option<String> {
        *self.local_calls.borrow_mut() += 1;
        (self.local)(*self.probed.borrow())
    }
}

/// Time moves only when the loop sleeps; after `ticks` waits of a full
/// interval it raises the stop flag.
struct FakeClock<'a> {
    now: RefCell<Instant>,
    slept: RefCell<Vec<Duration>>,
    interval: Duration,
    ticks: usize,
    stop: &'a AtomicBool,
}

impl FakeClock<'_> {
    fn waited(&self) -> Duration {
        self.slept.borrow().iter().sum()
    }
}

impl Clock for FakeClock<'_> {
    fn now(&self) -> Instant {
        *self.now.borrow()
    }

    fn sleep(&self, d: Duration) {
        self.slept.borrow_mut().push(d);
        *self.now.borrow_mut() += d;
        let waited: Duration = self.waited();
        if waited >= self.interval * self.ticks as u32 {
            self.stop.store(true, Ordering::SeqCst);
        }
    }
}

/// What the loop did: each sync (brain, the heads probed before it) and
/// each report.
#[derive(Default)]
struct Log {
    syncs: Vec<String>,
    /// The tick (1-based) each sync ran in.
    sync_ticks: Vec<usize>,
    reports: Vec<String>,
}

fn report_line(r: &Report) -> String {
    match r {
        Report::Synced(Ok(report)) => {
            if !report.conflicts.is_empty() {
                "conflict".into()
            } else if !report.blocked.is_empty() {
                "blocked".into()
            } else if report.offline {
                "offline".into()
            } else if report.pushed {
                "pushed".into()
            } else if report.merged {
                "pulled".into()
            } else {
                "up to date".into()
            }
        }
        Report::Synced(Err(why)) => format!("refused: {why}"),
        Report::NoUpstream => "no upstream".into(),
    }
}

/// Run the loop for `ticks` intervals over one brain, each sync answering
/// from `outcomes` (the last repeats) and settling at `settle`.
fn run_loop(
    git: &FakeGit,
    ticks: usize,
    outcomes: Vec<Result<SyncReport, String>>,
    settle: impl Fn(usize) -> Option<Heads>,
) -> Log {
    let stop = AtomicBool::new(false);
    let interval = Duration::from_secs(30);
    let clock = FakeClock {
        now: RefCell::new(Instant::now()),
        slept: RefCell::new(Vec::new()),
        interval,
        ticks,
        stop: &stop,
    };
    let root = PathBuf::from("/brains/acme");
    let brains = [Watched {
        name: "acme",
        root: &root,
    }];
    let mut log = Log::default();
    let mut outcomes: VecDeque<_> = outcomes.into();
    let mut last = Ok(SyncReport::default());
    let mut n = 0;
    let mut syncs = Vec::new();
    let mut sync_ticks = Vec::new();
    let mut reports = Vec::new();
    watch::watch(
        &brains,
        git,
        &clock,
        &stop,
        &WatchRequest {
            interval,
            probe_time_limit: Duration::from_secs(10),
        },
        &mut |brain| {
            n += 1;
            syncs.push(brain.name.to_string());
            sync_ticks.push(*git.probed.borrow());
            if let Some(next) = outcomes.pop_front() {
                last = next;
            }
            *git.settled.borrow_mut() = settle(n);
            last.clone()
        },
        &mut |brain, report| reports.push(format!("{}: {}", brain.name, report_line(&report))),
    );
    log.syncs = syncs;
    log.sync_ticks = sync_ticks;
    log.reports = reports;
    // Each tick waits one interval, in slices, before the next.
    assert_eq!(clock.waited(), interval * ticks as u32);
    assert!(
        clock
            .slept
            .borrow()
            .iter()
            .all(|d| *d <= Duration::from_secs(1)),
        "the wait is sliced so a stop is seen at once: {:?}",
        clock.slept.borrow()
    );
    log
}

fn pushed() -> SyncReport {
    SyncReport {
        fetched: true,
        pushed: true,
        ..SyncReport::default()
    }
}

fn pulled() -> SyncReport {
    SyncReport {
        fetched: true,
        merged: true,
        ..SyncReport::default()
    }
}

fn up_to_date() -> SyncReport {
    SyncReport {
        fetched: true,
        ..SyncReport::default()
    }
}

fn conflicted() -> SyncReport {
    SyncReport {
        fetched: true,
        conflicts: vec![kluris_core::sync::Conflict {
            path: "notes/first.md".into(),
            kind: kluris_core::sync::ConflictKind::Content,
            base: Some("b".into()),
            ours: Some("o".into()),
            theirs: Some("t".into()),
        }],
        ..SyncReport::default()
    }
}

fn blocked() -> SyncReport {
    SyncReport {
        fetched: true,
        blocked: vec!["notes/first.md".into()],
        skipped_dirty: vec!["notes/first.md".into()],
        ..SyncReport::default()
    }
}

fn offline() -> SyncReport {
    SyncReport {
        offline: true,
        ..SyncReport::default()
    }
}

#[test]
fn the_first_tick_syncs_at_once_then_only_a_change_does() {
    // Ticks: start, nothing new, nothing new.
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))]);
    let log = run_loop(&git, 3, vec![Ok(up_to_date())], |_| Some(heads("L1", "R1")));
    assert_eq!(log.syncs, ["acme"]);
    assert_eq!(log.reports, ["acme: up to date"]);
    assert_eq!(*git.probed.borrow(), 3);
}

#[test]
fn a_remote_change_runs_one_sync() {
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L2", "R2")),
    ]);
    // The pull moves the local head to the remote's.
    let log = run_loop(&git, 4, vec![Ok(up_to_date()), Ok(pulled())], |n| match n {
        1 => Some(heads("L1", "R1")),
        _ => Some(heads("L2", "R2")),
    });
    assert_eq!(log.syncs, ["acme", "acme"]);
    assert_eq!(log.reports, ["acme: up to date", "acme: pulled"]);
}

#[test]
fn a_new_local_commit_runs_one_sync_and_its_push_is_not_a_change() {
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L2", "R1")),
        // After the push, the remote has the local commit.
        Probe::Heads(heads("L2", "L2")),
    ]);
    let log = run_loop(&git, 3, vec![Ok(up_to_date()), Ok(pushed())], |n| match n {
        1 => Some(heads("L1", "R1")),
        _ => Some(heads("L2", "L2")),
    });
    assert_eq!(log.syncs, ["acme", "acme"]);
    assert_eq!(log.reports, ["acme: up to date", "acme: pushed"]);
}

#[test]
fn offline_is_quiet_and_syncs_once_back() {
    let git = FakeGit::new(vec![
        Probe::Offline,
        Probe::Offline,
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L1", "R1")),
    ]);
    let log = run_loop(&git, 3, vec![Ok(up_to_date())], |_| Some(heads("L1", "R1")));
    assert_eq!(log.syncs, ["acme"]);
    assert_eq!(log.reports, ["acme: up to date"]);
}

#[test]
fn a_sync_that_finds_the_remote_unreachable_is_quiet_and_tried_again() {
    // ls-remote answered, then the fetch failed: nothing printed, the
    // baseline kept, so the next tick syncs again.
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L2", "R1")),
        Probe::Heads(heads("L2", "R1")),
        Probe::Heads(heads("L2", "L2")),
    ]);
    let log = run_loop(&git, 2, vec![Ok(offline()), Ok(pushed())], |n| match n {
        1 => Some(heads("L2", "R1")),
        _ => Some(heads("L2", "L2")),
    });
    assert_eq!(log.syncs, ["acme", "acme"]);
    assert_eq!(log.reports, ["acme: pushed"]);
}

#[test]
fn a_conflict_is_printed_once_and_synced_when_a_head_moves() {
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L1", "R2")),
        // `kluris resolve` made the merge commit.
        Probe::Heads(heads("M", "R2")),
    ]);
    // The one-minute retry at tick 3 finds the same conflict: silent (T-154).
    let log = run_loop(
        &git,
        4,
        vec![Ok(conflicted()), Ok(conflicted()), Ok(pushed())],
        |n| match n {
            3 => Some(heads("M", "M")),
            _ => Some(heads("L1", "R2")),
        },
    );
    assert_eq!(log.sync_ticks, [1, 3, 4]);
    assert_eq!(log.reports, ["acme: conflict", "acme: pushed"]);
}

#[test]
fn a_block_is_printed_once_and_again_when_a_head_moves() {
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L1", "R2")),
        Probe::Heads(heads("L1", "R3")),
    ]);
    let log = run_loop(&git, 4, vec![Ok(blocked())], |_| Some(heads("L1", "R1")));
    // The one-minute retry at tick 3 finds the same block: silent (T-154).
    assert_eq!(log.sync_ticks, [1, 3, 4]);
    assert_eq!(log.reports, ["acme: blocked", "acme: blocked"]);
}

// A brain a person must see to is looked at again each tick (one status
// call) and synced once the reason is gone, though no head moved (T-145).

/// What `git status --porcelain` says with notes/first.md changed.
const DIRTY: &str = " M notes/first.md";

#[test]
fn a_block_that_clears_without_a_head_moving_syncs_again() {
    // A teammate's change waits behind an uncommitted edit; at tick 4 the
    // edit is undone (no head moved).
    // After the pull at tick 4, HEAD is the remote's.
    let mut probes = vec![Probe::Heads(heads("L1", "R2")); 4];
    probes.push(Probe::Heads(heads("R2", "R2")));
    let git = FakeGit::new(probes).with_local(|tick| {
        Some(if tick < 4 {
            DIRTY.into()
        } else {
            String::new()
        })
    });
    // The one-minute retry at tick 3 is still blocked (silent, T-154).
    let log = run_loop(
        &git,
        5,
        vec![Ok(blocked()), Ok(blocked()), Ok(pulled())],
        |n| match n {
            3 => Some(heads("R2", "R2")),
            _ => Some(heads("L1", "R1")),
        },
    );
    assert_eq!(log.sync_ticks, [1, 3, 4]);
    assert_eq!(log.reports, ["acme: blocked", "acme: pulled"]);
    // Once after each block, then once a tick (2, 3, 4) until it cleared;
    // none once the brain is in sync.
    assert_eq!(*git.local_calls.borrow(), 5);
}

#[test]
fn a_block_that_changes_but_stays_is_synced_again_and_printed_once() {
    // Another file is edited at tick 3: the brain is synced again, still
    // blocked by the same file, and nothing new is printed.
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R2"))]).with_local(|tick| {
        Some(if tick < 3 {
            DIRTY.into()
        } else {
            format!("{DIRTY}\0?? notes/new.md")
        })
    });
    let log = run_loop(&git, 5, vec![Ok(blocked())], |_| Some(heads("L1", "R1")));
    // The local change at tick 3 starts the backoff over: a retry a
    // minute later, at tick 5 (T-154).
    assert_eq!(log.sync_ticks, [1, 3, 5]);
    assert_eq!(log.reports, ["acme: blocked"]);
}

#[test]
fn a_brain_in_sync_runs_no_status_call() {
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))]);
    let log = run_loop(&git, 4, vec![Ok(up_to_date())], |_| Some(heads("L1", "R1")));
    assert_eq!(log.sync_ticks, [1]);
    assert_eq!(*git.local_calls.borrow(), 0);
}

#[test]
fn an_open_conflict_cleared_without_a_head_moving_syncs_again() {
    // The watch starts on a brain with an open conflict; at tick 3 it is
    // dropped (`kluris resolve --abort` removes its record; no head moves).
    let open = "it has an open sync conflict; settle it with `kluris resolve` (recorded in .git/kluris-sync/pending.json)";
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))]).with_local(|tick| {
        Some(if tick < 3 {
            "kluris-sync/pending.json".into()
        } else {
            String::new()
        })
    });
    let log = run_loop(&git, 4, vec![Err(open.into()), Ok(up_to_date())], |_| {
        Some(heads("L1", "R1"))
    });
    assert_eq!(log.sync_ticks, [1, 3]);
    assert_eq!(
        log.reports,
        [format!("acme: refused: {open}"), "acme: up to date".into()]
    );
}

#[test]
fn a_refusal_that_stays_is_printed_once() {
    // The rebase goes on (its files change), the refusal stays.
    let why = "a rebase is in progress; finish or abort it first";
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))])
        .with_local(|tick| Some(format!("rebase-merge {tick}")));
    let log = run_loop(&git, 4, vec![Err(why.into())], |_| None);
    assert_eq!(log.sync_ticks, [1, 2, 3, 4]);
    assert_eq!(log.reports, [format!("acme: refused: {why}")]);
}

// A parked brain is also synced again on a backoff, whatever its status
// says: some blockers clear without `git status` changing (T-150). The
// loop's fake interval is 30 s, so tick k runs at (k - 1) * 30 s.

fn blocked_by(path: &str) -> SyncReport {
    SyncReport {
        fetched: true,
        blocked: vec![path.into()],
        ..SyncReport::default()
    }
}

/// The tick at which a run starting at 0 s reaches `secs`.
fn tick_at(secs: u64) -> usize {
    (secs / 30) as usize + 1
}

#[test]
fn the_retry_backoff_is_one_minute_doubling_up_to_ten() {
    assert_eq!(RETRY_FIRST, Duration::from_secs(60));
    assert_eq!(RETRY_MAX, Duration::from_secs(600));
    // Blocked for good; the status never changes.
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R2"))]);
    let log = run_loop(&git, 72, vec![Ok(blocked())], |_| Some(heads("L1", "R1")));
    // Parked at 0 s; retries at 60, 180 (+120), 420 (+240), 900 (+480),
    // then every 600 s: 1500, 2100.
    let expected: Vec<usize> = [0, 60, 180, 420, 900, 1500, 2100]
        .iter()
        .map(|s| tick_at(*s))
        .collect();
    assert_eq!(log.sync_ticks, expected);
    // Printed once, however many retries.
    assert_eq!(log.reports, ["acme: blocked"]);
}

#[test]
fn an_ignored_file_in_the_way_removed_is_pulled_on_the_first_retry() {
    // An ignored file does not show in `git status`: the local state
    // stays the same; the retry a minute later gets through.
    let mut probes = vec![Probe::Heads(heads("L1", "R2")); 3];
    probes.push(Probe::Heads(heads("R2", "R2")));
    let git = FakeGit::new(probes);
    let log = run_loop(
        &git,
        5,
        vec![Ok(blocked_by("private/draft.md")), Ok(pulled())],
        |n| match n {
            1 => Some(heads("L1", "R1")),
            _ => Some(heads("R2", "R2")),
        },
    );
    assert_eq!(log.sync_ticks, [1, tick_at(60)]);
    assert_eq!(log.reports, ["acme: blocked", "acme: pulled"]);
}

#[test]
fn a_repaired_pre_push_hook_pushes_on_a_retry() {
    // The hook rejects the push (a refusal); it is fixed between the
    // first and second retry, which nothing in `git status` shows.
    let refused =
        "merged locally, but the push was refused: error: failed to push some refs to '/r.git'";
    let mut probes = vec![Probe::Heads(heads("L2", "R1")); 7];
    probes.push(Probe::Heads(heads("L2", "L2")));
    let git = FakeGit::new(probes);
    let log = run_loop(
        &git,
        9,
        vec![Err(refused.into()), Err(refused.into()), Ok(pushed())],
        |n| match n {
            3 => Some(heads("L2", "L2")),
            _ => None,
        },
    );
    assert_eq!(log.sync_ticks, [1, tick_at(60), tick_at(180)]);
    assert_eq!(
        log.reports,
        [format!("acme: refused: {refused}"), "acme: pushed".into()]
    );
}

#[test]
fn a_local_change_resets_the_backoff() {
    // Retries at 60 s and 180 s; at 210 s (tick 8) another file is edited:
    // synced at once (still blocked, nothing printed) and the backoff
    // starts over: 270 s, then 390 s (not 420 s).
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R2"))]).with_local(|tick| {
        Some(if tick < 8 {
            DIRTY.into()
        } else {
            format!("{DIRTY}\0?? notes/other.md")
        })
    });
    let log = run_loop(&git, 15, vec![Ok(blocked())], |_| Some(heads("L1", "R1")));
    assert_eq!(
        log.sync_ticks,
        [1, tick_at(60), tick_at(180), 8, tick_at(270), tick_at(390)]
    );
    assert_eq!(log.reports, ["acme: blocked"]);
}

#[test]
fn a_head_move_resets_the_backoff() {
    // The same, with a teammate's push at tick 8 instead: synced at once
    // (a head move prints its outcome), then 270 s and 390 s.
    let mut probes = vec![Probe::Heads(heads("L1", "R2")); 7];
    probes.push(Probe::Heads(heads("L1", "R3")));
    let git = FakeGit::new(probes);
    let log = run_loop(&git, 15, vec![Ok(blocked())], |_| Some(heads("L1", "R1")));
    assert_eq!(
        log.sync_ticks,
        [1, tick_at(60), tick_at(180), 8, tick_at(270), tick_at(390)]
    );
    assert_eq!(log.reports, ["acme: blocked", "acme: blocked"]);
}

#[test]
fn a_retry_that_finds_a_new_reason_prints_it_once() {
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R2"))]);
    let log = run_loop(
        &git,
        8,
        vec![Ok(blocked()), Ok(blocked_by("notes/other.md"))],
        |_| Some(heads("L1", "R1")),
    );
    assert_eq!(log.sync_ticks, [1, tick_at(60), tick_at(180)]);
    assert_eq!(log.reports, ["acme: blocked", "acme: blocked"]);
}

#[test]
fn a_block_that_clears_while_offline_syncs_once_the_remote_answers() {
    // Blocked; then the remote goes away (ticks 2-3: no status call, as a
    // sync could not run); the edit is undone meanwhile; back at tick 4.
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R2")),
        Probe::Offline,
        Probe::Offline,
        Probe::Heads(heads("L1", "R2")),
        // After the pull.
        Probe::Heads(heads("R2", "R2")),
    ])
    .with_local(|tick| {
        Some(if tick < 3 {
            DIRTY.into()
        } else {
            String::new()
        })
    });
    let log = run_loop(&git, 5, vec![Ok(blocked()), Ok(pulled())], |n| match n {
        1 => Some(heads("L1", "R1")),
        _ => Some(heads("R2", "R2")),
    });
    assert_eq!(log.sync_ticks, [1, 4]);
    assert_eq!(log.reports, ["acme: blocked", "acme: pulled"]);
    // After the block, then at tick 4.
    assert_eq!(*git.local_calls.borrow(), 2);
}

#[test]
fn a_sync_that_found_the_remote_away_is_tried_again_once_it_answers() {
    // The probe answered but the sync's fetch did not (quiet); the heads
    // stay as they are; the next ticks sync again until it gets through.
    // After the push at tick 3, the remote has the local commit.
    let mut probes = vec![Probe::Heads(heads("L2", "R1")); 3];
    probes.push(Probe::Heads(heads("L2", "L2")));
    let git = FakeGit::new(probes);
    let log = run_loop(
        &git,
        4,
        vec![Ok(offline()), Ok(offline()), Ok(pushed())],
        |n| match n {
            3 => Some(heads("L2", "L2")),
            _ => None,
        },
    );
    assert_eq!(log.sync_ticks, [1, 2, 3]);
    assert_eq!(log.reports, ["acme: pushed"]);
}

#[test]
fn a_refusal_is_printed_once_until_the_state_changes() {
    let git = FakeGit::new(vec![
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L1", "R1")),
        Probe::Heads(heads("L1", "R1")),
    ]);
    let log = run_loop(
        &git,
        2,
        vec![Err(
            "a rebase is in progress; finish or abort it first".into()
        )],
        |_| None,
    );
    assert_eq!(log.syncs, ["acme"]);
    assert_eq!(
        log.reports,
        ["acme: refused: a rebase is in progress; finish or abort it first"]
    );
}

#[test]
fn a_brain_that_is_not_a_repository_is_refused_once() {
    // Not offline: one sync runs, so its refusal is printed, once.
    let git = FakeGit::new(vec![
        Probe::NotARepo,
        Probe::NotARepo,
        Probe::NotARepo,
        Probe::Heads(heads("L1", "R1")),
    ]);
    let log = run_loop(
        &git,
        4,
        vec![Err("it is not a git repository".into()), Ok(up_to_date())],
        |_| Some(heads("L1", "R1")),
    );
    assert_eq!(log.syncs, ["acme", "acme"]);
    assert_eq!(
        log.reports,
        [
            "acme: refused: it is not a git repository",
            "acme: up to date"
        ]
    );
}

#[test]
fn no_upstream_is_printed_once() {
    let git = FakeGit::new(vec![
        Probe::NoUpstream,
        Probe::NoUpstream,
        Probe::NoUpstream,
        Probe::Heads(heads("L1", "R1")),
    ]);
    let log = run_loop(&git, 4, vec![Ok(up_to_date())], |_| Some(heads("L1", "R1")));
    assert_eq!(log.syncs, ["acme"]);
    assert_eq!(log.reports, ["acme: no upstream", "acme: up to date"]);
}

#[test]
fn every_brain_is_watched_on_its_own() {
    let stop = AtomicBool::new(false);
    let interval = Duration::from_secs(30);
    let clock = FakeClock {
        now: RefCell::new(Instant::now()),
        slept: RefCell::new(Vec::new()),
        interval,
        ticks: 1,
        stop: &stop,
    };
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))]);
    let (a, b) = (PathBuf::from("/a"), PathBuf::from("/b"));
    let brains = [
        Watched {
            name: "alpha",
            root: &a,
        },
        Watched {
            name: "beta",
            root: &b,
        },
    ];
    let mut syncs = Vec::new();
    watch::watch(
        &brains,
        &git,
        &clock,
        &stop,
        &WatchRequest {
            interval,
            probe_time_limit: Duration::from_secs(10),
        },
        &mut |brain| {
            syncs.push(brain.name.to_string());
            Ok(up_to_date())
        },
        &mut |_, _| {},
    );
    assert_eq!(syncs, ["alpha", "beta"]);
}

#[test]
fn a_fix_made_as_soon_as_the_block_is_printed_is_seen_next_tick() {
    // The person undoes the edit the moment the line appears: the local
    // state the block was found in is the one before the line, so the next
    // tick sees the change (not a minute later).
    let acted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = acted.clone();
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R2"))]).with_local(move |_| {
        Some(if seen.get() {
            String::new()
        } else {
            DIRTY.into()
        })
    });
    let stop = AtomicBool::new(false);
    let interval = Duration::from_secs(30);
    let clock = FakeClock {
        now: RefCell::new(Instant::now()),
        slept: RefCell::new(Vec::new()),
        interval,
        ticks: 2,
        stop: &stop,
    };
    let root = PathBuf::from("/a");
    let mut outcomes = VecDeque::from([Ok(blocked()), Ok(pulled())]);
    let mut sync_ticks = Vec::new();
    watch::watch(
        &[Watched {
            name: "acme",
            root: &root,
        }],
        &git,
        &clock,
        &stop,
        &WatchRequest {
            interval,
            probe_time_limit: Duration::from_secs(10),
        },
        &mut |_| {
            sync_ticks.push(*git.probed.borrow());
            outcomes.pop_front().unwrap_or(Ok(up_to_date()))
        },
        &mut |_, _| acted.set(true),
    );
    assert_eq!(sync_ticks, [1, 2]);
}

#[test]
fn a_stop_before_the_first_tick_ends_at_once() {
    let stop = AtomicBool::new(true);
    let clock = FakeClock {
        now: RefCell::new(Instant::now()),
        slept: RefCell::new(Vec::new()),
        interval: Duration::from_secs(30),
        ticks: 1,
        stop: &stop,
    };
    let git = FakeGit::new(vec![Probe::Heads(heads("L1", "R1"))]);
    let root = PathBuf::from("/a");
    let mut synced = false;
    watch::watch(
        &[Watched {
            name: "acme",
            root: &root,
        }],
        &git,
        &clock,
        &stop,
        &WatchRequest {
            interval: Duration::from_secs(30),
            probe_time_limit: Duration::from_secs(10),
        },
        &mut |_| {
            synced = true;
            Ok(up_to_date())
        },
        &mut |_, _| {},
    );
    assert!(!synced);
    assert_eq!(*git.probed.borrow(), 0);
    assert!(clock.slept.borrow().is_empty());
}

#[test]
fn the_interval_is_at_least_five_seconds() {
    assert_eq!(MIN_INTERVAL_SECS, 5);
    assert_eq!(watch::parse_interval("30"), Ok(Duration::from_secs(30)));
    assert_eq!(watch::parse_interval("5"), Ok(Duration::from_secs(5)));
    assert_eq!(
        watch::parse_interval("4"),
        Err("4 is not in the range x>=5.".to_string())
    );
    assert_eq!(
        watch::parse_interval("0"),
        Err("0 is not in the range x>=5.".to_string())
    );
    assert_eq!(
        watch::parse_interval("soon"),
        Err("'soon' is not a valid integer.".to_string())
    );
    assert_eq!(
        watch::parse_interval("-5"),
        Err("'-5' is not a valid integer.".to_string())
    );
}

// ---------------------------------------------------------------------------
// The binary over real git.
// ---------------------------------------------------------------------------

const ISOLATED_VARS: [&str; 8] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "KLURIS_AGENT",
];

const NEURON: &str =
    "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# First\n\nOne.\n";

fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// One world at a time: each runs git processes (the Windows runner caps a
/// run at 64).
static SERIAL: Mutex<()> = Mutex::new(());

/// A HOME with one brain `acme` pushed to a bare remote.
struct World {
    _serial: MutexGuard<'static, ()>,
    dir: TempDir,
}

impl World {
    fn new() -> World {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        let w = World {
            _serial: serial,
            dir,
        };
        w.brain();
        w
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn root(&self) -> PathBuf {
        self.home().join("acme")
    }

    fn bare(&self) -> PathBuf {
        self.home().join("acme.git")
    }

    fn hermetic(&self, cmd: &mut StdCommand) {
        let home = self.home();
        cmd.env("HOME", home)
            .env("USERPROFILE", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
            .env("KLURIS_NO_PROMPT", "1")
            .env("KLURIS_NO_AUTO_SYNC", "1")
            .env("NO_COLOR", "1");
        for var in ISOLATED_VARS {
            cmd.env_remove(var);
        }
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = StdCommand::new("git");
        self.hermetic(&mut cmd);
        let out = cmd.args(args).current_dir(cwd).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let path = rel.split('/').fold(self.root(), |p, part| p.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn brain(&self) {
        let root = self.root();
        fs::create_dir_all(&root).unwrap();
        self.write(".gitignore", "kluris.yml\n");
        self.write("kluris.yml", "name: acme\ndescription: d\n");
        self.write(
            "glossary.md",
            "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
        );
        self.write("notes/first.md", NEURON);
        let config = format!(
            "brains:\n  acme:\n    path: {}\n    description: ''\n",
            yaml_path(&root)
        );
        let kluris = self.home().join(".kluris");
        fs::create_dir_all(&kluris).unwrap();
        fs::write(kluris.join("config.yml"), config).unwrap();
        let (code, out, err) = self.run(&["dream", "--brain", "acme", "--json"]);
        assert_eq!(code, 0, "{out}{err}");
        self.git(&root, &["init", "-q", "-b", "main"]);
        self.git(&root, &["config", "user.name", "Ana Tester"]);
        self.git(&root, &["config", "user.email", "ana@example.com"]);
        self.git(&root, &["config", "core.autocrlf", "false"]);
        self.git(&root, &["add", "-A"]);
        self.git(&root, &["commit", "-q", "-m", "init"]);
        let bare = self.bare().to_string_lossy().into_owned();
        self.git(self.home(), &["init", "-q", "--bare", "-b", "main", &bare]);
        self.git(&root, &["remote", "add", "origin", &bare]);
        self.git(&root, &["push", "-q", "-u", "origin", "main"]);
    }

    /// A local commit not pushed yet.
    fn commit_locally(&self) {
        self.write("notes/second.md", &NEURON.replace("First", "Second"));
        let root = self.root();
        self.git(&root, &["add", "-A"]);
        self.git(&root, &["commit", "-q", "-m", "second"]);
    }

    /// A teammate's clone at `<home>/mate` commits `rel` and pushes.
    #[cfg_attr(not(unix), allow(dead_code))]
    fn teammate_pushes(&self, rel: &str) {
        let mate = self.home().join("mate");
        if !mate.exists() {
            let bare = self.bare().to_string_lossy().into_owned();
            let mate_s = mate.to_string_lossy().into_owned();
            self.git(self.home(), &["clone", "-q", &bare, &mate_s]);
            self.git(&mate, &["config", "user.name", "Bob"]);
            self.git(&mate, &["config", "user.email", "bob@example.com"]);
            self.git(&mate, &["config", "core.autocrlf", "false"]);
        }
        let path = rel.split('/').fold(mate.clone(), |p, part| p.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, NEURON.replace("First", "Teammate")).unwrap();
        self.git(&mate, &["add", "-A"]);
        self.git(&mate, &["commit", "-q", "-m", rel]);
        self.git(&mate, &["push", "-q"]);
    }

    fn head(&self) -> String {
        self.git(&self.root(), &["rev-parse", "HEAD"])
            .trim()
            .to_string()
    }

    fn remote_head(&self) -> String {
        self.git(&self.bare(), &["rev-parse", "main"])
            .trim()
            .to_string()
    }

    fn command(&self, args: &[&str]) -> StdCommand {
        let mut cmd = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.hermetic(&mut cmd);
        cmd.args(args);
        cmd
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::from_std(self.command(args)).output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }
}

#[test]
fn an_interval_under_five_seconds_is_refused() {
    let w = World::new();
    for (value, message) in [
        (
            "4",
            "Invalid value for '--interval': 4 is not in the range x>=5.",
        ),
        (
            "soon",
            "Invalid value for '--interval': 'soon' is not a valid integer.",
        ),
    ] {
        let (code, out, err) = w.run(&["sync", "--watch", "--interval", value]);
        assert_eq!(code, 2, "{out}{err}");
        assert!(err.contains(message), "{err}");
        assert!(err.contains("Usage: kluris sync [OPTIONS]"), "{err}");
    }
}

#[test]
fn interval_and_the_service_flags_go_with_their_own_mode() {
    let w = World::new();
    for args in [
        &["sync", "--interval", "30"][..],
        &["sync", "--watch", "--install-service", "--brain", "acme"],
        &[
            "sync",
            "--install-service",
            "--uninstall-service",
            "--brain",
            "acme",
        ],
        &[
            "sync",
            "--install-service",
            "--interval",
            "30",
            "--brain",
            "acme",
        ],
    ] {
        let (code, out, err) = w.run(args);
        assert_eq!(code, 2, "{args:?}: {out}{err}");
    }
}

#[test]
fn a_service_is_for_one_brain() {
    let w = World::new();
    let (code, out, err) = w.run(&["sync", "--install-service", "--brain", "all"]);
    assert_eq!(code, 1, "{out}{err}");
    assert!(
        err.contains("--install-service needs one brain"),
        "{out}{err}"
    );
}

#[test]
fn help_lists_the_watch_and_service_flags() {
    let w = World::new();
    let (code, out, _) = w.run(&["sync", "--help"]);
    assert_eq!(code, 0);
    for flag in [
        "--watch",
        "--interval",
        "--install-service",
        "--uninstall-service",
    ] {
        assert!(out.contains(flag), "{flag}: {out}");
    }
    assert!(
        out.contains("[default: 30]") || out.contains("default 30"),
        "{out}"
    );
    // Internal to the Windows service.
    assert!(!out.contains("--log-file"), "{out}");
    assert!(!out.contains("--config"), "{out}");
}

// ---------------------------------------------------------------------------
// The processes a test starts, each owned by a guard (F163-4).
// ---------------------------------------------------------------------------

/// How long a watch asked to stop (TERM to its group) has before its group
/// is killed: a sync in progress finishes first.
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(20);

/// A process this test started, with everything it starts: on Unix it
/// leads a process group of its own, on Windows it is in a job object of
/// this test's. Dropped, so on every unwind path too, it is stopped and
/// waited for: on Unix its group gets TERM (the watch's clean stop), then
/// KILL after [`GRACE`]; on Windows the job is terminated. A child already
/// waited for is never signalled: its id may be another process's by then.
///
/// No guard runs when the test process itself is killed (SIGKILL, or the
/// default action of a Ctrl-C or SIGTERM sent to it, which ends it without
/// unwinding). On Unix the watch, in a group of its own, then keeps running
/// under PID 1, and so does its HOME (the `TempDir`, not removed either),
/// until it is stopped by its PID. On Windows the job's handle closes with
/// the test process and the job's kill-on-close ends the tree.
struct Owned {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl Owned {
    fn spawn(mut cmd: StdCommand) -> Owned {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let child = cmd.spawn().unwrap();
        #[cfg(windows)]
        let job = job::Job::adopt(&child);
        Owned {
            child,
            #[cfg(windows)]
            job,
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stop the process (its whole group or job) and wait for it.
    #[cfg(unix)]
    fn end(&mut self) {
        if !self.running() {
            return;
        }
        // Not waited for yet, so the group's id is still this child's.
        let group = format!("-{}", self.pid());
        let signal = |name: &str| {
            let _ = StdCommand::new("kill")
                .args([name, "--", &group])
                .stderr(Stdio::null())
                .status();
        };
        signal("-TERM");
        let until = Instant::now() + GRACE;
        while self.running() && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        if self.running() {
            signal("-KILL");
        }
        let _ = self.child.wait();
    }

    /// Stop the process (its whole job) and wait for it.
    #[cfg(windows)]
    fn end(&mut self) {
        match &self.job {
            Some(job) => job.terminate(),
            None => {
                if self.running() {
                    let _ = self.child.kill();
                }
            }
        }
        let _ = self.child.wait();
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.end();
    }
}

/// A job object holding a test's process and everything it starts, killed
/// when its last handle closes.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    };

    pub struct Job(HANDLE);

    impl Job {
        /// The child's new job, or `None` when it cannot have one.
        pub fn adopt(child: &Child) -> Option<Job> {
            // SAFETY: plain Win32 calls on a handle created here and the
            // child's process handle, which `child` keeps open; every
            // pointer is to a live local of the stated size.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    u32::try_from(std::mem::size_of_val(&info)).unwrap_or(0),
                );
                let process = child.as_raw_handle() as HANDLE;
                if set == 0 || AssignProcessToJobObject(job, process) == 0 {
                    CloseHandle(job);
                    return None;
                }
                Some(Job(job))
            }
        }

        /// How many processes are in the job (`None` when it cannot say).
        pub fn active(&self) -> Option<u32> {
            // SAFETY: `self.0` is this job's open handle; the pointer is to
            // a live local of the stated size.
            unsafe {
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                let ok = QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    (&raw mut info).cast(),
                    u32::try_from(std::mem::size_of_val(&info)).unwrap_or(0),
                    std::ptr::null_mut(),
                );
                (ok != 0).then_some(info.ActiveProcesses)
            }
        }

        /// Kill every process in the job.
        pub fn terminate(&self) {
            // SAFETY: `self.0` is this job's open handle.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            self.terminate();
            // SAFETY: closed once, here.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Whether the process `pid` is running.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    StdCommand::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Whether the process `pid` is running.
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain Win32 calls; the handle opened here is closed here.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// A `kluris sync --watch` this test started, owned by its guard; its
/// stdout arrives line by line.
struct Watching {
    owned: Owned,
    lines: mpsc::Receiver<String>,
}

impl Watching {
    fn start(w: &World, extra: &[&str]) -> Watching {
        let mut args = vec!["sync", "--watch", "--brain", "acme", "--interval", "5"];
        args.extend(extra);
        let mut cmd = w.command(&args);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
        let mut owned = Owned::spawn(cmd);
        let stdout = owned.child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        Watching { owned, lines: rx }
    }

    fn pid(&self) -> u32 {
        self.owned.pid()
    }

    fn line(&self) -> String {
        self.line_within(Duration::from_secs(20))
    }

    fn line_within(&self, limit: Duration) -> String {
        self.lines
            .recv_timeout(limit)
            .unwrap_or_else(|e| panic!("a line within {limit:?}: {e:?}"))
    }

    /// `signal` to the watch's process group, as a terminal sends Ctrl-C.
    #[cfg(unix)]
    fn signal_group(&self, signal: &str) {
        let group = format!("-{}", self.pid());
        let status = StdCommand::new("kill")
            .args([&format!("-{signal}"), "--", &group])
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// CTRL_BREAK to the watch's process group; false when there is no
    /// console to send it through.
    #[cfg(windows)]
    fn ctrl_break(&self) -> bool {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
        // SAFETY: a plain Win32 call with the child's process group id.
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid()) != 0 }
    }

    /// Exit code and stderr within `limit`; past it the test fails and the
    /// guard stops the watch.
    fn exit(mut self, limit: Duration) -> (Option<i32>, String) {
        let until = Instant::now() + limit;
        loop {
            if let Some(status) = self.owned.child.try_wait().unwrap() {
                let mut err = String::new();
                if let Some(mut e) = self.owned.child.stderr.take() {
                    let _ = io::Read::read_to_string(&mut e, &mut err);
                }
                return (status.code(), err);
            }
            assert!(
                Instant::now() < until,
                "the watch did not exit within {limit:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// F163-4: a watch test that fails leaves no watch running. The remote is
/// gone, so the watch's fetch fails and it stays quiet (the review's probe);
/// waiting for its first line panics, and the panic unwinds through the
/// guard, which stops the watch and waits for it.
#[test]
fn a_failing_watch_test_leaves_no_watch_running() {
    let pid = Mutex::new(None);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let w = World::new();
        fs::rename(w.bare(), w.home().join("gone.git")).unwrap();
        let watching = Watching::start(&w, &[]);
        *pid.lock().unwrap() = Some(watching.pid());
        let first = watching.line_within(Duration::from_secs(8));
        assert!(first.ends_with("acme: up to date"), "{first}");
    }));
    assert!(
        failed.is_err(),
        "the injected failure did not fail the test"
    );
    let pid = pid.into_inner().unwrap().expect("the watch was started");
    let survived = alive(pid);
    if survived {
        // This test's own watch, by its recorded id.
        stop_pid(pid);
    }
    assert!(!survived, "watch {pid} outlived its failed test");
}

/// Stop the process `pid`, one this test recorded as its own.
#[cfg(unix)]
fn stop_pid(pid: u32) {
    let pid = pid.to_string();
    let _ = StdCommand::new("kill").args(["-KILL", &pid]).status();
}

/// Stop the process `pid`, one this test recorded as its own.
#[cfg(windows)]
fn stop_pid(pid: u32) {
    let pid = pid.to_string();
    let _ = StdCommand::new("taskkill")
        .args(["/PID", &pid, "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The watch started in a process group of its own, as a terminal starts a
/// foreground job: a signal to the group reaches every process in it, the
/// way Ctrl-C does.
#[cfg(unix)]
mod unix_signals {
    use super::*;

    #[test]
    fn sigint_while_waiting_exits_0_at_once() {
        let w = World::new();
        w.commit_locally();
        let watching = Watching::start(&w, &[]);
        let first = watching.line();
        assert!(first.ends_with("acme: pushed"), "{first}");
        assert_eq!(w.remote_head(), w.head());
        let sent = Instant::now();
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
        assert!(sent.elapsed() < Duration::from_secs(3));
        assert!(!err.contains("panicked"), "{err}");
    }

    #[test]
    fn sigterm_exits_0() {
        let w = World::new();
        let watching = Watching::start(&w, &[]);
        let first = watching.line();
        assert!(first.ends_with("acme: up to date"), "{first}");
        watching.signal_group("TERM");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }

    /// A hook that marks that it runs, then takes 2 s; Ctrl-C (SIGINT to
    /// the whole group) lands while it runs. git, in a group of its own, is
    /// not interrupted: the sync completes, its line is printed, the watch
    /// exits 0, and nothing is left running or half done.
    fn sigint_during_a_hook(w: &World, hook_name: &str, expected: &str) {
        let marks = w.home().join("marks");
        let hook = w.root().join(".git").join("hooks").join(hook_name);
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\necho $$ > '{0}.pid'\nsleep 2\necho done > '{0}.done'\nexit 0\n",
                marks.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let watching = Watching::start(w, &[]);
        let pid_file = PathBuf::from(format!("{}.pid", marks.display()));
        let until = Instant::now() + Duration::from_secs(20);
        while !pid_file.exists() {
            assert!(Instant::now() < until, "the {hook_name} hook never ran");
            thread::sleep(Duration::from_millis(20));
        }
        watching.signal_group("INT");
        let first = watching.line();
        assert!(first.ends_with(expected), "{first}");
        let (code, err) = watching.exit(Duration::from_secs(20));
        assert_eq!(code, Some(0), "{err}");
        assert!(
            PathBuf::from(format!("{}.done", marks.display())).exists(),
            "the {hook_name} hook was cut short"
        );
        assert_eq!(w.remote_head(), w.head(), "the sync did not finish");
        let pid = fs::read_to_string(&pid_file).unwrap();
        assert!(
            !alive(pid.trim().parse().unwrap()),
            "the hook outlived the watch"
        );
        let status = w.git(&w.root(), &["status", "--porcelain"]);
        assert_eq!(status, "", "the work tree is not clean");
        assert!(!w.root().join(".git").join("index.lock").exists());
    }

    #[test]
    fn sigint_during_a_push_lets_it_finish_then_exits_0() {
        let w = World::new();
        w.commit_locally();
        sigint_during_a_hook(&w, "pre-push", "acme: pushed");
    }

    #[test]
    fn sigint_during_a_pull_lets_it_finish_then_exits_0() {
        // The fast-forward's post-merge hook: a local git step.
        let w = World::new();
        w.teammate_pushes("notes/third.md");
        sigint_during_a_hook(&w, "post-merge", "acme: pulled");
        assert!(w.root().join("notes").join("third.md").exists());
    }

    #[test]
    fn json_prints_one_sync_document_per_line() {
        let w = World::new();
        w.commit_locally();
        let watching = Watching::start(&w, &["--json"]);
        let first = watching.line();
        let doc: Value = serde_json::from_str(&first).unwrap_or_else(|e| panic!("{e}: {first}"));
        assert_eq!(doc["ok"], true);
        assert_eq!(doc["brains"][0]["name"], "acme");
        assert_eq!(doc["brains"][0]["pushed"], true);
        assert_eq!(doc["brains"][0]["refused"], Value::Null);
        // The same document `kluris sync --json` prints.
        let keys: Vec<&String> = doc["brains"][0].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "name",
                "fetched",
                "merged",
                "pushed",
                "auto_merged",
                "conflicts",
                "skipped_dirty",
                "offline",
                "blocked",
                "refused"
            ]
        );
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }

    #[test]
    fn a_teammates_push_is_pulled_on_the_next_tick() {
        let w = World::new();
        let watching = Watching::start(&w, &[]);
        assert!(watching.line().ends_with("acme: up to date"));
        w.teammate_pushes("notes/third.md");
        let next = watching.line();
        assert!(next.ends_with("acme: pulled"), "{next}");
        assert!(w.root().join("notes").join("third.md").exists());
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }

    /// The review's case (T-145): an uncommitted edit blocks a teammate's
    /// change; once the edit is undone (no head moves) the next tick pulls.
    #[test]
    fn a_block_that_clears_is_pulled_on_the_next_tick() {
        let w = World::new();
        w.teammate_pushes("notes/first.md");
        w.write("notes/first.md", &NEURON.replace("One.", "One, mine."));
        let watching = Watching::start(&w, &[]);
        let first = watching.line();
        assert!(
            first.contains("acme: not synced, uncommitted changes are in the way: notes/first.md"),
            "{first}"
        );
        w.git(&w.root(), &["checkout", "--", "notes/first.md"]);
        let next = watching.line();
        assert!(next.ends_with("acme: pulled"), "{next}");
        assert!(
            fs::read_to_string(w.root().join("notes").join("first.md"))
                .unwrap()
                .contains("# Teammate"),
            "the teammate's change did not land"
        );
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }

    /// The narrow check's case (T-150): an ignored file is in the way of a
    /// teammate's new file. Removing it changes nothing `git status` shows,
    /// so the fast path cannot see it; the one-minute retry pulls.
    #[test]
    fn an_ignored_file_in_the_way_removed_is_pulled_on_the_retry() {
        let w = World::new();
        w.write(".gitignore", "kluris.yml\nprivate/\n");
        w.git(&w.root(), &["commit", "-q", "-am", "ignore private/"]);
        w.git(&w.root(), &["push", "-q"]);
        // The teammate adds private/draft.md on purpose.
        let mate = w.home().join("mate");
        let bare = w.bare().to_string_lossy().into_owned();
        let mate_s = mate.to_string_lossy().into_owned();
        w.git(w.home(), &["clone", "-q", &bare, &mate_s]);
        w.git(&mate, &["config", "user.name", "Bob"]);
        w.git(&mate, &["config", "user.email", "bob@example.com"]);
        fs::create_dir_all(mate.join("private")).unwrap();
        fs::write(mate.join("private").join("draft.md"), "theirs\n").unwrap();
        w.git(&mate, &["add", "-f", "private/draft.md"]);
        w.git(&mate, &["commit", "-q", "-m", "draft"]);
        w.git(&mate, &["push", "-q"]);
        // Mine, ignored, at the same path.
        w.write("private/draft.md", "mine\n");
        let watching = Watching::start(&w, &[]);
        let first = watching.line();
        assert!(
            first
                .contains("acme: not synced, uncommitted changes are in the way: private/draft.md"),
            "{first}"
        );
        let before = w.git(
            &w.root(),
            &["status", "--porcelain", "--untracked-files=all"],
        );
        fs::remove_file(w.root().join("private").join("draft.md")).unwrap();
        let after = w.git(
            &w.root(),
            &["status", "--porcelain", "--untracked-files=all"],
        );
        assert_eq!(before, after, "the removal must not show in git status");
        let next = watching.line_within(Duration::from_secs(90));
        assert!(next.ends_with("acme: pulled"), "{next}");
        assert_eq!(
            fs::read_to_string(w.root().join("private").join("draft.md")).unwrap(),
            "theirs\n"
        );
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }

    #[test]
    fn a_conflict_is_printed_once() {
        let w = World::new();
        // Overlapping edits on both sides.
        let mate = w.home().join("mate");
        let bare = w.bare().to_string_lossy().into_owned();
        let mate_s = mate.to_string_lossy().into_owned();
        w.git(w.home(), &["clone", "-q", &bare, &mate_s]);
        w.git(&mate, &["config", "user.name", "Bob"]);
        w.git(&mate, &["config", "user.email", "bob@example.com"]);
        fs::write(
            mate.join("notes").join("first.md"),
            NEURON.replace("One.", "One, theirs."),
        )
        .unwrap();
        w.git(&mate, &["commit", "-q", "-am", "theirs"]);
        w.git(&mate, &["push", "-q"]);
        w.write("notes/first.md", &NEURON.replace("One.", "One, ours."));
        w.git(&w.root(), &["commit", "-q", "-am", "ours"]);
        let watching = Watching::start(&w, &[]);
        let first = watching.line();
        assert!(first.contains("acme: 1 conflict(s)"), "{first}");
        assert!(first.contains("content: notes/first.md"), "{first}");
        // Two more ticks: nothing new is printed.
        assert!(
            watching
                .lines
                .recv_timeout(Duration::from_secs(12))
                .is_err(),
            "the conflict was printed again"
        );
        watching.signal_group("INT");
        let (code, err) = watching.exit(Duration::from_secs(5));
        assert_eq!(code, Some(0), "{err}");
    }
}

#[cfg(windows)]
#[test]
fn ctrl_break_exits_0() {
    let w = World::new();
    let watching = Watching::start(&w, &[]);
    let first = watching.line();
    assert!(first.ends_with("acme: up to date"), "{first}");
    if !watching.ctrl_break() {
        // The guard ends the watch.
        eprintln!(
            "skipped: no console to send CTRL_BREAK through ({})",
            io::Error::last_os_error()
        );
        return;
    }
    let (code, err) = watching.exit(Duration::from_secs(10));
    assert_eq!(
        code,
        Some(0),
        "the watch did not exit 0 on CTRL_BREAK: {err}"
    );
}

/// The Windows service's watch (`--log-file`) runs under `conhost.exe
/// --headless`, which `schtasks /End` ends without ending its child: the
/// watch ends itself once the process that started it is gone. The parent
/// and the watch it starts are in this test's job, so the watch is seen to
/// end (the job empties) and is ended by the guard if it does not.
#[cfg(windows)]
#[test]
fn a_service_watch_ends_when_its_parent_does() {
    let w = World::new();
    let log = w.home().join("logs").join("watch.log");
    let mut cmd = StdCommand::new("cmd");
    w.hermetic(&mut cmd);
    cmd.arg("/C")
        .arg(assert_cmd::cargo::cargo_bin!("kluris"))
        .args([
            "sync",
            "--watch",
            "--brain",
            "acme",
            "--interval",
            "5",
            "--log-file",
        ])
        .arg(&log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut parent = Owned::spawn(cmd);
    assert!(parent.job.is_some(), "the parent could not be put in a job");
    let until = Instant::now() + Duration::from_secs(30);
    while !fs::read_to_string(&log).is_ok_and(|t| t.contains("acme: up to date")) {
        assert!(
            Instant::now() < until,
            "the watch never logged its first sync"
        );
        thread::sleep(Duration::from_millis(100));
    }
    // The parent alone; the watch stays in the job.
    parent.child.kill().unwrap();
    let _ = parent.child.wait();
    let in_job = |parent: &Owned| parent.job.as_ref().and_then(job::Job::active);
    let until = Instant::now() + Duration::from_secs(10);
    while in_job(&parent) != Some(0) {
        assert!(
            Instant::now() < until,
            "the watch outlived its parent by 10 s ({:?} processes left)",
            in_job(&parent)
        );
        thread::sleep(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------------------
// The services, through a fake that records each command.
// ---------------------------------------------------------------------------

/// Records every command as one line (`program arg arg`); answers from
/// `answers` by exact line (exit 0 and no output otherwise). `files` keeps
/// what a file passed with `/XML` held when schtasks ran.
#[derive(Default)]
struct FakeRunner {
    calls: RefCell<Vec<String>>,
    answers: RefCell<Vec<(String, Ran)>>,
    xml: RefCell<Vec<Vec<u8>>>,
}

impl FakeRunner {
    fn answer(&self, line: &str, code: i32, stdout: &str, stderr: &str) {
        self.answers.borrow_mut().push((
            line.to_string(),
            Ran {
                code: Some(code),
                stdout: stdout.into(),
                stderr: stderr.into(),
            },
        ));
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }
}

impl Runner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Ran> {
        let line = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(i) = args.iter().position(|a| *a == "/XML") {
            self.xml
                .borrow_mut()
                .push(fs::read(args[i + 1]).unwrap_or_default());
        }
        self.calls.borrow_mut().push(line.clone());
        // The latest answer for a line wins.
        let answer = self
            .answers
            .borrow()
            .iter()
            .rev()
            .find(|(l, _)| *l == line)
            .map(|(_, ran)| ran.clone());
        Ok(answer.unwrap_or(Ran {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }))
    }

    fn pause(&self, _d: Duration) {}
}

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            dir: tempfile::TempDir::new().unwrap(),
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn env(&self, manager: Manager) -> ServiceEnv {
        ServiceEnv {
            manager,
            home: self.home(),
            xdg_config_home: None,
            local_app_data: Some(self.dir.path().join("lad")),
            uid: 501,
            user: Some(r"HOUSE\ana".into()),
            kluris: self.home().join(".kluris").join("bin").join("kluris"),
            path_var: Some("/opt/homebrew/bin:/usr/bin:/bin".into()),
            kluris_config: None,
            temp_dir: self.dir.path().join("tmp"),
        }
    }
}

const LABEL: &str = "dev.ngvoicu.kluris.sync.acme";

fn literal_env(manager: Manager) -> ServiceEnv {
    ServiceEnv {
        manager,
        home: PathBuf::from("/Users/ana"),
        xdg_config_home: None,
        local_app_data: Some(PathBuf::from(r"C:\Users\ana\AppData\Local")),
        uid: 501,
        user: Some(r"HOUSE\ana".into()),
        kluris: PathBuf::from("/Users/ana/.kluris/bin/kluris"),
        path_var: Some("/opt/homebrew/bin:/usr/bin:/bin".into()),
        kluris_config: None,
        temp_dir: PathBuf::from("/tmp"),
    }
}

#[test]
fn the_launchd_plist_byte_for_byte() {
    let env = literal_env(Manager::Launchd);
    assert_eq!(
        env.definition("acme"),
        Some(PathBuf::from(
            "/Users/ana/Library/LaunchAgents/dev.ngvoicu.kluris.sync.acme.plist"
        ))
    );
    assert_eq!(
        env.log("acme"),
        Some(PathBuf::from(
            "/Users/ana/Library/Logs/kluris/sync-acme.log"
        ))
    );
    assert_eq!(env.name("acme"), LABEL);
    assert_eq!(
        env.render("acme").unwrap(),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- A Kluris sync service: kluris sync writes it with its install-service option and removes it with uninstall-service. -->
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>dev.ngvoicu.kluris.sync.acme</string>
	<key>ProgramArguments</key>
	<array>
		<string>/Users/ana/.kluris/bin/kluris</string>
		<string>sync</string>
		<string>--watch</string>
		<string>--brain</string>
		<string>acme</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>/opt/homebrew/bin:/usr/bin:/bin</string>
	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ThrottleInterval</key>
	<integer>30</integer>
	<key>ExitTimeOut</key>
	<integer>300</integer>
	<key>ProcessType</key>
	<string>Background</string>
	<key>StandardOutPath</key>
	<string>/Users/ana/Library/Logs/kluris/sync-acme.log</string>
	<key>StandardErrorPath</key>
	<string>/Users/ana/Library/Logs/kluris/sync-acme.log</string>
</dict>
</plist>
"#
    );
}

#[test]
fn the_launchd_plist_carries_kluris_config_and_escapes_xml() {
    let mut env = literal_env(Manager::Launchd);
    env.kluris = PathBuf::from("/Users/a&b/<k>/kluris");
    env.kluris_config = Some(PathBuf::from("/Users/ana/k \"x\".yml"));
    env.path_var = None;
    let plist = env.render("acme").unwrap();
    assert!(
        plist.contains("\t\t<string>/Users/a&amp;b/&lt;k&gt;/kluris</string>\n"),
        "{plist}"
    );
    assert!(
        plist.contains(
            "\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>KLURIS_CONFIG</key>\n\t\t<string>/Users/ana/k \"x\".yml</string>\n\t</dict>\n"
        ),
        "{plist}"
    );
    env.kluris_config = None;
    assert!(
        !env.render("acme").unwrap().contains("EnvironmentVariables"),
        "no variables, no dict"
    );
}

#[test]
fn the_systemd_unit_byte_for_byte() {
    let mut env = literal_env(Manager::Systemd);
    env.home = PathBuf::from("/home/ana");
    env.kluris = PathBuf::from("/home/ana/.kluris/bin/kluris");
    env.kluris_config = Some(PathBuf::from("/home/ana/50% off/config.yml"));
    assert_eq!(
        env.definition("acme"),
        Some(PathBuf::from(
            "/home/ana/.config/systemd/user/kluris-sync-acme.service"
        ))
    );
    // The journal keeps the log.
    assert_eq!(env.log("acme"), None);
    assert_eq!(env.name("acme"), "kluris-sync-acme.service");
    assert_eq!(
        env.render("acme").unwrap(),
        "# Written by `kluris sync --install-service --brain acme`; `kluris sync --uninstall-service --brain acme` removes it.
[Unit]
Description=Kluris sync for the brain acme

[Service]
Type=simple
ExecStart=\"/home/ana/.kluris/bin/kluris\" sync --watch --brain acme
Environment=\"PATH=/opt/homebrew/bin:/usr/bin:/bin\"
Environment=\"KLURIS_CONFIG=/home/ana/50%% off/config.yml\"
Restart=on-failure
RestartSec=30
TimeoutStopSec=300

[Install]
WantedBy=default.target
"
    );
    // XDG_CONFIG_HOME is respected.
    env.xdg_config_home = Some(PathBuf::from("/xdg"));
    assert_eq!(
        env.definition("acme"),
        Some(PathBuf::from("/xdg/systemd/user/kluris-sync-acme.service"))
    );
}

#[test]
fn the_systemd_unit_escapes_its_command_line() {
    let mut env = literal_env(Manager::Systemd);
    env.kluris = PathBuf::from("/opt/my \"tools\"/$HOME/100%/kluris\\x");
    env.path_var = None;
    let unit = env.render("acme").unwrap();
    assert!(
        unit.contains(
            "ExecStart=\"/opt/my \\\"tools\\\"/$$HOME/100%%/kluris\\\\x\" sync --watch --brain acme\n"
        ),
        "{unit}"
    );
    assert!(!unit.contains("Environment="), "{unit}");
}

#[test]
fn the_task_xml_byte_for_byte() {
    let mut env = literal_env(Manager::TaskScheduler);
    env.kluris = PathBuf::from(r"C:\Users\ana\.kluris\bin\kluris.exe");
    assert_eq!(env.definition("acme"), None);
    assert_eq!(
        env.log("acme"),
        Some(PathBuf::from(
            r"C:\Users\ana\AppData\Local\kluris\logs\sync-acme.log"
        ))
    );
    assert_eq!(env.name("acme"), r"\Kluris\sync-acme");
    assert_eq!(
        env.render("acme").unwrap(),
        r#"<?xml version="1.0" encoding="UTF-16"?>
<!-- A Kluris sync service: kluris sync writes it with its install-service option and removes it with uninstall-service. -->
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Kluris sync for the brain acme</Description>
    <URI>\Kluris\sync-acme</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>HOUSE\ana</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>HOUSE\ana</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>%SystemRoot%\System32\conhost.exe</Command>
      <Arguments>--headless "C:\Users\ana\.kluris\bin\kluris.exe" sync --watch --brain acme --log-file "C:\Users\ana\AppData\Local\kluris\logs\sync-acme.log"</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    );
    // KLURIS_CONFIG travels as an argument: a task has no environment.
    env.kluris_config = Some(PathBuf::from(r"C:\Users\ana\k & co\config.yml"));
    let xml = env.render("acme").unwrap();
    assert!(
        xml.contains(r#" --log-file "C:\Users\ana\AppData\Local\kluris\logs\sync-acme.log" --config "C:\Users\ana\k &amp; co\config.yml"</Arguments>"#),
        "{xml}"
    );
    // A quote cannot be passed inside a quoted argument.
    env.kluris_config = Some(PathBuf::from(r#"C:\a"b\config.yml"#));
    assert!(matches!(
        env.render("acme"),
        Err(ServiceError::Unquotable(_))
    ));
}

/// An XML comment may not hold `--` (schtasks refuses the task: "incorrect
/// comment syntax").
#[test]
fn xml_comments_are_well_formed() {
    for manager in [Manager::Launchd, Manager::TaskScheduler] {
        let mut env = literal_env(manager);
        env.kluris = PathBuf::from(r"C:\k\kluris.exe");
        let text = env.render("my--brain").unwrap();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("<!--") {
            let after = &rest[start + 4..];
            let end = after.find("-->").expect("an open comment");
            assert!(
                !after[..end].contains("--"),
                "{manager:?}: {}",
                &after[..end]
            );
            rest = &after[end + 3..];
        }
    }
}

#[test]
fn a_brain_name_that_is_not_kluris_shaped_is_refused() {
    let env = literal_env(Manager::Launchd);
    for bad in ["", "Acme", "a b", "../x", "a/b", "-a", "a\\b"] {
        assert!(
            matches!(env.render(bad), Err(ServiceError::BadName(_))),
            "{bad:?}"
        );
        let runner = FakeRunner::default();
        assert!(service::install(&env, bad, &runner).is_err(), "{bad:?}");
        assert!(service::uninstall(&env, bad, &runner).is_err(), "{bad:?}");
        assert!(runner.take().is_empty());
    }
}

#[test]
fn launchd_install_and_uninstall_are_idempotent() {
    let s = Sandbox::new();
    let env = s.env(Manager::Launchd);
    let plist = env.definition("acme").unwrap();
    let print = format!("launchctl print gui/501/{LABEL}");
    let runner = FakeRunner::default();
    runner.answer(&print, 113, "", "Could not find service");

    // Fresh: written, then loaded.
    let done = service::install(&env, "acme", &runner).unwrap();
    assert!(done.changed);
    assert_eq!(done.name, LABEL);
    assert_eq!(done.definition.as_deref(), Some(plist.as_path()));
    assert_eq!(
        runner.take(),
        [
            print.clone(),
            format!("launchctl bootstrap gui/501 {}", plist.display()),
        ]
    );
    assert_eq!(
        fs::read_to_string(&plist).unwrap(),
        env.render("acme").unwrap()
    );
    assert!(s.home().join("Library/Logs/kluris").is_dir());

    // Again, loaded and running: nothing to do.
    runner.answer(&print, 0, "\tstate = running\n", "");
    let again = service::install(&env, "acme", &runner).unwrap();
    assert!(!again.changed);
    assert_eq!(runner.take(), std::slice::from_ref(&print));

    // Loaded but stopped: started again.
    runner.answer(&print, 0, "\tstate = not running\n", "");
    service::install(&env, "acme", &runner).unwrap();
    assert_eq!(
        runner.take(),
        [
            print.clone(),
            format!("launchctl kickstart gui/501/{LABEL}")
        ]
    );

    // A changed definition (another kluris path): reloaded.
    runner.answer(&print, 0, "\tstate = running\n", "");
    let mut moved = env.clone();
    moved.kluris = s.home().join("elsewhere").join("kluris");
    let changed = service::install(&moved, "acme", &runner).unwrap();
    assert!(changed.changed);
    assert_eq!(
        runner.take(),
        [
            print.clone(),
            format!("launchctl bootout gui/501/{LABEL}"),
            format!("launchctl bootstrap gui/501 {}", plist.display()),
        ]
    );

    // Uninstall: stopped and removed.
    let gone = service::uninstall(&moved, "acme", &runner).unwrap();
    assert!(gone.removed);
    assert_eq!(
        runner.take(),
        [print.clone(), format!("launchctl bootout gui/501/{LABEL}")]
    );
    assert!(!plist.exists());

    // Again: nothing there, nothing done.
    runner.answer(&print, 113, "", "Could not find service");
    let again = service::uninstall(&moved, "acme", &runner).unwrap();
    assert!(!again.removed);
    assert_eq!(runner.take(), [print]);
}

#[test]
fn a_bootstrap_that_fails_is_retried_then_reported() {
    let s = Sandbox::new();
    let env = s.env(Manager::Launchd);
    let plist = env.definition("acme").unwrap();
    let print = format!("launchctl print gui/501/{LABEL}");
    let bootstrap = format!("launchctl bootstrap gui/501 {}", plist.display());
    let runner = FakeRunner::default();
    runner.answer(&print, 113, "", "");
    runner.answer(&bootstrap, 5, "", "Bootstrap failed: 5: Input/output error");
    let err = service::install(&env, "acme", &runner).unwrap_err();
    assert!(
        err.to_string()
            .contains("Bootstrap failed: 5: Input/output error"),
        "{err}"
    );
    assert_eq!(
        runner.take(),
        [print, bootstrap.clone(), bootstrap.clone(), bootstrap]
    );
}

#[test]
fn systemd_install_and_uninstall_are_idempotent() {
    let s = Sandbox::new();
    let mut env = s.env(Manager::Systemd);
    env.xdg_config_home = Some(s.dir.path().join("xdg"));
    let unit = env.definition("acme").unwrap();
    assert!(unit.starts_with(s.dir.path().join("xdg")));
    let probe = "systemctl --user show-environment".to_string();
    let active = "systemctl --user is-active kluris-sync-acme.service".to_string();
    let runner = FakeRunner::default();
    runner.answer(&active, 3, "inactive\n", "");

    let done = service::install(&env, "acme", &runner).unwrap();
    assert!(done.changed);
    assert_eq!(done.log, None);
    assert_eq!(
        runner.take(),
        [
            probe.clone(),
            active.clone(),
            "systemctl --user daemon-reload".into(),
            "systemctl --user enable --now kluris-sync-acme.service".into(),
        ]
    );
    assert_eq!(
        fs::read_to_string(&unit).unwrap(),
        env.render("acme").unwrap()
    );

    runner.answer(&active, 0, "active\n", "");
    let again = service::install(&env, "acme", &runner).unwrap();
    assert!(!again.changed);
    assert_eq!(runner.take(), [probe.clone(), active.clone()]);

    // A changed unit while it runs: reloaded and restarted.
    let mut moved = env.clone();
    moved.kluris = s.home().join("elsewhere").join("kluris");
    service::install(&moved, "acme", &runner).unwrap();
    assert_eq!(
        runner.take(),
        [
            probe.clone(),
            active.clone(),
            "systemctl --user daemon-reload".into(),
            "systemctl --user enable --now kluris-sync-acme.service".into(),
            "systemctl --user restart kluris-sync-acme.service".into(),
        ]
    );

    let gone = service::uninstall(&moved, "acme", &runner).unwrap();
    assert!(gone.removed);
    assert_eq!(
        runner.take(),
        [
            probe.clone(),
            "systemctl --user disable --now kluris-sync-acme.service".into(),
            "systemctl --user daemon-reload".into(),
        ]
    );
    assert!(!unit.exists());

    let again = service::uninstall(&moved, "acme", &runner).unwrap();
    assert!(!again.removed);
    assert!(runner.take().is_empty());
}

#[test]
fn without_a_user_systemd_install_is_a_clear_error_and_writes_nothing() {
    let s = Sandbox::new();
    let env = s.env(Manager::Systemd);
    let runner = FakeRunner::default();
    runner.answer(
        "systemctl --user show-environment",
        1,
        "",
        "Failed to connect to bus: No medium found\n",
    );
    let err = service::install(&env, "acme", &runner).unwrap_err();
    assert!(matches!(err, ServiceError::NoUserSystemd(_)), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("systemd user instance"), "{text}");
    assert!(
        text.contains("Failed to connect to bus: No medium found"),
        "{text}"
    );
    assert!(!env.definition("acme").unwrap().exists());
    assert_eq!(runner.take(), ["systemctl --user show-environment"]);

    // A missing systemctl is the same error.
    struct Missing;
    impl Runner for Missing {
        fn run(&self, _program: &str, _args: &[&str]) -> io::Result<Ran> {
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"))
        }
    }
    let err = service::install(&env, "acme", &Missing).unwrap_err();
    assert!(matches!(err, ServiceError::NoUserSystemd(_)), "{err:?}");
}

#[test]
fn task_scheduler_install_and_uninstall_are_idempotent() {
    let s = Sandbox::new();
    let env = s.env(Manager::TaskScheduler);
    fs::create_dir_all(&env.temp_dir).unwrap();
    let task = r"\Kluris\sync-acme";
    let query = format!("schtasks /Query /TN {task}");
    let xml = env
        .temp_dir
        .join(format!("kluris-sync-acme-{}.xml", std::process::id()));
    let create = format!("schtasks /Create /TN {task} /XML {} /F", xml.display());
    let runner = FakeRunner::default();
    runner.answer(
        &query,
        1,
        "",
        "ERROR: The system cannot find the file specified.\n",
    );

    let done = service::install(&env, "acme", &runner).unwrap();
    assert!(done.changed);
    assert_eq!(done.definition, None);
    assert_eq!(
        runner.take(),
        [
            query.clone(),
            create.clone(),
            format!("schtasks /Run /TN {task}"),
        ]
    );
    // UTF-16LE with a byte order mark, as schtasks reads it; then removed.
    let bytes = runner.xml.borrow()[0].clone();
    assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
    let units: Vec<u16> = bytes[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(
        String::from_utf16(&units).unwrap(),
        env.render("acme").unwrap()
    );
    assert!(!xml.exists());

    // Installed: ended, replaced and started again.
    runner.answer(&query, 0, "", "");
    service::install(&env, "acme", &runner).unwrap();
    assert_eq!(
        runner.take(),
        [
            query.clone(),
            format!("schtasks /End /TN {task}"),
            create,
            format!("schtasks /Run /TN {task}"),
        ]
    );

    let gone = service::uninstall(&env, "acme", &runner).unwrap();
    assert!(gone.removed);
    assert_eq!(
        runner.take(),
        [
            query.clone(),
            format!("schtasks /End /TN {task}"),
            format!("schtasks /Delete /TN {task} /F"),
        ]
    );

    runner.answer(
        &query,
        1,
        "",
        "ERROR: The system cannot find the file specified.\n",
    );
    let again = service::uninstall(&env, "acme", &runner).unwrap();
    assert!(!again.removed);
    assert_eq!(runner.take(), [query]);
}

#[test]
fn installed_services_are_listed_with_whether_they_run() {
    // launchd: the plists in LaunchAgents, each asked.
    let s = Sandbox::new();
    let env = s.env(Manager::Launchd);
    let runner = FakeRunner::default();
    assert_eq!(service::installed(&env, &runner).unwrap(), []);
    assert!(runner.take().is_empty(), "no plist, no launchctl");
    let agents = s.home().join("Library/LaunchAgents");
    fs::create_dir_all(&agents).unwrap();
    for name in [
        "dev.ngvoicu.kluris.sync.beta.plist",
        "dev.ngvoicu.kluris.sync.acme.plist",
        "com.example.other.plist",
        "dev.ngvoicu.kluris.sync.Bad Name.plist",
    ] {
        fs::write(agents.join(name), "<plist/>").unwrap();
    }
    runner.answer(
        &format!("launchctl print gui/501/{LABEL}"),
        0,
        "\tstate = running\n",
        "",
    );
    runner.answer(
        "launchctl print gui/501/dev.ngvoicu.kluris.sync.beta",
        113,
        "",
        "",
    );
    let list = service::installed(&env, &runner).unwrap();
    assert_eq!(
        list,
        [
            Installed {
                brain: "acme".into(),
                name: LABEL.into(),
                definition: Some(agents.join("dev.ngvoicu.kluris.sync.acme.plist")),
                state: ServiceState::Running,
            },
            Installed {
                brain: "beta".into(),
                name: "dev.ngvoicu.kluris.sync.beta".into(),
                definition: Some(agents.join("dev.ngvoicu.kluris.sync.beta.plist")),
                state: ServiceState::Stopped("not loaded".into()),
            },
        ]
    );

    // systemd: the units, each asked.
    let s = Sandbox::new();
    let env = s.env(Manager::Systemd);
    let units = s.home().join(".config/systemd/user");
    fs::create_dir_all(&units).unwrap();
    fs::write(units.join("kluris-sync-acme.service"), "").unwrap();
    fs::write(units.join("other.service"), "").unwrap();
    let runner = FakeRunner::default();
    runner.answer(
        "systemctl --user is-active kluris-sync-acme.service",
        3,
        "failed\n",
        "",
    );
    assert_eq!(
        service::installed(&env, &runner).unwrap(),
        [Installed {
            brain: "acme".into(),
            name: "kluris-sync-acme.service".into(),
            definition: Some(units.join("kluris-sync-acme.service")),
            state: ServiceState::Stopped("failed".into()),
        }]
    );
    assert_eq!(
        runner.take(),
        [
            "systemctl --user show-environment",
            "systemctl --user is-active kluris-sync-acme.service"
        ]
    );
    // A unit no user systemd can stop: listing it is an error.
    runner.answer(
        "systemctl --user show-environment",
        1,
        "",
        "Failed to connect to bus\n",
    );
    assert!(matches!(
        service::installed(&env, &runner),
        Err(ServiceError::NoUserSystemd(_))
    ));

    // Task Scheduler: the tasks in \Kluris\, their state as the
    // ScheduledTask State enum's name (never the display language's word).
    let s = Sandbox::new();
    let env = s.env(Manager::TaskScheduler);
    let runner = FakeRunner::default();
    // Their names from schtasks (a German status column, never read)...
    runner.answer(
        TASK_NAMES,
        0,
        "\"\\Microsoft\\Windows\\X\",\"N/A\",\"Bereit\"\r\n\"\\Kluris\\sync-acme\",\"N/A\",\"Wird ausgef\u{fc}hrt\"\r\n\"\\Kluris\\sync-beta\",\"27.10.2026 08:00:00\",\"Bereit\"\r\n",
        "",
    );
    // ...their states from PowerShell.
    runner.answer(
        TASK_STATES,
        0,
        "sync-beta\tReady\r\nsync-acme\tRunning\r\nother\tRunning\r\nsync-Bad\tReady\r\nsync-gamma\tDisabled\r\n",
        "",
    );
    assert_eq!(
        service::installed(&env, &runner).unwrap(),
        [
            Installed {
                brain: "acme".into(),
                name: r"\Kluris\sync-acme".into(),
                definition: None,
                state: ServiceState::Running,
            },
            Installed {
                brain: "beta".into(),
                name: r"\Kluris\sync-beta".into(),
                definition: None,
                state: ServiceState::Stopped("Ready".into()),
            },
            Installed {
                brain: "gamma".into(),
                name: r"\Kluris\sync-gamma".into(),
                definition: None,
                state: ServiceState::Stopped("Disabled".into()),
            },
        ]
    );
    assert_eq!(runner.take(), [TASK_NAMES, TASK_STATES]);
}

/// Every task's name (column 1); the status column is translated, never read.
const TASK_NAMES: &str = "schtasks /Query /FO CSV /NH";

/// The query of the Task Scheduler states: PowerShell's ScheduledTask
/// objects, whose `State` is an enum (`Ready`, `Running`, `Disabled`,
/// `Queued`, `Unknown`), the same in every display language, where
/// `schtasks /Query`'s status column is translated. No `\Kluris\` folder
/// is the ObjectNotFound error, which is "none" (exit 0); any other error
/// exits 1 (`-Command` alone exits 1 for both, as zeewin showed).
const TASK_STATES: &str = r#"powershell.exe -NoProfile -NonInteractive -Command try { Get-ScheduledTask -TaskPath '\Kluris\' -ErrorAction Stop | ForEach-Object { $_.TaskName + [char]9 + [string]$_.State } } catch { if ($_.CategoryInfo.Category -ne 'ObjectNotFound') { [Console]::Error.WriteLine($_); exit 1 } }; exit 0"#;

#[test]
fn task_states_do_not_depend_on_the_display_language() {
    let s = Sandbox::new();
    let env = s.env(Manager::TaskScheduler);
    // No Kluris task: PowerShell never starts (it writes into the profile).
    let runner = FakeRunner::default();
    runner.answer(
        TASK_NAMES,
        0,
        "\"\\Microsoft\\Windows\\X\",\"N/A\",\"En cours d'ex\u{e9}cution\"\r\n\"\\Kluris\\other\",\"N/A\",\"Pr\u{ea}t\"\r\n",
        "",
    );
    assert_eq!(service::installed(&env, &runner).unwrap(), []);
    assert_eq!(runner.take(), [TASK_NAMES]);
    // A Kluris task: its state is PowerShell's enum name, whatever
    // schtasks' (French) status column says.
    runner.answer(
        TASK_NAMES,
        0,
        "\"\\Kluris\\sync-acme\",\"N/A\",\"Pr\u{ea}t\"\r\n",
        "",
    );
    runner.answer(TASK_STATES, 0, "sync-acme\tRunning\r\n", "");
    assert_eq!(
        service::installed(&env, &runner).unwrap(),
        [Installed {
            brain: "acme".into(),
            name: r"\Kluris\sync-acme".into(),
            definition: None,
            state: ServiceState::Running,
        }]
    );
    assert_eq!(runner.take(), [TASK_NAMES, TASK_STATES]);
    // A failing state query is an error, never "nothing installed".
    runner.answer(TASK_STATES, 1, "", "Get-ScheduledTask : Access denied\r\n");
    let err = service::installed(&env, &runner).unwrap_err();
    assert!(err.to_string().contains("Access denied"), "{err}");
}

// `kluris remove` and the brain's service.

#[test]
fn removing_a_brain_without_a_service_runs_nothing() {
    let s = Sandbox::new();
    for manager in [Manager::Launchd, Manager::Systemd] {
        let env = s.env(manager);
        let runner = FakeRunner::default();
        assert_eq!(
            service::remove_brain_service(&env, "acme", &runner),
            Ok(None)
        );
        assert!(runner.take().is_empty(), "{manager:?}");
    }
    let env = s.env(Manager::TaskScheduler);
    let runner = FakeRunner::default();
    let query = r"schtasks /Query /TN \Kluris\sync-acme";
    runner.answer(query, 1, "", "ERROR: ...\r\n");
    assert_eq!(
        service::remove_brain_service(&env, "acme", &runner),
        Ok(None)
    );
    assert_eq!(runner.take(), [query]);
    // A name no service can have.
    let runner = FakeRunner::default();
    assert_eq!(
        service::remove_brain_service(&env, "Not A Name", &runner),
        Ok(None)
    );
    assert!(runner.take().is_empty());
}

#[test]
fn removing_a_brain_stops_and_removes_its_service() {
    let s = Sandbox::new();
    // launchd.
    let env = s.env(Manager::Launchd);
    let plist = env.definition("acme").unwrap();
    fs::create_dir_all(plist.parent().unwrap()).unwrap();
    fs::write(&plist, "<plist/>").unwrap();
    let runner = FakeRunner::default();
    runner.answer(
        &format!("launchctl print gui/501/{LABEL}"),
        0,
        "\tstate = running\n",
        "",
    );
    assert_eq!(
        service::remove_brain_service(&env, "acme", &runner),
        Ok(Some(LABEL.to_string()))
    );
    assert_eq!(
        runner.take(),
        [
            format!("launchctl print gui/501/{LABEL}"),
            format!("launchctl bootout gui/501/{LABEL}"),
        ]
    );
    assert!(!plist.exists());
    // systemd.
    let env = s.env(Manager::Systemd);
    let unit = env.definition("acme").unwrap();
    fs::create_dir_all(unit.parent().unwrap()).unwrap();
    fs::write(&unit, "[Unit]\n").unwrap();
    let runner = FakeRunner::default();
    assert_eq!(
        service::remove_brain_service(&env, "acme", &runner),
        Ok(Some("kluris-sync-acme.service".to_string()))
    );
    assert_eq!(
        runner.take(),
        [
            "systemctl --user show-environment",
            "systemctl --user disable --now kluris-sync-acme.service",
            "systemctl --user daemon-reload",
        ]
    );
    assert!(!unit.exists());
    // Task Scheduler.
    let env = s.env(Manager::TaskScheduler);
    let runner = FakeRunner::default();
    assert_eq!(
        service::remove_brain_service(&env, "acme", &runner),
        Ok(Some(r"\Kluris\sync-acme".to_string()))
    );
    assert_eq!(
        runner.take(),
        [
            r"schtasks /Query /TN \Kluris\sync-acme",
            r"schtasks /End /TN \Kluris\sync-acme",
            r"schtasks /Delete /TN \Kluris\sync-acme /F",
        ]
    );
}

#[test]
fn a_service_that_will_not_stop_is_an_error_and_keeps_its_definition() {
    let s = Sandbox::new();
    let env = s.env(Manager::Launchd);
    let plist = env.definition("acme").unwrap();
    fs::create_dir_all(plist.parent().unwrap()).unwrap();
    fs::write(&plist, "<plist/>").unwrap();
    let runner = FakeRunner::default();
    runner.answer(
        &format!("launchctl print gui/501/{LABEL}"),
        0,
        "\tstate = running\n",
        "",
    );
    runner.answer(
        &format!("launchctl bootout gui/501/{LABEL}"),
        1,
        "",
        "Boot-out failed: 1: Operation not permitted\n",
    );
    let err = service::remove_brain_service(&env, "acme", &runner).unwrap_err();
    assert!(err.to_string().contains("Operation not permitted"), "{err}");
    assert!(plist.exists());
}

/// A fake `launchctl` (macOS) or `systemctl` (Linux) first on PATH: it
/// records its arguments and answers every service as running, or as
/// stopped when `stopped` is set.
#[cfg(unix)]
fn fake_tools(dir: &Path, stopped: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    let log = dir.join("tool-calls");
    let (launchctl_state, systemctl_state, code) = if stopped {
        ("not running", "inactive", 3)
    } else {
        ("running", "active", 0)
    };
    // `print` finds a label only while its plist is in LaunchAgents.
    let launchctl = format!(
        "#!/bin/sh\necho \"launchctl $*\" >> '{}'\ncase \"$1\" in print)\n  [ -f \"$HOME/Library/LaunchAgents/${{2##*/}}.plist\" ] || exit 113\n  printf '\\tstate = {launchctl_state}\\n';;\nbootout) [ -z \"$FAKE_STOP_FAILS\" ] || {{ echo 'Boot-out failed: 1: Operation not permitted' >&2; exit 1; }};;\nesac\nexit 0\n",
        log.display()
    );
    let systemctl = format!(
        "#!/bin/sh\necho \"systemctl $*\" >> '{}'\ncase \"$2\" in is-active) echo {systemctl_state}; exit {code};;\ndisable) [ -z \"$FAKE_STOP_FAILS\" ] || {{ echo 'Failed to disable unit: Access denied' >&2; exit 1; }};; esac\nexit 0\n",
        log.display()
    );
    for (name, text) in [("launchctl", launchctl), ("systemctl", systemctl)] {
        let path = bin.join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// `kluris remove acme` with acme's service installed, the fake
/// `launchctl`/`systemctl` first on PATH; `stop_fails` makes its stop fail.
#[cfg(unix)]
fn remove_with_a_service(stop_fails: bool, json: bool) -> (World, PathBuf, i32, String, String) {
    let w = World::new();
    let definition = if cfg!(target_os = "macos") {
        w.home()
            .join("Library/LaunchAgents/dev.ngvoicu.kluris.sync.acme.plist")
    } else {
        w.home()
            .join(".config/systemd/user/kluris-sync-acme.service")
    };
    fs::create_dir_all(definition.parent().unwrap()).unwrap();
    fs::write(&definition, "x").unwrap();
    let bin = fake_tools(w.home(), false);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let mut args = vec!["remove", "acme"];
    if json {
        args.push("--json");
    }
    let mut cmd = w.command(&args);
    cmd.env("PATH", path);
    if stop_fails {
        cmd.env("FAKE_STOP_FAILS", "1");
    }
    let out = cmd.output().unwrap();
    let (out_s, err_s) = (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    );
    (w, definition, out.status.code().unwrap(), out_s, err_s)
}

#[cfg(unix)]
fn service_name() -> &'static str {
    if cfg!(target_os = "macos") {
        LABEL
    } else {
        "kluris-sync-acme.service"
    }
}

#[cfg(unix)]
fn registered(w: &World) -> bool {
    fs::read_to_string(w.home().join(".kluris").join("config.yml"))
        .unwrap()
        .contains("acme:")
}

#[cfg(unix)]
#[test]
fn remove_stops_and_removes_the_brains_sync_service() {
    let (w, definition, code, out, err) = remove_with_a_service(false, true);
    assert_eq!(code, 0, "{out}{err}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        doc,
        serde_json::json!({"ok": true, "name": "acme", "service": service_name()})
    );
    assert!(!definition.exists());
    assert!(!registered(&w));
    let calls = fs::read_to_string(w.home().join("tool-calls")).unwrap();
    assert!(
        calls.contains("bootout") || calls.contains("disable --now"),
        "{calls}"
    );
    // One world at a time: this one holds the lock.
    drop(w);

    let (_, _, code, out, err) = remove_with_a_service(false, false);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains(&format!(
            "Stopped and removed the sync service {}",
            service_name()
        )),
        "{out}"
    );
    assert!(
        out.contains("Unregistered: acme (files preserved)"),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn remove_unregisters_even_when_the_service_will_not_stop() {
    let (w, definition, code, out, err) = remove_with_a_service(true, true);
    assert_eq!(code, 0, "{out}{err}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["name"], "acme");
    assert_eq!(doc.get("service"), None);
    let failed = doc["service_failed"].as_str().unwrap();
    assert!(failed.contains(service_name()), "{failed}");
    assert!(
        failed.contains("kluris sync --uninstall-service --brain acme"),
        "{failed}"
    );
    assert!(definition.exists());
    assert!(!registered(&w));
    // One world at a time: this one holds the lock.
    drop(w);

    let (_, _, code, out, err) = remove_with_a_service(true, false);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("Unregistered: acme (files preserved)"),
        "{out}"
    );
    assert!(err.contains("could not stop the sync service"), "{err}");
    assert!(
        err.contains("kluris sync --uninstall-service --brain acme"),
        "{err}"
    );
}

#[test]
fn remove_without_a_service_keeps_v2s_json() {
    let w = World::new();
    let (code, out, err) = w.run(&["remove", "acme", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc, serde_json::json!({"ok": true, "name": "acme"}));
}

#[cfg(unix)]
fn doctor_with_a_service(stopped: bool) -> (i32, Value, String) {
    let w = World::new();
    let definition = if cfg!(target_os = "macos") {
        w.home()
            .join("Library/LaunchAgents/dev.ngvoicu.kluris.sync.acme.plist")
    } else {
        w.home()
            .join(".config/systemd/user/kluris-sync-acme.service")
    };
    fs::create_dir_all(definition.parent().unwrap()).unwrap();
    fs::write(&definition, "x").unwrap();
    let bin = fake_tools(w.home(), stopped);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let mut cmd = w.command(&["doctor", "--no-refresh", "--json"]);
    cmd.env("PATH", path);
    let out = cmd.output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    let calls = fs::read_to_string(w.home().join("tool-calls")).unwrap_or_default();
    (out.status.code().unwrap(), doc, calls)
}

#[cfg(unix)]
#[test]
fn doctor_lists_a_running_sync_service() {
    let (code, doc, calls) = doctor_with_a_service(false);
    assert_eq!(code, 0, "{doc}");
    let check = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sync-service:acme")
        .unwrap_or_else(|| panic!("{doc}"));
    assert_eq!(check["passed"], true);
    assert_eq!(check.get("warning"), None);
    assert!(
        check["detail"].as_str().unwrap().starts_with("running"),
        "{check}"
    );
    // Only asked, never changed.
    for verb in [
        "bootstrap",
        "bootout",
        "kickstart",
        "enable",
        "disable",
        "restart",
    ] {
        assert!(!calls.contains(verb), "{calls}");
    }
}

#[cfg(unix)]
#[test]
fn doctor_warns_about_a_stopped_sync_service_without_failing() {
    let (code, doc, _) = doctor_with_a_service(true);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["ok"], true);
    let check = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sync-service:acme")
        .unwrap_or_else(|| panic!("{doc}"));
    assert_eq!(check["passed"], true);
    assert_eq!(check["warning"], true);
    assert!(
        check["detail"]
            .as_str()
            .unwrap()
            .starts_with("installed, not running"),
        "{check}"
    );
}

#[test]
fn doctor_without_sync_services_has_no_sync_check() {
    let w = World::new();
    let (code, out, err) = w.run(&["doctor", "--no-refresh", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(!out.contains("sync-service"), "{out}");
}

#[cfg(unix)]
#[test]
fn install_and_uninstall_through_the_binary() {
    let w = World::new();
    let bin = fake_tools(w.home(), false);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let run = |args: &[&str]| {
        let mut cmd = w.command(args);
        cmd.env("PATH", &path);
        let out = cmd.output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    let (code, out, err) = run(&["sync", "--install-service", "--brain", "acme", "--json"]);
    assert_eq!(code, 0, "{out}{err}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["brain"], "acme");
    let definition = PathBuf::from(doc["definition"].as_str().unwrap());
    assert!(definition.starts_with(w.home()), "{doc}");
    let text = fs::read_to_string(&definition).unwrap();
    let exe = fs::canonicalize(assert_cmd::cargo::cargo_bin!("kluris")).unwrap();
    assert!(text.contains(&exe.display().to_string()), "{text}");
    // KLURIS_CONFIG goes with it, so the service finds this brain.
    assert!(
        text.contains(
            &w.home()
                .join(".kluris")
                .join("config.yml")
                .display()
                .to_string()
        ),
        "{text}"
    );
    let calls = fs::read_to_string(w.home().join("tool-calls")).unwrap();
    assert!(
        calls.contains("bootstrap") || calls.contains("enable --now"),
        "{calls}"
    );

    let (code, out, err) = run(&["sync", "--uninstall-service", "--brain", "acme"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("Stopped and removed the sync service"),
        "{out}"
    );
    assert!(!definition.exists());
    // Removing what is not there is fine.
    let (code, out, err) = run(&["sync", "--uninstall-service", "--brain", "acme"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(
        out.contains("No sync service for acme is installed"),
        "{out}"
    );
}

/// F163-4: a service test that fails after its install leaves no service:
/// as the panic unwinds, the guard uninstalls the test's brain's service
/// (through the fake `launchctl`/`systemctl`).
#[cfg(unix)]
#[test]
fn a_failing_service_test_leaves_no_service_installed() {
    let w = World::new();
    let bin = fake_tools(w.home(), false);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let run = |args: &[&str]| {
        let mut cmd = w.command(args);
        cmd.env("PATH", &path);
        ran(cmd)
    };
    let definition = Mutex::new(None);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (_service, (code, out, err)) = TestService::install("acme", &run);
        assert_eq!(code, 0, "{out}{err}");
        let doc: Value = serde_json::from_str(&out).unwrap();
        let installed = PathBuf::from(doc["definition"].as_str().unwrap());
        assert!(installed.exists());
        *definition.lock().unwrap() = Some(installed);
        panic!("injected: the test fails after its install");
    }));
    assert!(
        failed.is_err(),
        "the injected failure did not fail the test"
    );
    let definition = definition.into_inner().unwrap().expect("installed");
    assert!(
        !definition.exists(),
        "the service outlived its failed test: {}",
        definition.display()
    );
    let calls = fs::read_to_string(w.home().join("tool-calls")).unwrap();
    assert!(
        calls.contains("bootout") || calls.contains("disable"),
        "{calls}"
    );
}

// ---------------------------------------------------------------------------
// The opt-in real round trip.
// ---------------------------------------------------------------------------

/// A command's exit code (-1 when it has none or could not start), stdout
/// and stderr. It never panics, so a guard can run it while unwinding.
fn ran(mut cmd: StdCommand) -> (i32, String, String) {
    match cmd.output() {
        Ok(out) => (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ),
        Err(e) => (-1, String::new(), e.to_string()),
    }
}

/// `kluris` run with a test's environment: exit code, stdout, stderr.
type Kluris<'a> = &'a dyn Fn(&[&str]) -> (i32, String, String);

/// A sync service a test installs, for that test's brain alone, through
/// `kluris` (the test's own environment). Dropped, so on every unwind path
/// too, it uninstalls that brain's service (never another), stops the watch
/// processes it recorded for it that are still running, each by its PID,
/// and removes the service's log.
///
/// No guard runs when the test process itself is killed: the service then
/// stays installed and running, with its log; `kluris sync
/// --uninstall-service --brain <name>` with the test's `KLURIS_CONFIG`
/// removes it (`no_round_trip_leftovers` finds a round trip's).
struct TestService<'a> {
    brain: String,
    kluris: Kluris<'a>,
    log: Option<PathBuf>,
    recorded: Vec<u32>,
    uninstalled: bool,
}

impl<'a> TestService<'a> {
    /// The guard, then the install (`--json`), so a failed or half-done
    /// install is undone too.
    fn install(brain: &str, kluris: Kluris<'a>) -> (TestService<'a>, (i32, String, String)) {
        let mut service = TestService {
            brain: brain.to_string(),
            kluris,
            log: None,
            recorded: Vec::new(),
            uninstalled: false,
        };
        let installed = kluris(&["sync", "--install-service", "--brain", brain, "--json"]);
        service.log = serde_json::from_str::<Value>(&installed.1)
            .ok()
            .and_then(|doc| doc["log"].as_str().map(PathBuf::from));
        (service, installed)
    }

    /// Record the watch processes running for this brain: those whose
    /// `--brain` is exactly its name. Only for a name no other run uses
    /// (the round trip's `kluris-rt-<pid>-<nonce>`).
    fn record(&mut self) {
        for pid in watch_pids(&self.brain) {
            if !self.recorded.contains(&pid) {
                self.recorded.push(pid);
            }
        }
    }

    fn uninstall(&mut self) -> (i32, String, String) {
        self.uninstalled = true;
        (self.kluris)(&["sync", "--uninstall-service", "--brain", &self.brain])
    }
}

impl Drop for TestService<'_> {
    fn drop(&mut self) {
        if !self.uninstalled {
            let (code, out, err) = self.uninstall();
            eprintln!("the guard uninstalled {}: {code} {out}{err}", self.brain);
        }
        // What the service manager left of the watches recorded, each
        // still a watch for this brain (not a reused id).
        for &pid in &self.recorded {
            if watch_pids(&self.brain).contains(&pid) {
                eprintln!("the guard stops watch {pid} of {}", self.brain);
                end_pid(pid, &self.brain);
            }
        }
        // The log, then its folders once empty: `kluris` (macOS),
        // `kluris\logs` (Windows).
        if let Some(log) = &self.log {
            let _ = fs::remove_file(log);
            let mut dir = log.parent();
            while let Some(d) = dir.filter(|d| d.ends_with("kluris") || d.ends_with("logs")) {
                if fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
    }
}

/// Stop the watch `pid` of `brain`, recorded as this run's: TERM first on
/// Unix (its clean stop), then [`stop_pid`] if it is still that watch.
fn end_pid(pid: u32, brain: &str) {
    #[cfg(unix)]
    {
        let _ = StdCommand::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stderr(Stdio::null())
            .status();
        let until = Instant::now() + Duration::from_secs(10);
        while watch_pids(brain).contains(&pid) && Instant::now() < until {
            thread::sleep(Duration::from_millis(100));
        }
    }
    if watch_pids(brain).contains(&pid) {
        stop_pid(pid);
    }
}

/// Installs a real service for a throwaway brain (`kluris-rt-<pid>-<nonce>`,
/// its config and remote in a temp folder), waits until it runs and has
/// pushed a local commit, uninstalls it, then checks nothing is left: the
/// definition, the service manager's entry, a watch process. Its guard
/// uninstalls the service and stops the watches it recorded however the
/// test ends.
/// `cargo test -p kluris --test watch -- --ignored real_service_round_trip`.
#[test]
#[ignore = "installs a real per-user service on this machine"]
fn real_service_round_trip() {
    let w = World::new();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let name = format!("kluris-rt-{}-{nonce}", std::process::id());
    // The brain under the throwaway name.
    let config = w.home().join(".kluris").join("config.yml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(&config, text.replace("  acme:", &format!("  {name}:"))).unwrap();
    w.commit_locally();
    let exe = fs::canonicalize(assert_cmd::cargo::cargo_bin!("kluris")).unwrap();
    let real = |args: &[&str]| {
        // The real HOME and service manager; only KLURIS_CONFIG and git's
        // config are the sandbox's.
        let mut cmd = StdCommand::new(&exe);
        cmd.args(args)
            .env("KLURIS_CONFIG", &config)
            .env("GIT_CONFIG_GLOBAL", w.home().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("KLURIS_NO_PROMPT", "1");
        ran(cmd)
    };
    let (mut service, (code, out, err)) = TestService::install(&name, &real);
    assert_eq!(code, 0, "install failed: {out}{err}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    eprintln!("installed: {doc}");
    let log = service.log.clone();
    let definition = doc["definition"].as_str().map(PathBuf::from);

    // It runs: doctor says so, it pushes the local commit, and its log
    // (where there is one) says so once the push is done.
    let read_log = || {
        log.as_ref()
            .and_then(|l| fs::read_to_string(l).ok())
            .unwrap_or_default()
    };
    let logged = || log.is_none() || read_log().contains(&format!("{name}: pushed"));
    let until = Instant::now() + Duration::from_secs(120);
    let mut running = false;
    let mut pushed = false;
    while Instant::now() < until && !(running && pushed && logged()) {
        service.record();
        let (_, out, _) = real(&["doctor", "--no-refresh", "--json"]);
        let doc: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
        running = doc["checks"].as_array().is_some_and(|checks| {
            checks.iter().any(|c| {
                c["name"] == format!("sync-service:{name}")
                    && c["detail"]
                        .as_str()
                        .is_some_and(|d| d.starts_with("running"))
            })
        });
        pushed = w.remote_head() == w.head();
        thread::sleep(Duration::from_millis(500));
    }
    service.record();
    let log_text = read_log();
    let was_logged = logged();
    let (code, out, err) = service.uninstall();
    eprintln!("uninstall: {code} {out}{err}");
    assert!(running, "the service never ran; log: {log_text}");
    assert!(pushed, "the service never pushed; log: {log_text}");
    assert!(was_logged, "the push was never logged: {log_text}");
    assert_eq!(code, 0, "{out}{err}");

    // Nothing is left.
    if let Some(definition) = definition {
        assert!(!definition.exists(), "{}", definition.display());
    }
    let (_, out, _) = real(&["doctor", "--no-refresh", "--json"]);
    assert!(!out.contains(&format!("sync-service:{name}")), "{out}");
    thread::sleep(Duration::from_secs(2));
    service.record();
    let left = watch_pids(&name);
    assert!(
        left.is_empty(),
        "a watch for {name} was still running after the uninstall (the guard stops it): {left:?}"
    );
}

/// Nothing of any real round trip is on this machine: no service
/// definition, task or unit for a `kluris-rt-*` brain, no watch for one, no
/// log of one. It only looks; what it finds is removed by hand.
/// `-- --test watch -- --ignored no_round_trip_leftovers`.
#[test]
#[ignore = "looks at this machine's real services"]
fn no_round_trip_leftovers() {
    let exe = assert_cmd::cargo::cargo_bin!("kluris");
    let out = StdCommand::new(exe)
        .args(["doctor", "--no-refresh", "--json"])
        .env("KLURIS_NO_AUTO_SYNC", "1")
        .output()
        .unwrap();
    let doctor = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(!doctor.contains("sync-service:kluris-rt-"), "{doctor}");
    let mut found = Vec::new();
    let home = std::env::home_dir().unwrap();
    let mut folders = vec![
        home.join("Library").join("LaunchAgents"),
        home.join("Library").join("Logs").join("kluris"),
        home.join(".config").join("systemd").join("user"),
    ];
    if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
        folders.push(PathBuf::from(lad).join("kluris").join("logs"));
    }
    for folder in folders {
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().contains("kluris-rt-") {
                found.push(entry.path().display().to_string());
            }
        }
    }
    if cfg!(windows) {
        let tasks = StdCommand::new("schtasks")
            .args(["/Query", "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        let tasks = String::from_utf8_lossy(&tasks.stdout).into_owned();
        found.extend(
            tasks
                .lines()
                .filter(|l| l.contains("kluris-rt-"))
                .map(str::to_string),
        );
    }
    found.extend(
        processes()
            .into_iter()
            .filter(|(_, line)| is_watch(line) && line.contains("--brain kluris-rt-"))
            .map(|(pid, line)| format!("watch {pid}: {line}")),
    );
    assert!(found.is_empty(), "left behind: {found:#?}");
    eprintln!("no round trip leftovers");
}

/// Every process on this machine: its id and command line.
fn processes() -> Vec<(u32, String)> {
    let out = if cfg!(windows) {
        StdCommand::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }",
            ])
            .output()
    } else {
        StdCommand::new("ps")
            .args(["-A", "-o", "pid=,args="])
            .output()
    };
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, command.trim().to_string()))
        })
        .collect()
}

/// Whether a command line is a `kluris sync --watch`: kluris itself (or the
/// conhost a Windows task runs it under), not a shell or a search whose
/// command line quotes the words.
fn is_watch(line: &str) -> bool {
    let line = line.trim_start();
    let first = match line.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        None => line.split_whitespace().next().unwrap_or_default(),
    };
    let name = first.rsplit(['/', '\\']).next().unwrap_or_default();
    ["kluris", "kluris.exe", "conhost.exe"].contains(&name.to_ascii_lowercase().as_str())
        && line.contains("--watch")
}

/// The watch processes whose `--brain` is exactly `brain`.
fn watch_pids(brain: &str) -> Vec<u32> {
    processes()
        .into_iter()
        .filter(|(_, line)| {
            is_watch(line) && {
                let words: Vec<&str> = line
                    .split_whitespace()
                    .map(|w| w.trim_matches('"'))
                    .collect();
                words.windows(2).any(|pair| pair == ["--brain", brain])
            }
        })
        .map(|(pid, _)| pid)
        .collect()
}
