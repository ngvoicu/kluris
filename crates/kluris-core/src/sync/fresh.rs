//! Fresh reads (SPEC Phase 7, TEST-K3-49): before `wake-up` and `search`
//! read a brain, a `git fetch` of its current branch's upstream, so the
//! remote-tracking refs hold what teammates pushed. The read then reads the
//! brain as it is on disk: a fresh read never changes the work tree, the
//! index or HEAD. Teammates' changes come into the work tree with `kluris
//! sync` ([`super::sync`]) and `kluris sync --watch`.
//!
//! - At most once per [`THROTTLE`] per brain: the time of the last attempt,
//!   in Unix seconds, is in `<git dir>/kluris-sync/last-fetch` ([`STAMP`]),
//!   written atomically before the fetch, so an offline brain costs the
//!   time limit once a minute, not on every read.
//! - Within [`TIME_LIMIT`] overall: a git still running then is stopped with
//!   every process it started ([`crate::process`]), and the read goes on.
//! - Only a brain at the root of its own work tree: a brain nested in a
//!   larger repo (a project's) is never fetched and gets no stamp.
//! - Nothing fails the read: no work tree, no upstream (or a detached
//!   HEAD), or a fetch that fails (offline, refused credentials) ends the
//!   step quietly, and nothing is printed.
//! - What it writes is what `git fetch` writes (objects, the remote-tracking
//!   refs, `FETCH_HEAD`) and the stamp: every git it runs has
//!   `GIT_OPTIONAL_LOCKS=0` ([`READ_ENV`]), so none refreshes the index.
//! - Never a prompt: [`prompt_env`] for the fetch.
//! - Off with `KLURIS_NO_AUTO_SYNC=1` or `sync: {auto_fetch: false}` in the
//!   brain's `kluris.yml`.
//!
//! Git runs behind [`FreshGit`] so the decisions can be tested with a fake.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::atomic;
use crate::config::read_brain_config;
use crate::git::{GitError, SystemGit};
use crate::process::{Finished, RunError, run_until};

/// The stamp of the last fetch, relative to the git directory.
pub const STAMP: &str = "kluris-sync/last-fetch";

/// The least time between two fetches of a brain.
pub const THROTTLE: Duration = Duration::from_secs(60);

/// The time the whole step may take.
pub const TIME_LIMIT: Duration = Duration::from_secs(3);

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
    /// Fetched less than [`THROTTLE`] ago.
    Throttled,
    /// A detached HEAD, or a branch without an upstream on a remote.
    NoUpstream,
    /// The fetch failed (offline, no such remote, refused credentials).
    FetchFailed,
    /// The time limit passed; whatever git was running was stopped.
    TimedOut,
    /// The upstream was fetched.
    Fetched,
    /// Another git step failed.
    Failed,
}

/// The upstream of the current branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// The remote to fetch (`%(upstream:remotename)`).
    pub remote: String,
    /// The upstream's full ref (`refs/remotes/origin/main`).
    pub refname: String,
}

/// Where the brain sits in its work tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkTree {
    /// The brain's folder relative to the work tree root, `/`-separated
    /// with a trailing `/` (`git rev-parse --show-prefix`); empty at the root.
    pub prefix: String,
    /// The paths asked for, inside the git directory.
    pub paths: Vec<PathBuf>,
}

/// A git step that did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshError {
    /// The deadline passed and git was stopped.
    TimedOut,
    /// git could not run or failed.
    Failed(GitError),
}

/// The git steps of a fresh read (and of `kluris sync`'s fetch), each
/// bounded by `deadline`.
pub trait FreshGit {
    /// Where `dir` sits in its work tree, and each of `names` inside its git
    /// directory (`git rev-parse --show-prefix --git-path`); `None` outside a
    /// work tree.
    fn git_paths(
        &self,
        dir: &Path,
        names: &[&str],
        deadline: Instant,
    ) -> Result<Option<WorkTree>, FreshError>;

    /// The current branch's upstream; `None` on a detached HEAD, without an
    /// upstream, or with a local branch as upstream.
    fn upstream(&self, dir: &Path, deadline: Instant) -> Result<Option<Upstream>, FreshError>;

    /// `git fetch <remote>`, never prompting.
    fn fetch(&self, dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError>;

    /// Whether `ancestor` is an ancestor of (or is) `descendant`.
    fn is_ancestor(
        &self,
        dir: &Path,
        ancestor: &str,
        descendant: &str,
        deadline: Instant,
    ) -> Result<bool, FreshError>;
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
/// documentation).
pub fn fresh_read(git: &impl FreshGit, req: &FreshRequest) -> Fresh {
    if req.disabled_by_env || turned_off_in_config(req.brain) {
        return Fresh::Disabled;
    }
    let deadline = Instant::now() + req.time_limit;
    match step(git, req, deadline) {
        Ok(fresh) | Err(fresh) => fresh,
    }
}

fn turned_off_in_config(brain: &Path) -> bool {
    read_brain_config(brain).is_ok_and(|cfg| cfg.sync.auto_fetch == Some(false))
}

/// A failed git step as the result it ends the read with.
fn ended(error: FreshError) -> Fresh {
    match error {
        FreshError::TimedOut => Fresh::TimedOut,
        FreshError::Failed(_) => Fresh::Failed,
    }
}

fn step(git: &impl FreshGit, req: &FreshRequest, deadline: Instant) -> Result<Fresh, Fresh> {
    let dir = req.brain;
    let Some(tree) = git.git_paths(dir, &[STAMP], deadline).map_err(ended)? else {
        return Ok(Fresh::NotARepo);
    };
    if !tree.prefix.is_empty() {
        return Ok(Fresh::Nested);
    }
    let [stamp] = tree.paths.as_slice() else {
        return Ok(Fresh::Failed);
    };
    let now = unix_seconds(req.now);
    if fetched_recently(stamp, now, req.throttle) {
        return Ok(Fresh::Throttled);
    }
    let Some(upstream) = git.upstream(dir, deadline).map_err(ended)? else {
        write_stamp(stamp, now);
        return Ok(Fresh::NoUpstream);
    };
    write_stamp(stamp, now);
    git.fetch(dir, &upstream.remote, deadline)
        .map_err(|e| match e {
            FreshError::TimedOut => Fresh::TimedOut,
            FreshError::Failed(_) => Fresh::FetchFailed,
        })?;
    Ok(Fresh::Fetched)
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether the stamp records a time in the last `throttle`; a stamp that
/// does not read, or from the future (the clock went back), does not.
fn fetched_recently(stamp: &Path, now: u64, throttle: Duration) -> bool {
    let Ok(text) = fs::read_to_string(stamp) else {
        return false;
    };
    let Ok(at) = text.trim().parse::<u64>() else {
        return false;
    };
    at <= now && now - at < throttle.as_secs()
}

/// Record an attempt; a stamp that cannot be written only costs a fetch.
fn write_stamp(stamp: &Path, now: u64) {
    if let Some(parent) = stamp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = atomic::write_atomic(stamp, format!("{now}\n").as_bytes());
}

/// The environment that keeps a fetch from prompting, given the caller's
/// environment (`get`) and whether git config sets `core.sshCommand`:
/// `GIT_TERMINAL_PROMPT=0` and an empty `GIT_ASKPASS` (no askpass helper,
/// `core.askPass` and `SSH_ASKPASS` included) always; `GCM_INTERACTIVE=never`
/// unless the user set it; `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` unless
/// the user chose an ssh command (`GIT_SSH_COMMAND`, `GIT_SSH` or
/// `core.sshCommand`, which the variable would override).
pub fn prompt_env(
    get: impl Fn(&str) -> Option<OsString>,
    ssh_command_configured: bool,
) -> Vec<(&'static str, &'static str)> {
    let set = |key: &str| get(key).is_some_and(|v| !v.is_empty());
    let mut env = vec![("GIT_TERMINAL_PROMPT", "0"), ("GIT_ASKPASS", "")];
    if !set("GCM_INTERACTIVE") {
        env.push(("GCM_INTERACTIVE", "never"));
    }
    if !(ssh_command_configured || set("GIT_SSH_COMMAND") || set("GIT_SSH")) {
        env.push(("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"));
    }
    env
}

/// Settings for the git runs that could start a background process (which
/// would be killed with the step): no automatic maintenance, no fsmonitor
/// daemon.
const NO_BACKGROUND: [&str; 6] = [
    "-c",
    "maintenance.auto=false",
    "-c",
    "gc.auto=0",
    "-c",
    "core.fsmonitor=false",
];

impl SystemGit {
    pub(crate) fn bounded(
        &self,
        dir: &Path,
        args: &[&str],
        env: &[(&str, &str)],
        deadline: Instant,
    ) -> Result<Finished, FreshError> {
        let mut cmd: Command = self.command(Some(dir), args);
        for (key, value) in env {
            cmd.env(key, value);
        }
        let error = |code, detail: String| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code,
            detail,
        };
        match run_until(cmd, deadline) {
            Ok(done) => Ok(done),
            Err(RunError::TimedOut) => Err(FreshError::TimedOut),
            Err(RunError::Spawn(e)) => Err(FreshError::Failed(error(None, e.to_string()))),
        }
    }

    /// [`SystemGit::bounded`], failing unless git exits 0.
    pub(crate) fn checked(
        &self,
        dir: &Path,
        args: &[&str],
        env: &[(&str, &str)],
        deadline: Instant,
    ) -> Result<Vec<u8>, FreshError> {
        let done = self.bounded(dir, args, env, deadline)?;
        if done.status.success() {
            Ok(done.stdout)
        } else {
            Err(FreshError::Failed(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: done.status.code(),
                detail: String::from_utf8_lossy(&done.stderr).trim().to_string(),
            }))
        }
    }

    pub(crate) fn with_no_background<'a>(args: &[&'a str]) -> Vec<&'a str> {
        NO_BACKGROUND
            .iter()
            .copied()
            .chain(args.iter().copied())
            .collect()
    }
}

/// git exited 0 with output this step cannot use.
fn unreadable(args: &[&str], why: &str) -> FreshError {
    FreshError::Failed(GitError {
        args: args.iter().map(|a| a.to_string()).collect(),
        code: Some(0),
        detail: why.into(),
    })
}

const NOT_UTF8: &str = "the output is not UTF-8";

impl FreshGit for SystemGit {
    fn git_paths(
        &self,
        dir: &Path,
        names: &[&str],
        deadline: Instant,
    ) -> Result<Option<WorkTree>, FreshError> {
        let mut args = vec!["rev-parse", "--is-inside-work-tree", "--show-prefix"];
        for name in names {
            args.extend(["--git-path", name]);
        }
        let done = self.bounded(dir, &args, &READ_ENV, deadline)?;
        if !done.status.success() {
            return Ok(None);
        }
        let text = String::from_utf8(done.stdout).map_err(|_| unreadable(&args, NOT_UTF8))?;
        let mut lines = text.lines();
        if lines.next() != Some("true") {
            return Ok(None);
        }
        let prefix = lines.next().unwrap_or_default().to_string();
        let paths: Vec<PathBuf> = lines.map(|line| dir.join(line)).collect();
        if paths.len() != names.len() {
            return Err(unreadable(&args, "not one path per name"));
        }
        Ok(Some(WorkTree { prefix, paths }))
    }

    fn upstream(&self, dir: &Path, deadline: Instant) -> Result<Option<Upstream>, FreshError> {
        let symbolic = ["symbolic-ref", "--quiet", "HEAD"];
        let head = self.bounded(dir, &symbolic, &READ_ENV, deadline)?;
        if !head.status.success() {
            return Ok(None);
        }
        let branch = String::from_utf8(head.stdout).map_err(|_| unreadable(&symbolic, NOT_UTF8))?;
        let args = [
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream)",
            branch.trim(),
        ];
        let out = self.checked(dir, &args, &READ_ENV, deadline)?;
        let text = String::from_utf8(out).map_err(|_| unreadable(&args, NOT_UTF8))?;
        let line = text.lines().next().unwrap_or_default();
        let Some((remote, refname)) = line.split_once('\0') else {
            return Ok(None);
        };
        if remote.is_empty() || remote == "." || refname.is_empty() {
            return Ok(None);
        }
        Ok(Some(Upstream {
            remote: remote.to_string(),
            refname: refname.to_string(),
        }))
    }

    fn fetch(&self, dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError> {
        let ssh_command = self.bounded(
            dir,
            &["config", "--get", "core.sshCommand"],
            &READ_ENV,
            deadline,
        )?;
        let configured =
            ssh_command.status.success() && !ssh_command.stdout.trim_ascii().is_empty();
        let mut env = prompt_env(|key| self.env_var(key), configured);
        env.extend(READ_ENV);
        let args =
            SystemGit::with_no_background(&["fetch", "--quiet", "--no-recurse-submodules", remote]);
        self.checked(dir, &args, &env, deadline).map(drop)
    }

    fn is_ancestor(
        &self,
        dir: &Path,
        ancestor: &str,
        descendant: &str,
        deadline: Instant,
    ) -> Result<bool, FreshError> {
        let args = ["merge-base", "--is-ancestor", ancestor, descendant];
        let done = self.bounded(dir, &args, &[], deadline)?;
        match done.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => Err(FreshError::Failed(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code,
                detail: String::from_utf8_lossy(&done.stderr).trim().to_string(),
            })),
        }
    }
}
