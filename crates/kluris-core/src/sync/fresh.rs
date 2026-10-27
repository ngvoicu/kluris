//! Fresh reads (SPEC Phase 7, TEST-K3-49): before `wake-up` and `search`
//! read a brain, a `git fetch` of the objects of its current branch's
//! upstream, so what teammates pushed is already here when `kluris sync`
//! fetches it. The read then reads the brain as it is on disk: a fresh read
//! changes no ref, the work tree, the index or HEAD. Teammates' changes come
//! into the work tree with `kluris sync` ([`super::sync`]) and `kluris sync
//! --watch`.
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
//! - At most once per [`THROTTLE`] per brain: the time of the last attempt,
//!   in Unix seconds, is in `<git dir>/kluris-sync/last-fetch` ([`STAMP`]),
//!   written atomically before the fetch, so an offline brain costs the
//!   time limit once a minute, not on every read.
//! - Within [`TIME_LIMIT`] overall: a git still running then is stopped with
//!   every process it started ([`crate::process`]), and the read goes on.
//! - Only a brain at the root of its own work tree: a brain nested in a
//!   larger repo (a project's) is never fetched and gets no stamp; nor is a
//!   shallow brain (deepening it would write `.git/shallow`).
//! - Nothing fails the read: no work tree, no upstream (or a detached
//!   HEAD), or a fetch that fails (offline, refused credentials) ends the
//!   step quietly, and nothing is printed.
//! - What it writes is objects and the stamp ([`SystemGit`]'s
//!   [`FreshGit::fetch_objects`]): no ref, `FETCH_HEAD`, `packed-refs`,
//!   `.git/shallow`, commit-graph or config, so it never holds a lock a
//!   stopped fetch could leave behind; every git it runs has
//!   `GIT_OPTIONAL_LOCKS=0` ([`READ_ENV`]), so none refreshes the index.
//! - No hook runs: every git after `rev-parse` (which finds the git
//!   directory and runs none) has `core.hooksPath` at [`NO_HOOKS`] under
//!   the git directory, a path that is never made; when something exists
//!   there, the read does not fetch.
//! - A fetch stopped at the deadline backs off ([`BACKOFF`]): no read
//!   fetches that brain for [`BACKOFF_FIRST`], each further stop doubles
//!   that up to [`BACKOFF_MAX`], and any fetch that succeeds (a read's or
//!   `kluris sync`'s) ends it. A slow link so leaves at most one partial
//!   pack (`tmp_pack_*`) per window, which git's gc expires; Kluris never
//!   removes git's temporary, `.keep` or lock files.
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

/// The hooks path of every git a fresh read runs after `rev-parse`,
/// relative to the git directory: it is never made, so no hook runs.
pub const NO_HOOKS: &str = "kluris-sync/no-hooks";

/// The back-off after a fetch stopped at the deadline, relative to the git
/// directory: `<until> <window>\n`, both in Unix seconds and seconds.
pub const BACKOFF: &str = "kluris-sync/fetch-backoff";

/// The file whose OS lock a read holds while it decides and fetches,
/// relative to the git directory.
pub const CLAIM: &str = "kluris-sync/fetch-claim";

/// The first back-off after a fetch stopped at the deadline.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(60 * 60);

/// The longest back-off: each stop doubles it up to this.
pub const BACKOFF_MAX: Duration = Duration::from_secs(24 * 60 * 60);

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
    /// Fetched less than [`THROTTLE`] ago.
    Throttled,
    /// A fetch was stopped at the deadline less than its back-off ago.
    BackedOff,
    /// A detached HEAD, or a branch without an upstream on a remote.
    NoUpstream,
    /// The fetch failed (offline, no such remote, refused credentials).
    FetchFailed,
    /// The time limit passed; whatever git was running was stopped.
    TimedOut,
    /// The upstream's objects were fetched.
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
    /// The branch on the remote (`branch.<name>.merge`, `refs/heads/main`).
    pub remote_ref: String,
}

/// Where the brain sits in its work tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkTree {
    /// The brain's folder relative to the work tree root, `/`-separated
    /// with a trailing `/` (`git rev-parse --show-prefix`); empty at the root.
    pub prefix: String,
    /// `git rev-parse --is-shallow-repository`.
    pub shallow: bool,
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
    /// Where `dir` sits in its work tree, whether the repository is
    /// shallow, and each of `names` inside its git directory (`git rev-parse
    /// --is-shallow-repository --show-prefix --git-path`); `None` outside a
    /// work tree.
    fn git_paths(
        &self,
        dir: &Path,
        names: &[&str],
        deadline: Instant,
    ) -> Result<Option<WorkTree>, FreshError>;

    /// The current branch's upstream; `None` on a detached HEAD, without an
    /// upstream, or with a local branch as upstream. Hooks off:
    /// `core.hooksPath` is `no_hooks`.
    fn upstream(
        &self,
        dir: &Path,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<Option<Upstream>, FreshError>;

    /// `git fetch <remote>`, never prompting (`kluris sync`'s fetch).
    fn fetch(&self, dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError>;

    /// Fetch the objects of the upstream's branch and write nothing else:
    /// no ref, `FETCH_HEAD`, tag, `packed-refs`, `.git/shallow`, commit-graph
    /// or config. Never prompting; hooks off (`core.hooksPath` is
    /// `no_hooks`).
    fn fetch_objects(
        &self,
        dir: &Path,
        upstream: &Upstream,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<(), FreshError>;

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
    let Some(tree) = git
        .git_paths(dir, &[STAMP, BACKOFF, NO_HOOKS, CLAIM], deadline)
        .map_err(ended)?
    else {
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
    let now = unix_seconds(req.now);
    if fetched_recently(stamp, now, req.throttle) {
        return Ok(Fresh::Throttled);
    }
    if backed_off(backoff, now) {
        return Ok(Fresh::BackedOff);
    }
    let Some(upstream) = git.upstream(dir, &no_hooks, deadline).map_err(ended)? else {
        write_stamp(stamp, now);
        return Ok(Fresh::NoUpstream);
    };
    write_stamp(stamp, now);
    match git.fetch_objects(dir, &upstream, &no_hooks, deadline) {
        Ok(()) => {
            end_backoff(backoff);
            Ok(Fresh::Fetched)
        }
        Err(FreshError::TimedOut) => {
            back_off(backoff, now);
            Err(Fresh::TimedOut)
        }
        Err(FreshError::Failed(_)) => Err(Fresh::FetchFailed),
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

/// The back-off's end and its window, as [`BACKOFF`] records them.
fn read_backoff(path: &Path) -> Option<(u64, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut fields = text.split_whitespace();
    let until = fields.next()?.parse::<u64>().ok()?;
    let window = fields.next()?.parse::<u64>().ok()?;
    Some((until, window))
}

/// Whether a read at `now` is inside the back-off: before its end, and no
/// further from it than its window (at most [`BACKOFF_MAX`]); one that does
/// not read, or lies further ahead (the clock went back), never blocks.
fn backed_off(path: &Path, now: u64) -> bool {
    read_backoff(path).is_some_and(|(until, window)| {
        let window = window.min(BACKOFF_MAX.as_secs());
        now < until && until - now <= window
    })
}

/// Record a fetch stopped at the deadline at `now`: the next window is
/// twice the last (the first [`BACKOFF_FIRST`]), at most [`BACKOFF_MAX`].
fn back_off(path: &Path, now: u64) {
    let window = read_backoff(path).map_or(BACKOFF_FIRST.as_secs(), |(_, last)| {
        last.saturating_mul(2)
            .clamp(BACKOFF_FIRST.as_secs(), BACKOFF_MAX.as_secs())
    });
    let until = now.saturating_add(window);
    let _ = atomic::write_atomic(path, format!("{until} {window}\n").as_bytes());
}

/// End the back-off: a fetch from the brain's remote succeeded.
pub(crate) fn end_backoff(path: &Path) {
    let _ = fs::remove_file(path);
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

impl SystemGit {
    /// [`prompt_env`] and [`READ_ENV`] for a fetch from `dir`; `config`
    /// options (`-c ...`) go before `git config`'s own arguments.
    fn no_prompt_env(
        &self,
        dir: &Path,
        options: &[&str],
        deadline: Instant,
    ) -> Result<Vec<(&'static str, &'static str)>, FreshError> {
        let mut args = options.to_vec();
        args.extend(["config", "--get", "core.sshCommand"]);
        let ssh_command = self.bounded(dir, &args, &READ_ENV, deadline)?;
        let configured =
            ssh_command.status.success() && !ssh_command.stdout.trim_ascii().is_empty();
        let mut env = prompt_env(|key| self.env_var(key), configured);
        env.extend(READ_ENV);
        Ok(env)
    }
}

/// `core.hooksPath=<no_hooks>`, for `-c`.
fn hooks_off(no_hooks: &Path) -> String {
    format!("core.hooksPath={}", no_hooks.display())
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
        let mut args = vec![
            "rev-parse",
            "--is-inside-work-tree",
            "--is-shallow-repository",
            "--show-prefix",
        ];
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
        let shallow = match lines.next() {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(unreadable(&args, "no shallow answer")),
        };
        let prefix = lines.next().unwrap_or_default().to_string();
        let paths: Vec<PathBuf> = lines.map(|line| dir.join(line)).collect();
        if paths.len() != names.len() {
            return Err(unreadable(&args, "not one path per name"));
        }
        Ok(Some(WorkTree {
            prefix,
            shallow,
            paths,
        }))
    }

    fn upstream(
        &self,
        dir: &Path,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<Option<Upstream>, FreshError> {
        let hooks = hooks_off(no_hooks);
        let symbolic = ["-c", &hooks, "symbolic-ref", "--quiet", "HEAD"];
        let head = self.bounded(dir, &symbolic, &READ_ENV, deadline)?;
        if !head.status.success() {
            return Ok(None);
        }
        let branch = String::from_utf8(head.stdout).map_err(|_| unreadable(&symbolic, NOT_UTF8))?;
        let args = [
            "-c",
            &hooks,
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream)%00%(upstream:remoteref)",
            branch.trim(),
        ];
        let out = self.checked(dir, &args, &READ_ENV, deadline)?;
        let text = String::from_utf8(out).map_err(|_| unreadable(&args, NOT_UTF8))?;
        let line = text.lines().next().unwrap_or_default();
        let mut fields = line.split('\0');
        let (Some(remote), Some(refname), Some(remote_ref)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Ok(None);
        };
        if remote.is_empty() || remote == "." || refname.is_empty() || remote_ref.is_empty() {
            return Ok(None);
        }
        Ok(Some(Upstream {
            remote: remote.to_string(),
            refname: refname.to_string(),
            remote_ref: remote_ref.to_string(),
        }))
    }

    fn fetch(&self, dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError> {
        let env = self.no_prompt_env(dir, &[], deadline)?;
        let args =
            SystemGit::with_no_background(&["fetch", "--quiet", "--no-recurse-submodules", remote]);
        self.checked(dir, &args, &env, deadline).map(drop)
    }

    fn fetch_objects(
        &self,
        dir: &Path,
        upstream: &Upstream,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<(), FreshError> {
        let hooks = hooks_off(no_hooks);
        let env = self.no_prompt_env(dir, &["-c", &hooks], deadline)?;
        // The source alone (no `:dst`) with an empty `--refmap=` updates no
        // remote-tracking ref (`remote.<name>.fetch` is ignored, and so is
        // `remote.<name>.followRemoteHEAD`); no tags, no pruning, no
        // `FETCH_HEAD`; no commit-graph, and no bundle URI (which writes
        // `refs/bundles/*` and `fetch.bundleCreationToken`).
        let args = SystemGit::with_no_background(&[
            "-c",
            &hooks,
            "-c",
            "fetch.writeCommitGraph=false",
            "-c",
            "fetch.bundleURI=",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-prune",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--refmap=",
            &upstream.remote,
            &upstream.remote_ref,
        ]);
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
