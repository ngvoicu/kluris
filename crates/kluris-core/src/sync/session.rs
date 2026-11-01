//! The session: the one step through which every command that opens a
//! brain syncs. [`open`] brings teammates' commits in before the command
//! works; [`close`] ends it, pushing what an explicit command committed.
//! [`open_all`] opens many brains at once, and [`close_all`] closes them.
//!
//! What a session does depends on its [`Intent`]:
//! - **A read** (`wake-up`, `search`, `status`, `register`, `doctor`) and
//!   **housekeeping** (`dream`, `mri`) fetch at most once per
//!   [`THROTTLE`] per brain (the stamp, [`STAMP`]) and never inside the
//!   back-off ([`BACKOFF`]); a fetch stopped at its bound starts one. Both
//!   files are in the repository's common git directory, like the remote
//!   claim, so one fetch throttles every linked worktree. Every git the
//!   read runs before it reads stops at [`READ_TIME_LIMIT`]: the queries
//!   that open the brain, the fetch, and the pull's local steps up to the
//!   land, its merge commit and that commit's hooks included (each with
//!   every process it started), and the land's own checks before it
//!   writes. The land itself, and what follows it, is never stopped: a
//!   land stopped partway would leave the work tree half moved. A step
//!   stopped at the limit leaves the brain as it is (`busy`); one that
//!   fails on its own is `failed`, whatever it says. The stop is git's
//!   own outcome ([`LeftAsIs::stopped`]), never read from what git says.
//!   A read never waits for the remote claim another command holds: it
//!   is `busy` at once.
//! - **An explicit** command (`commit`, `sync`, `resolve`, `create
//!   --remote`) always fetches, and its fetch and push each have
//!   [`NETWORK_TIME_LIMIT`]. It waits for the remote claim another command
//!   holds up to [`COMMAND_BOUND`], for its fetch, its push and every pull
//!   the push runs again.
//!
//! **Closing.** An explicit session pushes what is committed. The commits
//! the reads closed together leave waiting go to one background push for
//! all their brains ([`background`]), the program [`SessionEnv::program`]
//! names, unless that brain's last push failed lately or a background run
//! holds it. Every session reports the last background push as its record
//! says: a failure until a push succeeds, a success once.
//!
//! A pull that does not fetch lands nothing: only the commit this
//! invocation's own `ls-remote` named may land, never what the shared
//! remote-tracking ref holds. Inside the throttle or the back-off a read
//! says nothing; while another command holds the remote claim it is
//! `busy`.
//!
//! **The tree claim.** A read lands under the exclusive tree claim, waited
//! for up to [`READ_BOUND`] (`busy` past it, and the brain is read as it
//! is), then turns it shared ([`TreeClaim::downgrade`]) and holds it until
//! [`close`], so no land starts while the command reads; a read that
//! lands nothing takes it shared. A file lock cannot be turned shared in
//! place, so the claim is unlocked and locked shared again; the tree gate
//! the exclusive claim holds keeps every other land out of that moment.
//! An explicit session holds the exclusive claim, waited for up to
//! [`COMMAND_BOUND`], until [`close`], so the command's own writes and the
//! push that follows are one.
//!
//! **[`open_all`]** fetches [`PARALLEL_FETCHES`] brains at a time at most
//! (each with its own read time, which waiting for the others' lands does
//! not use), then lands them one by one in the brains' order.
//!
//! **Brains left alone**, with no report: one that is not in git, one
//! nested in a larger repository, a shallow one, one whose branch has no
//! upstream (an explicit session reports `no_upstream` there), and, for a
//! read, one whose git is too old to sync.
//!
//! **No off switch.** Syncing is what every command that opens a git brain
//! with an upstream does: no setting of the brain's and no environment
//! variable is read here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::background;
use super::claim::{COMMAND_BOUND, ClaimError, READ_BOUND, TreeClaim};
use super::engine::{self, Context};
use super::fetch::{BACKOFF, STAMP, THROTTLE, back_off, backed_off, fetched_recently, write_stamp};
use super::git::SyncGit;
use super::land::failed;
use super::{LeftAsIs, LeftReason, NETWORK_TIME_LIMIT, SyncError, SyncReport, SyncRequest};
use crate::dream::DreamOptions;
use crate::git::SystemGit;

/// How long the gits a read runs before it reads may take, all together.
pub const READ_TIME_LIMIT: Duration = Duration::from_secs(3);

/// How many brains [`open_all`] fetches at once at most.
pub const PARALLEL_FETCHES: usize = 8;

/// What the command that opens a brain does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// It reads the brain.
    Read,
    /// It reads the brain and tidies it.
    Housekeeping,
    /// The person asked for it to change the brain or sync it.
    Explicit,
}

impl Intent {
    /// How long its fetch (and its push) may take.
    pub fn network_bound(self) -> Duration {
        match self {
            Intent::Read | Intent::Housekeeping => READ_TIME_LIMIT,
            Intent::Explicit => NETWORK_TIME_LIMIT,
        }
    }

    /// How long it waits for the tree claim another command holds.
    pub fn tree_bound(self) -> Duration {
        match self {
            Intent::Read | Intent::Housekeeping => READ_BOUND,
            Intent::Explicit => COMMAND_BOUND,
        }
    }

    /// Whether it fetches at most once per [`THROTTLE`], outside the
    /// back-off, with every git before the land bounded.
    pub fn throttled(self) -> bool {
        !matches!(self, Intent::Explicit)
    }
}

/// A brain to open: its root, and what the maps a merge rebuilds say.
#[derive(Debug, Clone, Copy)]
pub struct Brain<'a> {
    pub root: &'a Path,
    pub maps: DreamOptions<'a>,
}

/// What every session of a command shares.
#[derive(Debug, Clone)]
pub struct SessionEnv {
    /// The git every step starts from.
    pub git: SystemGit,
    /// Skip the `pre-merge-commit` and `commit-msg` hooks of merges.
    pub no_verify: bool,
    /// The time the throttle and the back-off compare with.
    pub now: SystemTime,
    /// The program a read's [`close`] starts to push the commits it leaves
    /// waiting (the CLI passes its own executable); `None`: none starts.
    pub program: Option<PathBuf>,
}

/// One brain opened by a command, until [`close`].
#[derive(Debug)]
pub struct Session<'a> {
    intent: Intent,
    /// The engine's brain; `None` for a brain left alone, or one that
    /// could not be opened.
    ctx: Option<Context<'a, SystemGit>>,
    /// `None` for a brain left alone.
    report: Option<SyncReport>,
    /// Shared for a read, exclusive for an explicit command.
    claim: Option<TreeClaim>,
    /// [`SessionEnv::program`].
    program: Option<PathBuf>,
    /// [`SessionEnv::now`].
    now: SystemTime,
}

impl Session<'_> {
    /// What the session has to say so far: for an explicit session of a
    /// brain it syncs, always; for a read, when something is to be said.
    pub fn report(&self) -> Option<&SyncReport> {
        let report = self.report.as_ref()?;
        (self.intent == Intent::Explicit || !silent(report)).then_some(report)
    }
}

/// A report with nothing in it.
fn silent(report: &SyncReport) -> bool {
    *report == SyncReport::default()
}

/// Open the brain for `intent`: fetch, then land (see the module docs).
pub fn open<'a>(brain: Brain<'a>, intent: Intent, env: &SessionEnv) -> Session<'a> {
    settle(fetch(brain, intent, env), env)
}

/// Open every brain for `intent`: [`PARALLEL_FETCHES`] fetch at a time at
/// most, then each lands in turn; the sessions in the brains' order.
pub fn open_all<'a>(brains: &[Brain<'a>], intent: Intent, env: &SessionEnv) -> Vec<Session<'a>> {
    let next = AtomicUsize::new(0);
    // Each worker takes the next brain until none is left, and gives back
    // what it fetched with each brain's place. A worker's panic is a bug,
    // and the command's.
    let mut fetched: Vec<(usize, Fetched<'a>)> = thread::scope(|scope| {
        let workers: Vec<_> = (0..PARALLEL_FETCHES.min(brains.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(brain) = brains.get(index) else {
                            return done;
                        };
                        done.push((index, fetch(*brain, intent, env)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    fetched.sort_by_key(|(index, _)| *index);
    fetched
        .into_iter()
        .map(|(_, fetched)| settle(fetched, env))
        .collect()
}

/// End the session: release its claim and say what was done ([`close_all`]).
pub fn close(session: Session) -> Option<SyncReport> {
    close_all(vec![session]).pop().flatten()
}

/// End the sessions, each as [`close`] says, the reports in their order.
/// An explicit session pushes what is committed first ([`engine::share`]).
/// The commits the reads leave waiting go to one background push for all
/// their brains ([`push_later`]).
pub fn close_all(sessions: Vec<Session>) -> Vec<Option<SyncReport>> {
    let mut waiting = Vec::new();
    let mut program = None;
    let reports = sessions
        .into_iter()
        .map(|session| {
            let (report, handed) = finish(session);
            if let Some((root, by)) = handed {
                waiting.push(root);
                program = Some(by);
            }
            report
        })
        .collect();
    if let Some(program) = program {
        push_later(&program, &waiting);
    }
    reports
}

/// Start one background push of the commits waiting in the brains at
/// `roots`. A start that fails or is refused leaves them counted in
/// `to_push`, and the next read tries again.
fn push_later(program: &Path, roots: &[PathBuf]) {
    let _ = background::start(program, roots);
}

/// One session closed: its report, and the brain to hand to the
/// background push with the program that pushes it.
fn finish(session: Session) -> (Option<SyncReport>, Option<(PathBuf, PathBuf)>) {
    let Session {
        intent,
        ctx,
        report,
        claim,
        program,
        now,
    } = session;
    let Some(ctx) = ctx else {
        let report = report.filter(|r| intent == Intent::Explicit || !silent(r));
        return (report, None);
    };
    let mut report = report.unwrap_or_default();
    // What the command committed since the session opened counts.
    if let Some(count) = engine::to_push(&ctx) {
        report.to_push = count;
    }
    if intent == Intent::Explicit {
        report = engine::share(&ctx, claim.as_ref(), report);
        drop(claim);
        last_push(&ctx, &mut report, intent, now);
        return (Some(report), None);
    }
    drop(claim);
    last_push(&ctx, &mut report, intent, now);
    let common = &ctx.dirs().common;
    let handed = program
        .filter(|_| report.to_push > 0 && background::wanted(common, now))
        .map(|program| (ctx.repo().dir.clone(), program));
    ((!silent(&report)).then_some(report), handed)
}

/// The last background push, as `report` says it: a failure until a push
/// succeeds, a success once. A record a live background run may be
/// replacing stays; one an explicit push that succeeded makes moot goes
/// unreported.
fn last_push(ctx: &Context<SystemGit>, report: &mut SyncReport, intent: Intent, now: SystemTime) {
    let common = &ctx.dirs().common;
    let path = common.join(background::LAST_PUSH);
    let Some(record) = background::LastPush::read(&path) else {
        return;
    };
    let running =
        background::Lock::read(&common.join(background::LOCK)).is_some_and(|lock| lock.live(now));
    let done = record.result == background::PushResult::Pushed;
    let moot = !done && intent == Intent::Explicit && report.pushed;
    if (done || moot) && !running {
        let _ = std::fs::remove_file(&path);
    }
    if !moot {
        report.last_push = Some(record);
    }
}

/// A brain's fetch: what [`open`] does before the land.
#[derive(Debug)]
struct Fetched<'a> {
    intent: Intent,
    ctx: Option<Context<'a, SystemGit>>,
    report: Option<SyncReport>,
    /// The upstream's commit the fetch named, to land (`Some(None)`: the
    /// remote has no such branch); `None`: nothing to land.
    theirs: Option<Option<String>>,
    /// The read time the fetch used.
    spent: Duration,
}

impl<'a> Fetched<'a> {
    fn alone(intent: Intent) -> Fetched<'a> {
        Fetched {
            intent,
            ctx: None,
            report: None,
            theirs: None,
            spent: Duration::ZERO,
        }
    }

    /// Left as it is for `left`.
    fn left(mut self, left: LeftAsIs) -> Fetched<'a> {
        let left = in_time(left);
        self.report
            .get_or_insert_with(SyncReport::default)
            .left_as_is = Some(left);
        self
    }
}

/// Open the brain at `brain.root` and fetch its upstream when it is time
/// to (see the module docs).
fn fetch<'a>(brain: Brain<'a>, intent: Intent, env: &SessionEnv) -> Fetched<'a> {
    let started = Instant::now();
    let deadline = intent.throttled().then(|| started + READ_TIME_LIMIT);
    let git = env.git.clone().with_deadline(deadline);
    let request = SyncRequest {
        no_verify: env.no_verify,
        maps: brain.maps,
        network_time_limit: intent.network_bound(),
        tree_bound: intent.tree_bound(),
    };
    let alone = Fetched::alone(intent);
    let mut ctx = match engine::open(brain.root, &git, &request) {
        Ok(ctx) => ctx,
        Err(SyncError::NotARepo | SyncError::Nested { .. }) => return alone,
        Err(SyncError::GitTooOld { .. }) if intent.throttled() => return alone,
        Err(e) => return alone.left(e.into()),
    };
    if !intent.throttled() {
        ctx.wait_for_remote(COMMAND_BOUND);
    }
    match ctx.git().is_shallow(ctx.repo()) {
        Ok(false) => {}
        Ok(true) => return alone,
        Err(e) => return alone.left(e.into()),
    }
    if ctx.upstream().is_none() {
        return match engine::ready(&ctx) {
            Err(left) if !intent.throttled() => alone.left(left),
            _ => alone,
        };
    }
    let mut fetched = Fetched {
        report: Some(SyncReport::default()),
        ..alone
    };
    let common = &ctx.dirs().common;
    let (stamp, backoff) = (common.join(STAMP), common.join(BACKOFF));
    let quiet = || {
        intent.throttled()
            && (fetched_recently(&stamp, env.now, THROTTLE) || backed_off(&backoff, env.now))
    };
    if !quiet() {
        match ctx.take_remote() {
            Err(left) => fetched = fetched.left(left),
            // Another read that just fetched holds it, or another command.
            Ok(None) if quiet() => {}
            Ok(None) => fetched = fetched.left(engine::remote_busy()),
            // Checked again under the claim: one fetch per throttle.
            Ok(Some(_remote)) if quiet() => {}
            Ok(Some(remote)) => match engine::ready(&ctx) {
                Err(left) => fetched = fetched.left(left),
                Ok(upstream) => {
                    write_stamp(&stamp, env.now);
                    let bound = Instant::now() + intent.network_bound();
                    let bound = deadline.map_or(bound, |d| d.min(bound));
                    match engine::fetch_upstream(&ctx, upstream, &remote) {
                        Ok(theirs) => fetched.theirs = Some(theirs),
                        Err(left) => {
                            if left.reason == LeftReason::Offline && Instant::now() >= bound {
                                back_off(&backoff, env.now);
                            }
                            fetched = fetched.left(left);
                        }
                    }
                }
            },
        }
    }
    fetched.spent = started.elapsed();
    fetched.ctx = Some(ctx);
    fetched
}

/// Land what the fetch named under the tree claim, and hold the claim the
/// session keeps until [`close`] (see the module docs).
fn settle<'a>(fetched: Fetched<'a>, env: &SessionEnv) -> Session<'a> {
    let Fetched {
        intent,
        ctx,
        mut report,
        theirs,
        spent,
    } = fetched;
    let Some(mut ctx) = ctx else {
        return Session {
            intent,
            ctx: None,
            report,
            claim: None,
            program: env.program.clone(),
            now: env.now,
        };
    };
    let own = ctx.dirs().own.clone();
    let claim = match theirs {
        // Nothing to land: read under the shared claim.
        None if intent.throttled() => match TreeClaim::shared(&own, intent.tree_bound()) {
            Ok(claim) => Some(claim),
            Err(e) => {
                add_left(&mut report, claim_left(e));
                None
            }
        },
        theirs => match TreeClaim::exclusive(&own, intent.tree_bound()) {
            Ok(claim) => {
                if let Some(theirs) = theirs {
                    if intent.throttled() {
                        ctx.until(Some(Instant::now() + READ_TIME_LIMIT.saturating_sub(spent)));
                    }
                    let mut pulled = engine::pull_fetched(&ctx, &claim, theirs);
                    pulled.left_as_is = pulled.left_as_is.map(in_time);
                    report = Some(pulled);
                }
                if intent.throttled() {
                    match claim.downgrade(READ_BOUND) {
                        Ok(shared) => Some(shared),
                        Err(e) => {
                            add_left(&mut report, claim_left(e));
                            None
                        }
                    }
                } else {
                    Some(claim)
                }
            }
            Err(e) => {
                add_left(&mut report, claim_left(e));
                // Read as it is: shared with other readers when they are
                // all that hold it.
                intent
                    .throttled()
                    .then(|| TreeClaim::shared(&own, Duration::ZERO).ok())
                    .flatten()
            }
        },
    };
    // The read has started: what the command does with the brain next is
    // never stopped.
    ctx.until(None);
    Session {
        intent,
        ctx: Some(ctx),
        report,
        claim,
        program: env.program.clone(),
        now: env.now,
    }
}

/// `report` left as it is for `why`, unless it already was for another
/// reason.
fn add_left(report: &mut Option<SyncReport>, why: LeftAsIs) {
    let report = report.get_or_insert_with(SyncReport::default);
    if report.left_as_is.is_none() {
        report.left_as_is = Some(why);
    }
}

/// Why the tree claim was not had.
fn claim_left(e: ClaimError) -> LeftAsIs {
    match e {
        ClaimError::Busy => LeftAsIs::because(
            LeftReason::Busy,
            "another kluris command is changing this brain's files, so nothing was pulled and it was read as it is; run again once it is done",
        ),
        ClaimError::Io(e) => failed(format!("cannot claim the work tree: {e}")),
    }
}

/// `left` as the session reports it: a step whose git was stopped at the
/// read's time limit leaves the brain as it is (`busy`). Only a read's
/// gits before the land have a deadline, so nothing else is ever stopped.
fn in_time(left: LeftAsIs) -> LeftAsIs {
    if !left.stopped {
        return left;
    }
    LeftAsIs::because(
        LeftReason::Busy,
        format!(
            "git did not finish within the read's {} s, so nothing was pulled and the brain was read as it is: {}",
            READ_TIME_LIMIT.as_secs(),
            left.detail.unwrap_or_default()
        ),
    )
}
