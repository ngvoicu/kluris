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
//! - after a conflict, a block or a refusal, the heads probed before it,
//!   and its local state ([`WatchGit::local_state`]: one `git status` and
//!   the operation markers). It is reported once. Each tick it is synced
//!   again when a head moved (a `kluris resolve`, a commit, a teammate's
//!   push) or its local state changed (an edit undone, a rebase aborted, an
//!   open conflict dropped); and, as some blockers clear without either (an
//!   ignored file removed, a `pre-push` hook repaired), on a backoff:
//!   [`RETRY_FIRST`] after it was parked, the wait doubling up to
//!   [`RETRY_MAX`], starting over when a head moves or the local state
//!   changes. A sync no head move started that finds the same reason
//!   prints nothing;
//! - after a sync that found the remote unreachable, where it was: the
//!   next tick tries again.
//!
//! Offline (the remote does not answer) is quiet: nothing is reported. A
//! branch without an upstream is reported once, and so is a brain that is
//! not a git repository (one sync runs and says so). The first tick runs at
//! once; then every [`WatchRequest::interval`], waited in short slices so a
//! stop is seen at once. A stop ends the loop between two brains, never
//! during a sync.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::SyncReport;
use super::fresh::{FreshError, IN_PROGRESS, prompt_env};
use super::plumbing::SyncGit;
use crate::git::{Git, Plumbing, SystemGit};

/// The interval when none is given, in seconds.
pub const DEFAULT_INTERVAL_SECS: u64 = 30;

/// The shortest interval, in seconds.
pub const MIN_INTERVAL_SECS: u64 = 5;

/// A parked brain's first real sync again, whatever its status says.
pub const RETRY_FIRST: Duration = Duration::from_secs(60);

/// The longest wait between two of those syncs (the wait doubles).
pub const RETRY_MAX: Duration = Duration::from_secs(600);

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
    /// The brain is not in a git work tree: one sync says so.
    NotARepo,
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

    /// What a person changes to clear a block or a refusal, without
    /// moving a head: the files `git status` lists (untracked included)
    /// and which operation markers exist (a merge, rebase, cherry-pick or
    /// revert in progress, an open sync conflict). Compared from tick to
    /// tick; `None` when it cannot be read. One cheap call, never a sync.
    fn local_state(&self, root: &Path, deadline: Instant) -> Option<String>;
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
    /// The brain waits for a person: why, and its local state then.
    parked: Option<Parked>,
}

/// A conflict, a block or a refusal, the local state it was found in, and
/// when to sync again though nothing seems to have changed.
struct Parked {
    reason: Reason,
    local: Option<String>,
    /// The wait before the next retry was scheduled.
    backoff: Duration,
    retry_at: Instant,
}

/// What made a parked brain's sync run.
#[derive(PartialEq)]
enum Trigger {
    /// A head moved (or the brain was not parked).
    Head,
    /// Its local state changed.
    Local,
    /// The backoff ran out.
    Retry,
}

/// What a person must see to, as it was reported.
#[derive(PartialEq)]
enum Reason {
    Conflicts(Vec<super::Conflict>),
    Blocked(Vec<String>),
    Refused(String),
}

impl Reason {
    /// The reason of a parked outcome ([`parked`]).
    fn of(outcome: &Result<SyncReport, String>) -> Reason {
        match outcome {
            Ok(report) if !report.conflicts.is_empty() => {
                Reason::Conflicts(report.conflicts.clone())
            }
            Ok(report) => Reason::Blocked(report.blocked.clone()),
            Err(why) => Reason::Refused(why.clone()),
        }
    }
}

/// Whether a sync's outcome is a problem a person must see to (a conflict,
/// a block, a refusal or a failure): the brain is synced again once a head
/// moves or its local state changes.
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
        // No heads to compare: an empty pair stands for "not a repository",
        // so the refusal is reported once and again only once it is one.
        Probe::NotARepo => Heads {
            local: String::new(),
            remote: String::new(),
        },
    };
    state.told_no_upstream = false;
    let deadline = || clock.now() + request.probe_time_limit;
    // Neither head moved: a brain in sync rests; one that waits for a
    // person is synced again if its local state changed (the fast path), or
    // when its backoff runs out, as some blockers clear unseen by `git
    // status` (an ignored file removed, a hook repaired).
    let trigger = if state.left_at.as_ref() != Some(&probed) {
        Trigger::Head
    } else {
        let Some(parked) = &mut state.parked else {
            return;
        };
        if git.local_state(brain.root, deadline()) != parked.local {
            Trigger::Local
        } else if clock.now() >= parked.retry_at {
            parked.backoff = (parked.backoff * 2).min(RETRY_MAX);
            parked.retry_at = clock.now() + parked.backoff;
            Trigger::Retry
        } else {
            return;
        }
    };
    let outcome = sync(brain);
    if quiet(&outcome) {
        return;
    }
    if parked(&outcome) {
        let reason = Reason::of(&outcome);
        // The state the reason was found in, read before the line goes out:
        // what a person does on seeing it is then a change.
        let local = git.local_state(brain.root, deadline());
        // A sync no head move started that finds the same reason says
        // nothing new.
        let same =
            trigger != Trigger::Head && state.parked.as_ref().is_some_and(|p| p.reason == reason);
        if !same {
            report(brain, Report::Synced(&outcome));
        }
        // A head move or a local change starts the backoff over.
        let (backoff, retry_at) = match (&trigger, &state.parked) {
            (Trigger::Retry, Some(p)) => (p.backoff, p.retry_at),
            _ => (RETRY_FIRST, clock.now() + RETRY_FIRST),
        };
        state.parked = Some(Parked {
            reason,
            local,
            backoff,
            retry_at,
        });
        state.left_at = Some(probed);
        return;
    }
    state.parked = None;
    report(brain, Report::Synced(&outcome));
    state.left_at = if matches!(&outcome, Ok(r) if r.offline) {
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
            return Probe::NotARepo;
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

    fn local_state(&self, root: &Path, deadline: Instant) -> Option<String> {
        let args = [
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ];
        let out = self.checked(root, &args, &[], deadline).ok()?;
        let mut state = String::from_utf8_lossy(&out).into_owned();
        if let Some(dir) = git_dir(root) {
            for marker in IN_PROGRESS {
                let path = marker.split('/').fold(dir.clone(), |p, part| p.join(part));
                if fs::symlink_metadata(path).is_ok() {
                    state.push('\0');
                    state.push_str(marker);
                }
            }
        }
        Some(state)
    }
}

/// The git directory of a brain at the root of its work tree, read from
/// its `.git` (a folder, or a linked worktree's `gitdir:` file) without
/// running git: every operation marker and an open sync conflict are in it
/// (`git rev-parse --git-path` gives the same folder for each).
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if fs::metadata(&dot).ok()?.is_dir() {
        return Some(dot);
    }
    let text = fs::read_to_string(&dot).ok()?;
    let path = PathBuf::from(text.lines().next()?.strip_prefix("gitdir:")?.trim());
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}
