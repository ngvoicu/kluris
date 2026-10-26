//! `kluris sync --watch` (SPEC Phase 7, TEST-K3-51): keep brains in sync
//! while people work.
//!
//! Each tick looks at every brain without changing anything ([`WatchGit::
//! probe`]): its local `HEAD` and the remote's branch (`git ls-remote`,
//! never prompting, within the probe time limit). When either differs from
//! what the brain was left at, one [`super::sync`] runs (the caller's
//! `sync`), and the brain is left at:
//!
//! - after a clean sync, what it settled to ([`WatchGit::settled`]: `HEAD`
//!   and the upstream's remote-tracking ref), so its own push is not a
//!   change the next tick;
//! - after a conflict, a block or a refusal, the heads probed before it: it
//!   is reported once and not synced again until one of them moves (a
//!   `kluris resolve`, a commit, a teammate's push);
//! - after a sync that found the remote unreachable, where it was: the
//!   next tick tries again.
//!
//! Offline (the remote does not answer) is quiet: nothing is reported. A
//! branch without an upstream is reported once. The first tick runs at
//! once; then every [`WatchRequest::interval`], waited in short slices so a
//! stop is seen at once. A stop ends the loop between two brains, never
//! during a sync.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::SyncReport;
use super::fresh::{FreshError, prompt_env};
use super::plumbing::SyncGit;
use crate::git::{Git, Plumbing, SystemGit};

/// The interval when none is given, in seconds.
pub const DEFAULT_INTERVAL_SECS: u64 = 30;

/// The shortest interval, in seconds.
pub const MIN_INTERVAL_SECS: u64 = 5;

/// The time a probe of one brain may take.
pub const PROBE_TIME_LIMIT: Duration = Duration::from_secs(15);

/// The longest single sleep while waiting for the next tick.
const SLICE: Duration = Duration::from_millis(200);

/// Where a brain is: its `HEAD` and the remote's branch (empty when the
/// remote has no such branch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heads {
    pub local: String,
    pub remote: String,
}

/// What a look at a brain found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Heads(Heads),
    /// A detached `HEAD`, or a branch without an upstream on a remote.
    NoUpstream,
    /// The remote did not answer in time, or the brain could not be read.
    Offline,
}

/// The git steps of the watch.
pub trait WatchGit {
    /// The brain's heads, by `deadline`; never prompts, never changes
    /// anything.
    fn probe(&self, root: &Path, deadline: Instant) -> Probe;

    /// After a sync: `HEAD` and the upstream's remote-tracking ref; `None`
    /// when either cannot be read.
    fn settled(&self, root: &Path) -> Option<Heads>;
}

/// The time the loop waits by.
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, d: Duration);
}

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// A brain to watch.
#[derive(Debug, Clone, Copy)]
pub struct Watched<'a> {
    pub name: &'a str,
    pub root: &'a Path,
}

#[derive(Debug, Clone, Copy)]
pub struct WatchRequest {
    pub interval: Duration,
    pub probe_time_limit: Duration,
}

/// What the loop reports for a brain.
#[derive(Debug, Clone, Copy)]
pub enum Report<'a> {
    /// A sync ran: its report, or why the brain was refused or the sync
    /// failed.
    Synced(&'a Result<SyncReport, String>),
    /// The brain has no upstream to sync with (reported once).
    NoUpstream,
}

/// `--interval`'s value, as Click's `IntRange(min=5)` checks it.
pub fn parse_interval(raw: &str) -> Result<Duration, String> {
    let value: u64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("'{raw}' is not a valid integer."))?;
    if value < MIN_INTERVAL_SECS {
        return Err(format!(
            "{value} is not in the range x>={MIN_INTERVAL_SECS}."
        ));
    }
    Ok(Duration::from_secs(value))
}

/// What the loop knows of one brain.
#[derive(Default)]
struct State {
    /// Where the brain was left; a probe that differs runs a sync.
    left_at: Option<Heads>,
    /// "No upstream" was reported and nothing changed since.
    told_no_upstream: bool,
}

/// Whether a sync's outcome is a problem a person must see to (a conflict,
/// a block, a refusal or a failure): the brain is not synced again until a
/// head moves.
fn parked(outcome: &Result<SyncReport, String>) -> bool {
    match outcome {
        Ok(report) => !report.conflicts.is_empty() || !report.blocked.is_empty(),
        Err(_) => true,
    }
}

/// Whether a sync's outcome found the remote unreachable before doing
/// anything: quiet, and tried again next tick.
fn quiet(outcome: &Result<SyncReport, String>) -> bool {
    matches!(outcome, Ok(report) if report.offline && !report.merged && !parked(outcome))
}

/// Watch `brains` until `stop` is set (see the module docs).
pub fn watch(
    brains: &[Watched],
    git: &impl WatchGit,
    clock: &impl Clock,
    stop: &AtomicBool,
    request: &WatchRequest,
    sync: &mut dyn FnMut(&Watched) -> Result<SyncReport, String>,
    report: &mut dyn FnMut(&Watched, Report),
) {
    let stopped = || stop.load(Ordering::SeqCst);
    let mut states: Vec<State> = brains.iter().map(|_| State::default()).collect();
    while !stopped() {
        for (brain, state) in brains.iter().zip(states.iter_mut()) {
            if stopped() {
                return;
            }
            tick(brain, state, git, clock, request, sync, report);
        }
        let started = clock.now();
        loop {
            if stopped() {
                return;
            }
            let waited = clock.now().saturating_duration_since(started);
            let Some(left) = request
                .interval
                .checked_sub(waited)
                .filter(|d| !d.is_zero())
            else {
                break;
            };
            clock.sleep(left.min(SLICE));
        }
    }
}

fn tick(
    brain: &Watched,
    state: &mut State,
    git: &impl WatchGit,
    clock: &impl Clock,
    request: &WatchRequest,
    sync: &mut dyn FnMut(&Watched) -> Result<SyncReport, String>,
    report: &mut dyn FnMut(&Watched, Report),
) {
    let probed = match git.probe(brain.root, clock.now() + request.probe_time_limit) {
        Probe::Offline => return,
        Probe::NoUpstream => {
            if !state.told_no_upstream {
                report(brain, Report::NoUpstream);
                state.told_no_upstream = true;
            }
            state.left_at = None;
            return;
        }
        Probe::Heads(heads) => heads,
    };
    state.told_no_upstream = false;
    if state.left_at.as_ref() == Some(&probed) {
        return;
    }
    let outcome = sync(brain);
    if quiet(&outcome) {
        return;
    }
    report(brain, Report::Synced(&outcome));
    state.left_at = if parked(&outcome) {
        Some(probed)
    } else if matches!(&outcome, Ok(r) if r.offline) {
        // Pulled, but the push found the remote unreachable: try again.
        state.left_at.take()
    } else {
        git.settled(brain.root).or(Some(probed))
    };
}

fn first_word(out: &[u8]) -> String {
    String::from_utf8_lossy(out)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

impl WatchGit for SystemGit {
    fn probe(&self, root: &Path, deadline: Instant) -> Probe {
        let Some(repo) = self.repo(root) else {
            return Probe::Offline;
        };
        let tracking = match self.tracking(&repo) {
            Ok(Some(tracking)) => tracking,
            Ok(None) => return Probe::NoUpstream,
            Err(_) => return Probe::Offline,
        };
        let Ok(Some(local)) = self.head(&repo) else {
            return Probe::Offline;
        };
        let ssh = self.bounded(
            &repo.dir,
            &["config", "--get", "core.sshCommand"],
            &[],
            deadline,
        );
        let configured = ssh
            .as_ref()
            .is_ok_and(|done| done.status.success() && !done.stdout.trim_ascii().is_empty());
        let env = prompt_env(|key| self.env_var(key), configured);
        let args = [
            "ls-remote",
            "--quiet",
            "--refs",
            &tracking.remote,
            &tracking.remote_ref,
        ];
        match self.bounded(&repo.dir, &args, &env, deadline) {
            Ok(done) if done.status.success() => Probe::Heads(Heads {
                local,
                remote: first_word(&done.stdout),
            }),
            Ok(_) | Err(FreshError::TimedOut | FreshError::Failed(_)) => Probe::Offline,
        }
    }

    fn settled(&self, root: &Path) -> Option<Heads> {
        let repo = self.repo(root)?;
        let tracking = self.tracking(&repo).ok()??;
        let local = self.head(&repo).ok()??;
        let remote = self.rev_parse(&repo, &tracking.refname).ok()?;
        Some(Heads { local, remote })
    }
}
