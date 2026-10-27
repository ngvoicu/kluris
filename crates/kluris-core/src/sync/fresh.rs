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
//!   [`FAST_FORWARD_RESERVE`] left, not for more than
//!   [`MAX_FAST_FORWARD_FILES`] incoming files, and not when it could stop
//!   partway (git then leaves HEAD and the index as they were but the
//!   files it already wrote changed): a checkout filter would run on an
//!   incoming path ([`filter_would_run`]: git's effective attributes, from
//!   `git check-attr`, or the upstream's `.gitattributes`, assign
//!   `filter=<name>` and git has a driver for that name), a path would be
//!   too long for Windows ([`windows_path_too_long`], without
//!   `core.longpaths`), a folder it writes in is not writable, or the
//!   incoming files would not fit on the disk ([`fits_on_disk`]). Last, it
//!   does not start when an ignored file is where the incoming tree writes
//!   (`--no-overwrite-ignore` makes git refuse one that appears after). It
//!   runs without autostash. On Windows it does not start either when
//!   another program holds a file it replaces or deletes ([`files_in_use`]);
//!   one opened between that check and git's write is a race left open. A
//!   failing `post-merge` hook runs after the fast-forward is whole and
//!   changes nothing.
//! - A started fast-forward that still fails (git exits non-zero) is
//!   [`Fresh::PartlyUpdated`], which the command reports in one stderr line
//!   ([`Fresh::report`]), so a mixed tree is never silent.
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
    /// Fetched only: a path would be too long for Windows.
    PathTooLong,
    /// Fetched only: a folder the fast-forward writes in is not writable.
    Unwritable,
    /// Fetched only: the incoming files might not fit on the disk.
    NoSpace,
    /// Fetched only (Windows): another program holds a file the
    /// fast-forward replaces or deletes.
    InUse,
    /// The branch moved to its upstream.
    FastForwarded,
    /// The fast-forward started and git exited non-zero: the work tree may
    /// be partly updated.
    PartlyUpdated,
    /// Another git step failed.
    Failed,
}

impl Fresh {
    /// The one stderr line a command prints after this step: only for a
    /// brain left partly updated.
    pub fn report(&self, brain: &str) -> Option<String> {
        (*self == Fresh::PartlyUpdated).then(|| {
            format!(
                "kluris: {brain} was only partly updated from its remote; run `kluris sync --brain {brain}` to finish"
            )
        })
    }
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

    /// Whether a checkout filter would run on one of the `incoming` paths
    /// ([`filter_would_run`]): git's effective attributes for them (`git
    /// check-attr filter`: the work tree's `.gitattributes` at every level,
    /// tracked, untracked or ignored, `<git dir>/info/attributes` and the
    /// `core.attributesFile`), or a `.gitattributes` of `to`, assign
    /// `filter=<name>`, and git config, in any scope, has a smudge, clean or
    /// process driver for it.
    fn uses_filters(
        &self,
        dir: &Path,
        to: &str,
        incoming: &[String],
        deadline: Instant,
    ) -> Result<bool, FreshError>;

    /// The bytes `to`'s versions of the `incoming` paths take.
    fn incoming_size(
        &self,
        dir: &Path,
        to: &str,
        incoming: &[String],
        deadline: Instant,
    ) -> Result<u64, FreshError>;

    /// `core.longpaths` (Git for Windows' long path support).
    fn long_paths(&self, dir: &Path, deadline: Instant) -> Result<bool, FreshError>;

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
    if git
        .uses_filters(dir, theirs, &incoming, deadline)
        .map_err(ended)?
    {
        return Ok(Fresh::Filtered);
    }
    if cfg!(windows)
        && incoming.iter().any(|p| windows_path_too_long(dir, p))
        && !git.long_paths(dir, deadline).map_err(ended)?
    {
        return Ok(Fresh::PathTooLong);
    }
    if !folders_writable(dir, &incoming) {
        return Ok(Fresh::Unwritable);
    }
    let size = git
        .incoming_size(dir, theirs, &incoming, deadline)
        .map_err(ended)?;
    let files = u64::try_from(incoming.len()).unwrap_or(u64::MAX);
    if free_space(dir).is_some_and(|free| !fits_on_disk(size, files, free)) {
        return Ok(Fresh::NoSpace);
    }
    let ignored = git.ignored(dir, deadline).map_err(ended)?;
    if ignored_in_the_way(&ignored, &incoming) {
        return Ok(Fresh::IgnoredInTheWay);
    }
    if files_in_use(dir, &incoming) {
        return Ok(Fresh::InUse);
    }
    match git.fast_forward(dir, theirs) {
        Ok(()) => Ok(Fresh::FastForwarded),
        Err(FreshError::Failed(GitError { code: Some(_), .. })) => Ok(Fresh::PartlyUpdated),
        Err(e) => Err(ended(e)),
    }
}

/// Whether another program holds a file the fast-forward replaces or
/// deletes: Windows only, where git cannot replace a file open without
/// delete sharing. Each existing regular file among `incoming` is opened
/// with no sharing and closed at once; a sharing violation, a lock
/// violation or access denied means it is held. Elsewhere `false`: an open
/// file does not stop its replacement.
#[cfg(windows)]
pub fn files_in_use(dir: &Path, incoming: &[String]) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    incoming.iter().any(|rel| {
        let path = dir.join(rel);
        if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            return false;
        }
        match fs::OpenOptions::new().read(true).share_mode(0).open(&path) {
            Ok(_) => false,
            Err(e) => matches!(
                e.raw_os_error(),
                Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            ),
        }
    })
}

#[cfg(not(windows))]
pub fn files_in_use(_dir: &Path, _incoming: &[String]) -> bool {
    false
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

/// The space kept free on the disk beside the incoming files.
const SPARE_SPACE: u64 = 64 * 1024 * 1024;

/// The block a file's size is rounded up to.
const BLOCK: u64 = 4096;

/// Whether `bytes` in `files` files fit in `free` bytes with
/// [`SPARE_SPACE`] to spare, each file rounded up to a [`BLOCK`] (the
/// space the old versions free is not counted).
pub fn fits_on_disk(bytes: u64, files: u64, free: u64) -> bool {
    // A need too large to count never fits.
    let needed = bytes
        .div_ceil(BLOCK)
        .checked_mul(BLOCK)
        .and_then(|n| n.checked_add(files.saturating_sub(1).checked_mul(BLOCK)?))
        .and_then(|n| n.checked_add(SPARE_SPACE));
    needed.is_some_and(|needed| needed <= free)
}

/// The longest path Windows takes without long path support: a folder
/// must leave room for an 8.3 name (`MAX_PATH` 260 less 12).
const WINDOWS_MAX_PATH: usize = 247;

/// Whether `root` joined with the `/`-separated `rel` is longer than
/// Windows takes without `core.longpaths` (counted in UTF-16 units).
pub fn windows_path_too_long(root: &Path, rel: &str) -> bool {
    let root = root.as_os_str().to_string_lossy();
    let root = root.trim_end_matches(['/', '\\']);
    root.encode_utf16().count() + 1 + rel.encode_utf16().count() > WINDOWS_MAX_PATH
}

/// Whether the fast-forward can write where the `incoming` paths go: for
/// each, the nearest existing folder at or above its parent (below `dir`)
/// is writable and searchable by this user. Windows: not checked (a folder's
/// read-only attribute does not block writes; ACLs and locks are not seen).
#[cfg(unix)]
fn folders_writable(dir: &Path, incoming: &[String]) -> bool {
    use rustix::fs::{Access, AtFlags, CWD, accessat};
    incoming.iter().all(|rel| {
        let target = dir.join(rel);
        let mut folder = target.parent().unwrap_or(dir);
        while folder != dir && fs::symlink_metadata(folder).is_err() {
            folder = folder.parent().unwrap_or(dir);
        }
        accessat(
            CWD,
            folder,
            Access::WRITE_OK | Access::EXEC_OK,
            AtFlags::EACCESS,
        )
        .is_ok()
    })
}

#[cfg(not(unix))]
fn folders_writable(_dir: &Path, _incoming: &[String]) -> bool {
    true
}

/// The bytes free for this user on the disk holding `dir`; `None` when it
/// cannot be told (and then not checked).
#[cfg(unix)]
fn free_space(dir: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(dir).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

#[cfg(windows)]
fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is NUL-terminated and lives across the call; `free`
    // is a live u64; the other two outputs may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn free_space(_dir: &Path) -> Option<u64> {
    None
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

    fn uses_filters(
        &self,
        dir: &Path,
        to: &str,
        incoming: &[String],
        deadline: Instant,
    ) -> Result<bool, FreshError> {
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
        // The checkout reads the incoming tree's `.gitattributes` too.
        let grep = [
            "grep",
            "-h",
            "-I",
            "-e",
            "filter=",
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
        // git's own view of every incoming path's attributes now.
        if !incoming.is_empty() {
            let mut check = vec![
                "-c",
                "core.quotePath=false",
                "check-attr",
                "-z",
                "filter",
                "--",
            ];
            check.extend(incoming.iter().map(String::as_str));
            let out = self.checked(dir, &check, &[], deadline)?;
            let fields = nul_separated(&out, &check)?;
            for record in fields.chunks(3) {
                if let [path, _, value] = record
                    && !matches!(value.as_str(), "unspecified" | "unset" | "set")
                {
                    attributes.push(format!("{path} filter={value}"));
                }
            }
        }
        Ok(filter_would_run(&attributes, &keys))
    }

    fn incoming_size(
        &self,
        dir: &Path,
        to: &str,
        incoming: &[String],
        deadline: Instant,
    ) -> Result<u64, FreshError> {
        if incoming.is_empty() {
            return Ok(0);
        }
        let mut args = vec![
            "--literal-pathspecs",
            "-c",
            "core.quotePath=false",
            "ls-tree",
            "-r",
            "-l",
            "-z",
            to,
            "--",
        ];
        args.extend(incoming.iter().map(String::as_str));
        let out = self.checked(dir, &args, &[], deadline)?;
        // `<mode> <type> <object> <size>\t<path>`; `-` for a non-blob.
        let total = nul_separated(&out, &args)?
            .iter()
            .filter_map(|entry| entry.split('\t').next()?.split_whitespace().nth(3))
            .filter_map(|size| size.parse::<u64>().ok())
            .fold(0u64, u64::saturating_add);
        Ok(total)
    }

    fn long_paths(&self, dir: &Path, deadline: Instant) -> Result<bool, FreshError> {
        let args = ["config", "--bool", "--get", "core.longpaths"];
        let done = self.bounded(dir, &args, &[], deadline)?;
        match done.status.code() {
            Some(0) => Ok(done.stdout.trim_ascii() == b"true"),
            Some(1) => Ok(false),
            code => Err(failed(&args, code, &done.stderr)),
        }
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
