//! Fresh reads (SPEC Phase 7, TEST-K3-49): before `wake-up` and `search`
//! read a brain, a `git fetch` and a fast-forward-only update of its current
//! branch from its upstream, so an agent sees what teammates pushed.
//!
//! - At most once per [`THROTTLE`] per brain: the time of the last attempt,
//!   in Unix seconds, is in `<git dir>/kluris-sync/last-fetch` ([`STAMP`]),
//!   written atomically before the fetch, so an offline brain costs the
//!   time limit once a minute, not on every read.
//! - Within [`TIME_LIMIT`] overall: a git still running then is killed with
//!   every process it started ([`crate::process`]), and the read goes on.
//! - Only a brain at the root of its own work tree: a brain nested in a
//!   larger repo (a project's) is never fetched or moved, as a read must not
//!   move a repository the user did not ask Kluris to manage.
//! - Nothing but a clean fast-forward ever changes, and nothing fails the
//!   read: no work tree, no upstream (or a detached HEAD), a tracked change,
//!   a merge, rebase, cherry-pick or revert in progress (or an open `kluris
//!   sync` conflict), a fetch that fails
//!   (offline, refused credentials) or a branch that diverged all end the
//!   step quietly.
//! - The time limit covers only what can be abandoned safely: the fetch and
//!   the read-only checks. A fast-forward, once started, is never killed (a
//!   `git merge` stopped while it updates the work tree leaves a mixed tree
//!   and `index.lock` behind); it runs to its end. So it starts only when it
//!   is cheap and safe, and otherwise the read keeps just the fetch and the
//!   next `kluris sync` moves the branch: not with less than
//!   [`FAST_FORWARD_RESERVE`] left, not when a checkout filter would run
//!   ([`filter_would_run`]: an attribute assigns `filter=<name>` and git
//!   has a driver for that name), not for
//!   more than [`MAX_FAST_FORWARD_FILES`] incoming files, and not when an
//!   ignored file is where the incoming tree writes (checked right before
//!   it; `--no-overwrite-ignore` makes git refuse one that appears after).
//!   It runs without autostash.
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
use crate::process::{Finished, RunError, run_to_end, run_until};

/// The stamp of the last fetch, relative to the git directory.
pub const STAMP: &str = "kluris-sync/last-fetch";

/// The least time between two fetches of a brain.
pub const THROTTLE: Duration = Duration::from_secs(60);

/// The time the whole step may take.
pub const TIME_LIMIT: Duration = Duration::from_secs(3);

/// The least time left for a fast-forward to start.
pub const FAST_FORWARD_RESERVE: Duration = Duration::from_secs(1);

/// The most files a fresh read's fast-forward may write; past it the next
/// `kluris sync` moves the branch.
pub const MAX_FAST_FORWARD_FILES: usize = 200;

/// The markers of an operation in progress, relative to the git directory:
/// git's, and an open `kluris sync` conflict waiting for `kluris resolve`.
pub(super) const IN_PROGRESS: [&str; 6] = [
    "MERGE_HEAD",
    "rebase-merge",
    "rebase-apply",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    super::PENDING,
];

/// What the step did. Never shown: the read goes on the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fresh {
    /// `KLURIS_NO_AUTO_SYNC=1` or `sync.auto_fetch: false`.
    Disabled,
    /// The brain is not in a git work tree.
    NotARepo,
    /// The brain is below the root of a larger work tree.
    Nested,
    /// Fetched less than [`THROTTLE`] ago.
    Throttled,
    /// A merge, rebase, cherry-pick or revert is in progress.
    Busy,
    /// A detached HEAD, or a branch without an upstream on a remote.
    NoUpstream,
    /// A tracked file has a change, staged or not.
    Dirty,
    /// The fetch failed (offline, no such remote, refused credentials).
    FetchFailed,
    /// The time limit passed; whatever git was running was killed.
    TimedOut,
    /// The branch has every upstream commit (equal or ahead).
    UpToDate,
    /// Both sides have commits the other lacks.
    Diverged,
    /// Fetched only: a checkout filter is configured or set.
    Filtered,
    /// Fetched only: more than [`MAX_FAST_FORWARD_FILES`] incoming files.
    TooBig,
    /// Fetched only: an ignored file is where the incoming tree writes.
    IgnoredInTheWay,
    /// The branch moved to its upstream.
    FastForwarded,
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
    /// The deadline passed and git was killed.
    TimedOut,
    /// git could not run or failed.
    Failed(GitError),
}

/// The git steps of a fresh read, each bounded by `deadline`.
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

    /// Whether any tracked file of the work tree differs from HEAD, in the
    /// index or on disk.
    fn has_tracked_changes(&self, dir: &Path, deadline: Instant) -> Result<bool, FreshError>;

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

    /// The paths that differ between `HEAD` and `to`: what a fast-forward
    /// to `to` writes or removes.
    fn incoming(&self, dir: &Path, to: &str, deadline: Instant) -> Result<Vec<String>, FreshError>;

    /// Whether a checkout filter would run ([`filter_would_run`]): an
    /// attribute assigns `filter=<name>` (a `.gitattributes` of `HEAD` or
    /// `to`, `<git dir>/info/attributes`, the `core.attributesFile`) and git
    /// config, in any scope, has a smudge, clean or process driver for it.
    fn uses_filters(&self, dir: &Path, to: &str, deadline: Instant) -> Result<bool, FreshError>;

    /// The work tree's ignored files, a wholly ignored folder as `dir/`.
    fn ignored(&self, dir: &Path, deadline: Instant) -> Result<Vec<String>, FreshError>;

    /// Fast-forward the current branch and work tree to `to`, or fail,
    /// never over an ignored file and without autostash. No deadline: once
    /// started it runs to its end.
    fn fast_forward(&self, dir: &Path, to: &str) -> Result<(), FreshError>;
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

/// Fetch and fast-forward the brain if it is time to and safe (see the
/// module documentation).
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
    let mut names = vec![STAMP];
    names.extend(IN_PROGRESS);
    let Some(tree) = git.git_paths(dir, &names, deadline).map_err(ended)? else {
        return Ok(Fresh::NotARepo);
    };
    if !tree.prefix.is_empty() {
        return Ok(Fresh::Nested);
    }
    let Some((stamp, markers)) = tree.paths.split_first() else {
        return Ok(Fresh::Failed);
    };
    let now = unix_seconds(req.now);
    if fetched_recently(stamp, now, req.throttle) {
        return Ok(Fresh::Throttled);
    }
    if in_progress(markers) {
        return Ok(Fresh::Busy);
    }
    let Some(upstream) = git.upstream(dir, deadline).map_err(ended)? else {
        write_stamp(stamp, now);
        return Ok(Fresh::NoUpstream);
    };
    if git.has_tracked_changes(dir, deadline).map_err(ended)? {
        return Ok(Fresh::Dirty);
    }
    write_stamp(stamp, now);
    git.fetch(dir, &upstream.remote, deadline)
        .map_err(|e| match e {
            FreshError::TimedOut => Fresh::TimedOut,
            FreshError::Failed(_) => Fresh::FetchFailed,
        })?;
    let theirs = upstream.refname.as_str();
    if git
        .is_ancestor(dir, theirs, "HEAD", deadline)
        .map_err(ended)?
    {
        return Ok(Fresh::UpToDate);
    }
    if !git
        .is_ancestor(dir, "HEAD", theirs, deadline)
        .map_err(ended)?
    {
        return Ok(Fresh::Diverged);
    }
    let incoming = git.incoming(dir, theirs, deadline).map_err(ended)?;
    if incoming.len() > MAX_FAST_FORWARD_FILES {
        return Ok(Fresh::TooBig);
    }
    if git.uses_filters(dir, theirs, deadline).map_err(ended)? {
        return Ok(Fresh::Filtered);
    }
    if deadline.saturating_duration_since(Instant::now()) < FAST_FORWARD_RESERVE {
        return Ok(Fresh::TimedOut);
    }
    // The tree may have changed during the fetch.
    if in_progress(markers) {
        return Ok(Fresh::Busy);
    }
    if git.has_tracked_changes(dir, deadline).map_err(ended)? {
        return Ok(Fresh::Dirty);
    }
    let ignored = git.ignored(dir, deadline).map_err(ended)?;
    if ignored_in_the_way(&ignored, &incoming) {
        return Ok(Fresh::IgnoredInTheWay);
    }
    git.fast_forward(dir, theirs).map_err(ended)?;
    Ok(Fresh::FastForwarded)
}

/// Whether a checkout filter would run: one of the attribute texts assigns
/// `filter=<name>` (in any line, a `[attr]` macro included; `#` lines are
/// comments) and the git config keys (`git config --name-only`) define
/// `filter.<name>.smudge`, `.clean` or `.process` for that name. A driver
/// nothing names, and a name with no driver, run nothing.
pub fn filter_would_run(attributes: &[String], config_keys: &[String]) -> bool {
    let drivers = filter_drivers(config_keys);
    if drivers.is_empty() {
        return false;
    }
    attributes.iter().flat_map(|text| text.lines()).any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.split_whitespace()
            .skip(1)
            .filter_map(|token| token.strip_prefix("filter="))
            .any(|name| drivers.contains(&name))
    })
}

/// The filter names the git config keys define a smudge, clean or process
/// driver for (`filter.<name>.<variable>`; the section and the variable
/// are case-insensitive, the name is not).
fn filter_drivers(config_keys: &[String]) -> Vec<&str> {
    config_keys
        .iter()
        .filter_map(|key| {
            let (section, rest) = key.split_once('.')?;
            let (name, variable) = rest.rsplit_once('.')?;
            let runs = ["smudge", "clean", "process"]
                .iter()
                .any(|v| variable.eq_ignore_ascii_case(v));
            (section.eq_ignore_ascii_case("filter") && runs).then_some(name)
        })
        .collect()
}

/// Whether an ignored file or folder (`dir/`) is where the `incoming` paths
/// go: the same path, a folder an incoming file goes into, a file where an
/// incoming file needs a folder, or a file inside a folder that becomes an
/// incoming file. All paths are `/`-separated from the work tree root.
pub fn ignored_in_the_way(ignored: &[String], incoming: &[String]) -> bool {
    let under = |path: &str, folder: &str| {
        path.strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
    };
    ignored.iter().any(|entry| {
        let entry = entry.trim_end_matches('/');
        incoming
            .iter()
            .any(|path| path == entry || under(path, entry) || under(entry, path))
    })
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

fn in_progress(markers: &[PathBuf]) -> bool {
    markers.iter().any(|m| fs::symlink_metadata(m).is_ok())
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
        let done = self.bounded(dir, &args, &[], deadline)?;
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
        let head = self.bounded(dir, &["symbolic-ref", "--quiet", "HEAD"], &[], deadline)?;
        if !head.status.success() {
            return Ok(None);
        }
        let branch =
            String::from_utf8(head.stdout).map_err(|_| unreadable(&["symbolic-ref"], NOT_UTF8))?;
        let args = [
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream)",
            branch.trim(),
        ];
        let out = self.checked(dir, &args, &[], deadline)?;
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

    fn has_tracked_changes(&self, dir: &Path, deadline: Instant) -> Result<bool, FreshError> {
        let args = [
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ];
        let out = self.checked(dir, &args, &[], deadline)?;
        Ok(!out.is_empty())
    }

    fn fetch(&self, dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError> {
        let ssh_command =
            self.bounded(dir, &["config", "--get", "core.sshCommand"], &[], deadline)?;
        let configured =
            ssh_command.status.success() && !ssh_command.stdout.trim_ascii().is_empty();
        let env = prompt_env(|key| self.env_var(key), configured);
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

    fn incoming(&self, dir: &Path, to: &str, deadline: Instant) -> Result<Vec<String>, FreshError> {
        let args = [
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "HEAD",
            to,
            "--",
        ];
        let out = self.checked(dir, &args, &[], deadline)?;
        nul_separated(&out, &args)
    }

    fn uses_filters(&self, dir: &Path, to: &str, deadline: Instant) -> Result<bool, FreshError> {
        let config = ["config", "-z", "--name-only", "--get-regexp", r"^filter\."];
        let done = self.bounded(dir, &config, &[], deadline)?;
        let keys = match done.status.code() {
            Some(0) => nul_separated(&done.stdout, &config)?,
            Some(1) => return Ok(false),
            code => return Err(failed(&config, code, &done.stderr)),
        };
        if filter_drivers(&keys).is_empty() {
            return Ok(false);
        }
        let mut attributes = Vec::new();
        let grep = [
            "grep",
            "-h",
            "-I",
            "-e",
            "filter=",
            "HEAD",
            to,
            "--",
            ":(glob)**/.gitattributes",
        ];
        let done = self.bounded(dir, &grep, &[], deadline)?;
        match done.status.code() {
            Some(0) => attributes.push(String::from_utf8_lossy(&done.stdout).into_owned()),
            Some(1) => {}
            code => return Err(failed(&grep, code, &done.stderr)),
        }
        for file in self.attribute_files(dir, deadline)? {
            if let Ok(bytes) = fs::read(&file) {
                attributes.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        Ok(filter_would_run(&attributes, &keys))
    }

    fn ignored(&self, dir: &Path, deadline: Instant) -> Result<Vec<String>, FreshError> {
        let args = [
            "-c",
            "core.quotePath=false",
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ];
        let out = self.checked(dir, &args, &[], deadline)?;
        nul_separated(&out, &args)
    }

    fn fast_forward(&self, dir: &Path, to: &str) -> Result<(), FreshError> {
        let args = SystemGit::with_no_background(&[
            "-c",
            "merge.autostash=false",
            "merge",
            "--ff-only",
            "--no-overwrite-ignore",
            "--quiet",
            to,
        ]);
        let done = match run_to_end(self.command(Some(dir), &args)) {
            Ok(done) => done,
            Err(RunError::Spawn(e)) => {
                return Err(FreshError::Failed(GitError {
                    args: args.iter().map(|a| a.to_string()).collect(),
                    code: None,
                    detail: e.to_string(),
                }));
            }
            Err(RunError::TimedOut) => return Err(FreshError::TimedOut),
        };
        if done.status.success() {
            Ok(())
        } else {
            Err(failed(&args, done.status.code(), &done.stderr))
        }
    }
}

impl SystemGit {
    /// The attribute files outside the trees: `<git dir>/info/attributes`
    /// and the `core.attributesFile` (when unset, git's default,
    /// `$XDG_CONFIG_HOME/git/attributes` or `~/.config/git/attributes`).
    fn attribute_files(&self, dir: &Path, deadline: Instant) -> Result<Vec<PathBuf>, FreshError> {
        let info = ["rev-parse", "--git-path", "info/attributes"];
        let out = self.checked(dir, &info, &[], deadline)?;
        let info = String::from_utf8(out).map_err(|_| unreadable(&info, NOT_UTF8))?;
        let mut files = vec![dir.join(info.trim_end_matches(['\r', '\n']))];
        let global = ["config", "--path", "--get", "core.attributesFile"];
        let done = self.bounded(dir, &global, &[], deadline)?;
        match done.status.code() {
            Some(0) => {
                let path =
                    String::from_utf8(done.stdout).map_err(|_| unreadable(&global, NOT_UTF8))?;
                files.push(dir.join(path.trim_end_matches(['\r', '\n'])));
            }
            Some(1) => {
                let xdg = self.env_var("XDG_CONFIG_HOME").filter(|v| !v.is_empty());
                let home = self.env_var("HOME").filter(|v| !v.is_empty());
                let base = xdg
                    .map(PathBuf::from)
                    .or_else(|| home.map(|h| PathBuf::from(h).join(".config")));
                if let Some(base) = base {
                    files.push(base.join("git").join("attributes"));
                }
            }
            code => return Err(failed(&global, code, &done.stderr)),
        }
        Ok(files)
    }
}

/// git exited `code`, which this step does not expect.
fn failed(args: &[&str], code: Option<i32>, stderr: &[u8]) -> FreshError {
    FreshError::Failed(GitError {
        args: args.iter().map(|a| a.to_string()).collect(),
        code,
        detail: String::from_utf8_lossy(stderr).trim().to_string(),
    })
}

/// NUL-terminated paths, as `-z` prints them.
fn nul_separated(out: &[u8], args: &[&str]) -> Result<Vec<String>, FreshError> {
    out.split(|b| *b == 0)
        .filter(|item| !item.is_empty())
        .map(|item| String::from_utf8(item.to_vec()).map_err(|_| unreadable(args, NOT_UTF8)))
        .collect()
}
