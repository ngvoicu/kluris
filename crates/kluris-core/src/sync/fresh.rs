//! Fresh reads: before `wake-up` and `search` read a brain, the objects of
//! its current branch's upstream are fetched ([`SyncGit::fetch_branch`]), so
//! what teammates pushed is already here when `kluris sync` fetches it. The
//! read then reads the brain as it is on disk: a fresh read changes no ref,
//! the work tree, the index or HEAD.
//!
//! - One read at a time decides and fetches: a read holds an exclusive OS
//!   lock (`flock`, `LockFileEx`) on [`CLAIM`] from before its checks until
//!   it is done, and one that finds it held reads the brain as it is
//!   ([`Fresh::Throttled`]); one that cannot take it for another reason
//!   (a read-only folder) does not fetch ([`Fresh::NoClaim`]). The lock
//!   goes with the process, so a read that crashes or is killed blocks no
//!   other; the file is made when missing and never removed. It is
//!   Kluris's own, not a git lock. `kluris sync` neither takes nor waits
//!   for it.
//! - At most once per [`super::fetch::THROTTLE`] per brain, stamped before
//!   the fetch ([`super::fetch::write_stamp`]), so an offline brain costs
//!   the time limit once a minute, not on every read; a fetch stopped at
//!   the deadline backs off ([`super::fetch::back_off`]).
//! - Within [`TIME_LIMIT`] overall: a git still running then is stopped with
//!   every process it started ([`crate::process`]), and the read goes on.
//! - Only a brain at the root of its own work tree: a brain nested in a
//!   larger repo (a project's) is never fetched and gets no stamp; nor is a
//!   shallow brain (deepening it would write `.git/shallow`).
//! - Nothing fails the read: no work tree, no upstream (or a detached
//!   HEAD), or a fetch that fails (offline, refused credentials) ends the
//!   step quietly, and nothing is printed.
//! - What it writes is objects and the stamp: no ref, `FETCH_HEAD`,
//!   `packed-refs`, `.git/shallow`, commit-graph or config; every git it
//!   runs has `GIT_OPTIONAL_LOCKS=0` ([`READ_ENV`]), so none refreshes the
//!   index, and the remote environment, so none prompts.
//! - No hook runs: every git after `rev-parse` (which finds the git
//!   directory and runs none) has `core.hooksPath` at [`NO_HOOKS`] under
//!   the git directory, a path that is never made; when something exists
//!   there, the read does not fetch.
//! - Off with `KLURIS_NO_AUTO_SYNC=1` or `sync: {auto_fetch: false}` in the
//!   brain's `kluris.yml`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::fetch::{
    BACKOFF, STAMP, THROTTLE, back_off, backed_off, end_backoff, fetched_recently, write_stamp,
};
use super::git::{RemoteError, SyncGit};
use crate::config::read_brain_config;
use crate::git::{Repo, SystemGit};

/// The time the whole step may take.
pub const TIME_LIMIT: Duration = Duration::from_secs(3);

/// The hooks path of every git a fresh read runs after `rev-parse`,
/// relative to the git directory: it is never made, so no hook runs.
pub const NO_HOOKS: &str = "kluris-sync/no-hooks";

/// The file whose OS lock a read holds while it decides and fetches,
/// relative to the git directory.
pub const CLAIM: &str = "kluris-sync/fetch-claim";

/// The environment of every git a fresh read runs: no optional locks, so
/// nothing it runs writes the index.
pub const READ_ENV: [(&str, &str); 1] = [("GIT_OPTIONAL_LOCKS", "0")];

/// What the step did. Never shown: the read goes on the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fresh {
    /// `KLURIS_NO_AUTO_SYNC=1` or `sync.auto_fetch: false`.
    Disabled,
    /// The brain is not in a git work tree.
    NotARepo,
    /// The brain is below the root of a larger work tree.
    Nested,
    /// The brain is a shallow clone.
    Shallow,
    /// Something exists at [`NO_HOOKS`], so hooks could run.
    HooksPathTaken,
    /// The claim ([`CLAIM`]) could not be taken (not another read's).
    NoClaim,
    /// Fetched less than the throttle ago, or another read is fetching.
    Throttled,
    /// A fetch was stopped at the deadline less than its back-off ago.
    BackedOff,
    /// A detached HEAD, or a branch without an upstream on a remote.
    NoUpstream,
    /// The fetch failed (offline, no such remote or branch, refused
    /// credentials).
    FetchFailed,
    /// The time limit passed; whatever git was running was stopped.
    TimedOut,
    /// The upstream's objects were fetched.
    Fetched,
    /// Another git step failed.
    Failed,
}

/// One fresh read of the brain at `brain`.
#[derive(Debug, Clone)]
pub struct FreshRequest<'a> {
    pub brain: &'a Path,
    /// `KLURIS_NO_AUTO_SYNC` is `1`.
    pub disabled_by_env: bool,
    /// The time the throttle compares with and the stamp records.
    pub now: SystemTime,
    pub throttle: Duration,
    pub time_limit: Duration,
}

impl FreshRequest<'_> {
    /// A request now, with the standard throttle and time limit.
    pub fn new(brain: &Path, disabled_by_env: bool) -> FreshRequest<'_> {
        FreshRequest {
            brain,
            disabled_by_env,
            now: SystemTime::now(),
            throttle: THROTTLE,
            time_limit: TIME_LIMIT,
        }
    }
}

/// The fresh read with the system's git.
pub fn refresh(brain: &Path, disabled_by_env: bool) -> Fresh {
    fresh_read(
        &SystemGit::new(),
        &FreshRequest::new(brain, disabled_by_env),
    )
}

/// Fetch the brain's upstream if it is time to (see the module
/// documentation), with `git` as the base of every git the read runs.
pub fn fresh_read(git: &SystemGit, req: &FreshRequest) -> Fresh {
    if req.disabled_by_env || turned_off_in_config(req.brain) {
        return Fresh::Disabled;
    }
    let deadline = Instant::now() + req.time_limit;
    let mut git = git.clone();
    for (key, value) in READ_ENV {
        git = git.env(key, value);
    }
    match step(&git, req, deadline) {
        Ok(fresh) | Err(fresh) => fresh,
    }
}

fn turned_off_in_config(brain: &Path) -> bool {
    read_brain_config(brain).is_ok_and(|cfg| cfg.sync.auto_fetch == Some(false))
}

/// Where the brain sits in its work tree.
struct WorkTree {
    /// `git rev-parse --show-prefix`: empty at the root.
    prefix: String,
    shallow: bool,
    /// The paths asked for, inside the git directory.
    paths: Vec<PathBuf>,
}

/// Where `dir` sits in its work tree, whether the repository is shallow,
/// and each of `names` inside its git directory (`git rev-parse
/// --is-shallow-repository --show-prefix --git-path`); `None` outside a work
/// tree.
fn work_tree(
    git: &SystemGit,
    dir: &Path,
    names: &[&str],
    deadline: Instant,
) -> Result<Option<WorkTree>, Fresh> {
    let mut args = vec![
        "rev-parse",
        "--is-inside-work-tree",
        "--is-shallow-repository",
        "--show-prefix",
    ];
    for name in names {
        args.extend(["--git-path", name]);
    }
    let done = git.bounded(dir, &args, deadline).map_err(ended)?;
    if !done.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8(done.stdout).map_err(|_| Fresh::Failed)?;
    let mut lines = text.lines();
    if lines.next() != Some("true") {
        return Ok(None);
    }
    let shallow = match lines.next() {
        Some("true") => true,
        Some("false") => false,
        _ => return Err(Fresh::Failed),
    };
    let prefix = lines.next().unwrap_or_default().to_string();
    let paths: Vec<PathBuf> = lines.map(|line| dir.join(line)).collect();
    if paths.len() != names.len() {
        return Err(Fresh::Failed);
    }
    Ok(Some(WorkTree {
        prefix,
        shallow,
        paths,
    }))
}

/// A git step stopped at the deadline, or failed, as the read's result.
fn ended(error: RemoteError) -> Fresh {
    match error {
        RemoteError::TimedOut => Fresh::TimedOut,
        RemoteError::Failed(_) => Fresh::Failed,
    }
}

fn step(git: &SystemGit, req: &FreshRequest, deadline: Instant) -> Result<Fresh, Fresh> {
    let dir = req.brain;
    let Some(tree) = work_tree(git, dir, &[STAMP, BACKOFF, NO_HOOKS, CLAIM], deadline)? else {
        return Ok(Fresh::NotARepo);
    };
    if !tree.prefix.is_empty() {
        return Ok(Fresh::Nested);
    }
    if tree.shallow {
        return Ok(Fresh::Shallow);
    }
    let [stamp, backoff, no_hooks, claim] = tree.paths.as_slice() else {
        return Ok(Fresh::Failed);
    };
    // git resolves a relative hooks path from the work tree root.
    let no_hooks = std::path::absolute(no_hooks).unwrap_or_else(|_| no_hooks.clone());
    if fs::symlink_metadata(&no_hooks).is_ok() {
        return Ok(Fresh::HooksPathTaken);
    }
    // Held until the read is done, so the checks, the stamp, the fetch and
    // the back-off are one read's at a time.
    let _claim = match take_claim(claim) {
        Ok(file) => file,
        Err(fs::TryLockError::WouldBlock) => return Ok(Fresh::Throttled),
        Err(fs::TryLockError::Error(_)) => return Ok(Fresh::NoClaim),
    };
    if fetched_recently(stamp, req.now, req.throttle) {
        return Ok(Fresh::Throttled);
    }
    if backed_off(backoff, req.now) {
        return Ok(Fresh::BackedOff);
    }
    let repo = Repo {
        dir: dir.to_path_buf(),
        prefix: tree.prefix,
    };
    let git = git
        .clone()
        .config(format!("core.hooksPath={}", no_hooks.display()));
    let branch = git.branch(&repo).map_err(|_| Fresh::Failed)?;
    let upstream = match branch {
        Some(branch) => git.upstream(&repo, &branch).map_err(|_| Fresh::Failed)?,
        None => None,
    };
    let Some(upstream) = upstream else {
        write_stamp(stamp, req.now);
        return Ok(Fresh::NoUpstream);
    };
    write_stamp(stamp, req.now);
    let git = git.for_remote(&repo).map_err(|_| Fresh::Failed)?;
    match git.fetch_branch(&repo, &upstream, deadline) {
        Ok(()) => {
            end_backoff(backoff);
            Ok(Fresh::Fetched)
        }
        Err(RemoteError::TimedOut) => {
            back_off(backoff, req.now);
            Err(Fresh::TimedOut)
        }
        Err(RemoteError::Failed(_)) => Err(Fresh::FetchFailed),
    }
}

/// The claim: [`CLAIM`] (made when missing, never removed) opened and
/// locked exclusively without waiting (`flock`, `LockFileEx`). The lock
/// goes with the file handle, and so with a read that crashes or is killed.
fn take_claim(path: &Path) -> Result<fs::File, fs::TryLockError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fs::TryLockError::Error)?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(fs::TryLockError::Error)?;
    file.try_lock()?;
    Ok(file)
}
