//! The background push: the commits a read or housekeeping session leaves
//! waiting are pushed by one detached `kluris sync --background --path
//! <root>...` ([`start`]), which outlives the command that started it and
//! whose result the next command reports ([`LastPush`]).
//!
//! **What starts one.** Closing a read's sessions with commits waiting
//! ([`super::session::close_all`]) starts one child for every such brain, the
//! program the caller named. A brain is handed to it only when
//! [`wanted`]: no live run holds its [`LOCK`], and its last push did not
//! fail ([`PushResult::holds_off`]) in the last [`HOLD_OFF`].
//!
//! **What the child does** ([`run`]), for each brain in turn, never
//! touching its work tree, index or HEAD, and never the tree claim:
//! 1. it takes the brain's [`LOCK`] (`{"pid", "host", "started"}`) by
//!    creating the file whole, never over a live one; a stale one (older
//!    than [`STALE`], or a pid gone on this host) is replaced; it is
//!    removed once the brain is done;
//! 2. it sends its stderr, git's and the hooks' included, to [`LOG`],
//!    replaced by each run;
//! 3. under the remote claim (waited for, as an explicit command does) it
//!    learns the upstream's commit from its own `ls-remote`; one the pinned
//!    branch does not hold is `needs_pull`, and nothing lands. Otherwise it
//!    pushes the pinned branch with the engine's push, git's own `git
//!    push` with `pre-push`; a push the remote turned back because it
//!    moved is `needs_pull` too;
//! 4. it writes [`LAST_PUSH`] whole ([`crate::atomic`]).
//!
//! A brain's root reaches it absolute: [`start`] resolves each against the
//! caller's working directory, since the child runs in the temp folder,
//! and [`run`] skips a root that is not absolute, touching nothing of it.
//!
//! None of the gits it starts can prompt. The first, local queries (git's
//! version, the repository, `core.sshCommand`) run before the engine's
//! remote environment is taken; every git after them has it, the fetch and
//! the push among them, and so does every hook and every git a hook
//! starts, since the variables are in git's own environment.
//!
//! **Reporting the record** ([`report_last_push`]): a failure is reported
//! by every command until a push succeeds, and never removed by one; a
//! success is reported by one command at most. That one claims it
//! ([`claim_last_push`]): under the lock of [`LAST_PUSH_CLAIM`], it moves
//! the record to a name of its own, reads it there, and removes it only
//! when it is what it meant to take; else it puts it back, unless a newer
//! record is there by then. An explicit
//! push that succeeds removes the failure it read before the push the same
//! way, and only that one.
//!
//! The lock, the log and the record belong to the remote, so they are in
//! the repository's common git directory, like the remote claim.
//!
//! **Detaching** ([`start`]): Unix, a session of its own (`setsid`), null
//! standard streams, and every descriptor from 3 up closed at `exec`, set
//! so in the child itself; Windows, `CreateProcessW` with no handle
//! inherited, detached, in a group of its own, out of this process's job.
//! Where the job forbids breakaway and ends its processes with it, or
//! cannot be read, nothing is started: the commits stay counted in
//! `to_push`. The child is never waited for, and a failed start never
//! fails the command.

#[cfg(windows)]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::claim::COMMAND_BOUND;
use super::engine::{self, Pushed};
use super::git::SyncGit;
use super::{LeftAsIs, LeftReason, NETWORK_TIME_LIMIT, SyncRequest};
use crate::atomic;
use crate::dream::DreamOptions;
use crate::git::SystemGit;
use crate::json::{Json, to_string};
use crate::yaml::Date;

/// The background run's lock, relative to the common git directory.
pub const LOCK: &str = "kluris-sync/background.lock";

/// The background run's stderr, relative to the common git directory.
pub const LOG: &str = "kluris-sync/background.log";

/// What the last background push did, relative to the common git
/// directory.
pub const LAST_PUSH: &str = "kluris-sync/last-push.json";

/// The file whose OS lock one claim of the record holds at a time,
/// relative to the common git directory; made when missing, never
/// removed.
pub const LAST_PUSH_CLAIM: &str = "kluris-sync/last-push.claim";

/// How old a lock is when it is stale, whatever its pid.
pub const STALE: Duration = Duration::from_secs(10 * 60);

/// How long a failed push holds off the next automatic start.
pub const HOLD_OFF: Duration = Duration::from_secs(10 * 60);

/// How a background push ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushResult {
    Pushed,
    Offline,
    Refused,
    Failed,
    /// The remote has commits the brain does not: a pull comes first.
    NeedsPull,
}

impl PushResult {
    const ALL: [PushResult; 5] = [
        PushResult::Pushed,
        PushResult::Offline,
        PushResult::Refused,
        PushResult::Failed,
        PushResult::NeedsPull,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PushResult::Pushed => "pushed",
            PushResult::Offline => "offline",
            PushResult::Refused => "refused",
            PushResult::Failed => "failed",
            PushResult::NeedsPull => "needs_pull",
        }
    }

    /// The result [`PushResult::as_str`] spells `name`.
    pub fn from_name(name: &str) -> Option<PushResult> {
        PushResult::ALL.into_iter().find(|r| r.as_str() == name)
    }

    /// Whether it holds off automatic starts for [`HOLD_OFF`]: a push that
    /// failed would fail again.
    pub fn holds_off(self) -> bool {
        matches!(
            self,
            PushResult::Offline | PushResult::Refused | PushResult::Failed
        )
    }
}

/// The record of the last background push ([`LAST_PUSH`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastPush {
    /// When it ended, RFC 3339 in UTC.
    pub at: String,
    /// The pinned branch's commit it pushed or meant to.
    pub head: String,
    pub result: PushResult,
    pub detail: Option<String>,
    /// The commits it pushed, or that wait.
    pub commits: usize,
}

fn count_json(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

impl LastPush {
    /// The record at `path`; `None` when there is none or it does not read.
    pub fn read(path: &Path) -> Option<LastPush> {
        let text = fs::read_to_string(path).ok()?;
        let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(LastPush {
            at: doc.get("at")?.as_str()?.to_string(),
            head: doc.get("head")?.as_str()?.to_string(),
            result: PushResult::from_name(doc.get("result")?.as_str()?)?,
            detail: doc.get("detail")?.as_str().map(str::to_string),
            commits: usize::try_from(doc.get("commits")?.as_u64()?).ok()?,
        })
    }

    /// Write the record to `path` whole: a reader sees the old one or this.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let detail = self.detail.clone().map_or(Json::Null, Json::Str);
        let doc = Json::Object(vec![
            ("at".into(), Json::Str(self.at.clone())),
            ("head".into(), Json::Str(self.head.clone())),
            ("result".into(), Json::Str(self.result.as_str().into())),
            ("detail".into(), detail),
            ("commits".into(), count_json(self.commits)),
        ]);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        atomic::write_atomic(path, format!("{}\n", to_string(&doc)).as_bytes())
    }

    /// As a command reports it: `{at, result, detail, commits}`.
    pub fn to_json(&self) -> Json {
        Json::Object(vec![
            ("at".into(), Json::Str(self.at.clone())),
            ("result".into(), Json::Str(self.result.as_str().into())),
            (
                "detail".into(),
                self.detail.clone().map_or(Json::Null, Json::Str),
            ),
            ("commits".into(), count_json(self.commits)),
        ])
    }
}

/// `time` as RFC 3339 in UTC, to the second.
pub fn timestamp(time: SystemTime) -> String {
    jiff::Timestamp::from_second(unix(time))
        .map(|t| t.to_string())
        .unwrap_or_default()
}

/// The Unix seconds an RFC 3339 `text` names.
pub fn seconds(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>().ok().map(|t| t.as_second())
}

fn unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Whether `text` names a time within `window` of `now`, either side.
fn within(text: &str, now: SystemTime, window: Duration) -> bool {
    let window = i64::try_from(window.as_secs()).unwrap_or(i64::MAX);
    seconds(text).is_some_and(|at| (unix(now) - at).abs() < window)
}

/// The background run's lock ([`LOCK`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub pid: u32,
    pub host: String,
    /// When the run started, RFC 3339 in UTC.
    pub started: String,
}

impl Lock {
    /// The lock at `path`; `None` when there is none or it does not read.
    pub fn read(path: &Path) -> Option<Lock> {
        let text = fs::read_to_string(path).ok()?;
        let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(Lock {
            pid: u32::try_from(doc.get("pid")?.as_u64()?).ok()?,
            host: doc.get("host")?.as_str()?.to_string(),
            started: doc.get("started")?.as_str()?.to_string(),
        })
    }

    /// Whether a run may still hold it at `now`: started less than
    /// [`STALE`] ago, and, on this host, by a process that is still there.
    pub fn live(&self, now: SystemTime) -> bool {
        within(&self.started, now, STALE) && (self.host != this_host() || alive(self.pid))
    }

    fn text(&self) -> String {
        let doc = Json::Object(vec![
            ("pid".into(), Json::Int(i64::from(self.pid))),
            ("host".into(), Json::Str(self.host.clone())),
            ("started".into(), Json::Str(self.started.clone())),
        ]);
        format!("{}\n", to_string(&doc))
    }
}

/// This machine's name.
pub fn this_host() -> String {
    #[cfg(unix)]
    {
        rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }
}

/// Whether the brain whose common git directory is `common` is handed to
/// a background push at `now`: no live run holds its lock, and its last
/// push did not fail in the last [`HOLD_OFF`].
pub fn wanted(common: &Path, now: SystemTime) -> bool {
    let running = Lock::read(&common.join(LOCK)).is_some_and(|lock| lock.live(now));
    let held_off = LastPush::read(&common.join(LAST_PUSH))
        .is_some_and(|last| last.result.holds_off() && within(&last.at, now, HOLD_OFF));
    !running && !held_off
}

/// What [`start`] did.
#[derive(Debug)]
pub enum Start {
    /// The child runs, with this process id.
    Started(u32),
    /// Nothing was started, and why: a child would end with this
    /// process.
    Refused(String),
    /// The child could not be started.
    Failed(io::Error),
}

/// Start `program sync --background --path <root>...`, detached (see the
/// module docs), and never wait for it. Each root is made absolute against
/// this process's working directory first; one that cannot be is left out.
pub fn start(program: &Path, roots: &[PathBuf]) -> Start {
    let mut args: Vec<OsString> = vec!["sync".into(), "--background".into()];
    for root in roots {
        if let Ok(root) = std::path::absolute(root) {
            args.push("--path".into());
            args.push(root.into_os_string());
        }
    }
    // No folder of a brain's is kept in use by the child.
    detach::spawn(program, &args, &std::env::temp_dir())
}

/// The child: push each brain at `roots` in turn (see the module docs),
/// with `git` as the base of every git it runs. A root that is not
/// absolute is skipped: it names a folder of this process's working
/// directory, not the brain the command opened.
pub fn run(roots: &[PathBuf], git: &SystemGit) {
    for root in roots.iter().filter(|root| root.is_absolute()) {
        push_brain(root, git);
    }
}

/// What a command reports of the last background push of the brain whose
/// common git directory is `common`: a failure as it is, left in place; a
/// success once, claimed ([`claim_last_push`]); `None` when there is
/// none, or another command claimed it.
pub fn report_last_push(common: &Path) -> Option<LastPush> {
    let record = LastPush::read(&common.join(LAST_PUSH))?;
    if record.result != PushResult::Pushed {
        return Some(record);
    }
    claim_last_push(common, |claimed| claimed.result == PushResult::Pushed)
}

/// Take the record of the brain whose common git directory is `common` if
/// `accept` says it is the one to take: it is moved to a name of this
/// call's own (so no other caller takes it too), read there, and removed
/// when accepted. One not accepted (a newer record, published between the
/// caller's look and this claim) is put back, unless a newer one still is
/// there by then (a link never replaces a file). `None` when nothing was
/// taken: none was there (another caller claimed it), or it was put back.
///
/// One claim runs at a time, under the OS lock of [`LAST_PUSH_CLAIM`]: on
/// Windows a rename goes through a handle opened on the file first, so two
/// renames of one record, started together, could each move it in turn.
/// The move still guards against the background run, which publishes a
/// new record without taking that lock.
pub fn claim_last_push(common: &Path, accept: impl FnOnce(&LastPush) -> bool) -> Option<LastPush> {
    static CLAIMS: AtomicU64 = AtomicU64::new(0);
    let gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(common.join(LAST_PUSH_CLAIM))
        .ok()?;
    gate.lock().ok()?;
    let path = common.join(LAST_PUSH);
    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".claimed-{}-{}",
        std::process::id(),
        CLAIMS.fetch_add(1, Ordering::Relaxed)
    ));
    let claimed = PathBuf::from(name);
    fs::rename(&path, &claimed).ok()?;
    let taken = LastPush::read(&claimed).filter(|record| accept(record));
    if taken.is_none() {
        let _ = fs::hard_link(&claimed, &path);
    }
    let _ = fs::remove_file(&claimed);
    // Closing the file unlocks it too.
    let _ = gate.unlock();
    taken
}

/// What the maps a merge rebuilds would say: the background push merges
/// nothing.
fn no_maps() -> Option<DreamOptions<'static>> {
    Some(DreamOptions {
        name: "",
        description: "",
        today: Date::new(1970, 1, 1)?,
    })
}

fn push_brain(root: &Path, git: &SystemGit) {
    let Some(maps) = no_maps() else {
        return;
    };
    let request = SyncRequest {
        no_verify: false,
        maps,
        network_time_limit: NETWORK_TIME_LIMIT,
        tree_bound: COMMAND_BOUND,
    };
    let Ok(mut ctx) = engine::open(root, git, &request) else {
        return;
    };
    ctx.wait_for_remote(COMMAND_BOUND);
    let common = ctx.dirs().common.clone();
    let Ok(Some(_lock)) = Held::take(&common.join(LOCK)) else {
        return;
    };
    let _log = log::Log::open(&common.join(LOG));
    eprintln!(
        "kluris sync --background (pid {}): {}",
        std::process::id(),
        root.display()
    );
    if let Some(record) = push(&ctx)
        && let Err(e) = record.write(&common.join(LAST_PUSH))
    {
        eprintln!("kluris: cannot record the push: {e}");
    }
}

/// Fetch, then push the pinned branch when the remote holds nothing it
/// lacks; the record of it, or `None` when there was nothing to push or
/// the remote claim stayed held.
fn push<G: SyncGit>(ctx: &engine::Context<G>) -> Option<LastPush> {
    let upstream = ctx.upstream()?.clone();
    let head = ctx.git().ref_value(ctx.repo(), &upstream.branch).ok()??;
    let record = |result: PushResult, detail: Option<String>, commits: usize| LastPush {
        at: timestamp(SystemTime::now()),
        head: head.clone(),
        result,
        detail,
        commits,
    };
    let waiting = || engine::to_push(ctx).unwrap_or(0);
    let remote = match ctx.take_remote() {
        Ok(Some(remote)) => remote,
        Ok(None) => return None,
        Err(left) => return Some(record(PushResult::Failed, left.detail, waiting())),
    };
    match engine::fetch_upstream(ctx, &upstream, &remote) {
        Err(left) => return Some(record(result_of(&left), left.detail, waiting())),
        Ok(Some(theirs)) => match ctx.git().is_ancestor(ctx.repo(), &theirs, &head) {
            Ok(true) => {}
            Ok(false) => return Some(needs_pull(record, waiting())),
            Err(e) => return Some(record(PushResult::Failed, Some(e.to_string()), waiting())),
        },
        Ok(None) => {}
    }
    let commits = waiting();
    if commits == 0 {
        return None;
    }
    Some(match engine::push_claimed(ctx, &remote) {
        Pushed::Pushed => record(PushResult::Pushed, None, commits),
        Pushed::RemoteMoved => needs_pull(record, waiting()),
        Pushed::Left(left) => record(result_of(&left), left.detail.clone(), waiting()),
    })
}

fn needs_pull(
    record: impl Fn(PushResult, Option<String>, usize) -> LastPush,
    commits: usize,
) -> LastPush {
    record(
        PushResult::NeedsPull,
        Some(
            "the remote has commits this brain does not; nothing was pushed: a read or kluris sync brings them in"
                .into(),
        ),
        commits,
    )
}

/// How a push or fetch that left the brain as it is is recorded.
fn result_of(left: &LeftAsIs) -> PushResult {
    match left.reason {
        LeftReason::Offline => PushResult::Offline,
        LeftReason::Refused | LeftReason::Hook => PushResult::Refused,
        _ => PushResult::Failed,
    }
}

/// The lock, held by this process until dropped.
struct Held {
    path: PathBuf,
}

impl Held {
    /// The lock at `path`, made whole with this process's pid, host and
    /// start, never over a live one; `None` while a live run holds it.
    fn take(path: &Path) -> io::Result<Option<Held>> {
        let lock = Lock {
            pid: std::process::id(),
            host: this_host(),
            started: timestamp(SystemTime::now()),
        };
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        // Written beside it, then linked in: the lock is never seen half
        // written, and never made over a lock that is there.
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{}", lock.pid));
        let draft = PathBuf::from(name);
        fs::write(&draft, lock.text())?;
        let taken = Held::link(&draft, path);
        let _ = fs::remove_file(&draft);
        taken
    }

    fn link(draft: &Path, path: &Path) -> io::Result<Option<Held>> {
        for _ in 0..2 {
            match fs::hard_link(draft, path) {
                Ok(()) => {
                    return Ok(Some(Held {
                        path: path.to_path_buf(),
                    }));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let there = fs::read(path).unwrap_or_default();
            if Lock::read(path).is_some_and(|lock| lock.live(SystemTime::now())) {
                return Ok(None);
            }
            // Stale: removed only while it still holds what was read.
            if fs::read(path).is_ok_and(|now| now == there) {
                let _ = fs::remove_file(path);
            }
        }
        Ok(None)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Only this run's own: never one that replaced it as stale.
        if Lock::read(&self.path).is_some_and(|lock| lock.pid == std::process::id()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Whether the process `pid` is there on this host.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(e) => e == rustix::io::Errno::PERM,
    }
}

/// Whether the process `pid` is there on this host.
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, FALSE, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: a handle opened here and closed here; the exit code goes to
    // a local.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if process.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let asked = GetExitCodeProcess(process, &raw mut code);
        CloseHandle(process);
        asked != 0 && code == STILL_ACTIVE as u32
    }
}

/// The child's stderr sent to the log.
mod log {
    use std::fs;
    use std::path::Path;

    /// The log, this process's stderr while it is held.
    pub struct Log {
        #[cfg(windows)]
        file: fs::File,
    }

    impl Log {
        /// Replace the log at `path` and send this process's stderr there;
        /// `None` when it cannot be made.
        pub fn open(path: &Path) -> Option<Log> {
            let file = fs::File::create(path).ok()?;
            #[cfg(unix)]
            {
                rustix::stdio::dup2_stderr(&file).ok()?;
                Some(Log {})
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
                // SAFETY: the handle stays open while this log is held, and
                // is taken back as stderr before it closes.
                let set = unsafe { SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle()) };
                (set != 0).then_some(Log { file })
            }
        }
    }

    #[cfg(windows)]
    impl Drop for Log {
        fn drop(&mut self) {
            use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
            let _ = &self.file;
            // SAFETY: stderr goes nowhere before the file closes.
            unsafe {
                SetStdHandle(STD_ERROR_HANDLE, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(unix)]
mod detach {
    use std::ffi::{OsString, c_int};
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::Start;

    const F_SETFD: c_int = 2;
    const FD_CLOEXEC: c_int = 1;

    /// The most descriptors the loop marks, however high the limit.
    const MOST: c_int = 1 << 20;

    unsafe extern "C" {
        fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        fn getdtablesize() -> c_int;
    }

    #[cfg(target_os = "linux")]
    mod range {
        use std::ffi::c_long;

        const SYS_CLOSE_RANGE: c_long = 436;
        const CLOSE_RANGE_CLOEXEC: c_long = 4;

        unsafe extern "C" {
            fn syscall(number: c_long, ...) -> c_long;
        }

        /// Every descriptor from 3 up set to close at `exec` in one call
        /// (`close_range`, Linux 5.11 and newer); false where it is
        /// missing.
        pub fn close_on_exec_from_3() -> bool {
            // SAFETY: a system call with plain integer arguments; it
            // touches only this process's descriptor flags.
            unsafe {
                syscall(
                    SYS_CLOSE_RANGE,
                    3 as c_long,
                    c_long::from(u32::MAX),
                    CLOSE_RANGE_CLOEXEC,
                ) == 0
            }
        }
    }

    /// Every descriptor from 3 below `limit` set to close at `exec`, as
    /// the child runs it between fork and exec: only async-signal-safe
    /// calls, nothing allocated. std's own pipe for an exec error is
    /// already close-on-exec, so it still reports one.
    fn close_on_exec_from_3(limit: c_int) {
        #[cfg(target_os = "linux")]
        if range::close_on_exec_from_3() {
            return;
        }
        for fd in 3..limit {
            // SAFETY: `fcntl` on a number that may name no descriptor,
            // which only fails (EBADF).
            unsafe {
                fcntl(fd, F_SETFD, FD_CLOEXEC);
            }
        }
    }

    /// `program` with `args` in `dir`, in a session of its own, with null
    /// standard streams and nothing else of this process's open in it.
    pub fn spawn(program: &Path, args: &[OsString], dir: &Path) -> Start {
        // SAFETY: no preconditions; read here, before the fork.
        let limit = unsafe { getdtablesize() }.clamp(3, MOST);
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: the closure runs between fork and exec and calls only
        // `setsid`, `syscall` and `fcntl`, which are async-signal-safe; it
        // allocates nothing.
        unsafe {
            cmd.pre_exec(move || {
                rustix::process::setsid().map_err(io::Error::from)?;
                close_on_exec_from_3(limit);
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(child) => Start::Started(child.id()),
            Err(e) => Start::Failed(e),
        }
    }
}

#[cfg(windows)]
mod detach {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::{
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CreateProcessW, DETACHED_PROCESS,
        PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
    };

    use super::Start;

    /// `ERROR_ACCESS_DENIED`: the job forbids breakaway.
    const DENIED: i32 = 5;

    /// Whether this process's job ends its processes when it closes;
    /// `None` when it cannot be read.
    fn job_kills_on_close() -> Option<bool> {
        // SAFETY: a query of this process's own job into a local of its
        // size.
        unsafe {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            let size = u32::try_from(std::mem::size_of_val(&info)).ok()?;
            let read = QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectExtendedLimitInformation,
                (&raw mut info).cast(),
                size,
                std::ptr::null_mut(),
            );
            (read != 0).then_some(
                info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0,
            )
        }
    }

    /// `path` as a NUL-terminated wide string.
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    /// `program` with `args` in `dir`, started with `flags` and no handle
    /// inherited (no standard handles either): its process id.
    fn create(
        program: &Path,
        args: &[OsString],
        dir: &Path,
        flags: PROCESS_CREATION_FLAGS,
    ) -> io::Result<u32> {
        let application = wide(program);
        let mut line = super::command_line(program.as_os_str(), args);
        let dir = wide(dir);
        // SAFETY: every string is NUL-terminated and outlives the call, the
        // command line is a buffer Windows may write into, and the two
        // structures are locals of their size; the handles it gives back
        // are closed here.
        unsafe {
            let mut startup: STARTUPINFOW = std::mem::zeroed();
            startup.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
            let mut info: PROCESS_INFORMATION = std::mem::zeroed();
            let made = CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                FALSE,
                flags,
                std::ptr::null(),
                dir.as_ptr(),
                &raw const startup,
                &raw mut info,
            );
            if made == 0 {
                return Err(io::Error::last_os_error());
            }
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
            Ok(info.dwProcessId)
        }
    }

    /// `program` with `args` in `dir`: detached, in a group of its own,
    /// out of this process's job when the job lets it leave; in it when the
    /// job keeps it but does not end it with this process.
    pub fn spawn(program: &Path, args: &[OsString], dir: &Path) -> Start {
        let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        match create(program, args, dir, detached | CREATE_BREAKAWAY_FROM_JOB) {
            Ok(pid) => Start::Started(pid),
            Err(e) if e.raw_os_error() == Some(DENIED) => match job_kills_on_close() {
                Some(false) => match create(program, args, dir, detached) {
                    Ok(pid) => Start::Started(pid),
                    Err(e) => Start::Failed(e),
                },
                Some(true) => Start::Refused(
                    "this process's job ends its processes with it and forbids leaving it".into(),
                ),
                None => Start::Refused(
                    "this process's job forbids leaving it and could not be read".into(),
                ),
            },
            Err(e) => Start::Failed(e),
        }
    }
}

/// The Windows command line of `program` with `args`, NUL-terminated: the
/// program quoted (a Windows path holds no quote), each argument as the C
/// runtime and `CommandLineToArgvW` read it back ([`quote`]).
#[cfg(windows)]
pub fn command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut line = vec![u16::from(b'"')];
    line.extend(program.encode_wide());
    line.push(u16::from(b'"'));
    for arg in args {
        line.push(u16::from(b' '));
        quote(&arg.encode_wide().collect::<Vec<u16>>(), &mut line);
    }
    line.push(0);
    line
}

/// `arg` appended to a Windows command line: as it is when it has no
/// space, tab, line break or quote and is not empty; else in quotes, each
/// quote escaped with a backslash, and the backslashes before a quote or
/// the closing quote doubled.
#[cfg(any(windows, test))]
fn quote(arg: &[u16], line: &mut Vec<u16>) {
    let [space, tab, newline, vtab, quote, backslash] =
        [b' ', b'\t', b'\n', 0x0b, b'"', b'\\'].map(u16::from);
    let plain = !arg.is_empty()
        && !arg
            .iter()
            .any(|c| [space, tab, newline, vtab, quote].contains(c));
    if plain {
        line.extend_from_slice(arg);
        return;
    }
    line.push(quote);
    let mut backslashes = 0;
    for &c in arg {
        if c == backslash {
            backslashes += 1;
        } else {
            if c == quote {
                line.extend(std::iter::repeat_n(backslash, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(c);
    }
    line.extend(std::iter::repeat_n(backslash, backslashes));
    line.push(quote);
}

#[cfg(test)]
mod tests {
    use super::quote;

    /// `line` split into arguments by the C runtime's rules for every
    /// argument after the program's: a backslash run before a quote is
    /// halved (an odd one escapes the quote), quotes open and close, and
    /// spaces and tabs outside quotes split.
    fn split(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        let (mut args, mut arg, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let run = chars[i..].iter().take_while(|&&c| c == '\\').count();
                if chars.get(i + run) == Some(&'"') {
                    arg.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        arg.push('"');
                        i += run + 1;
                    } else {
                        i += run;
                    }
                } else {
                    arg.extend(std::iter::repeat_n('\\', run));
                    i += run;
                }
                any = true;
                continue;
            }
            if c == '"' {
                quoted = !quoted;
                any = true;
            } else if (c == ' ' || c == '\t') && !quoted {
                if any {
                    args.push(std::mem::take(&mut arg));
                    any = false;
                }
            } else {
                arg.push(c);
                any = true;
            }
            i += 1;
        }
        if any {
            args.push(arg);
        }
        args
    }

    fn quoted(arg: &str) -> String {
        let mut line = Vec::new();
        quote(&arg.encode_utf16().collect::<Vec<u16>>(), &mut line);
        String::from_utf16(&line).unwrap()
    }

    #[test]
    fn each_argument_is_quoted_as_windows_reads_it_back() {
        for (arg, expected) in [
            ("sync", "sync"),
            ("", r#""""#),
            (r"C:\brains\alpha", r"C:\brains\alpha"),
            (r"C:\my brains\alpha", r#""C:\my brains\alpha""#),
            (r#"a "quoted" brain"#, r#""a \"quoted\" brain""#),
            (r#"back\"slash"#, r#""back\\\"slash""#),
            (r"trailing space\", r#""trailing space\\""#),
            ("tab\there", "\"tab\there\""),
        ] {
            assert_eq!(quoted(arg), expected, "{arg:?}");
        }
        let args = [
            "sync",
            "--path",
            r"C:\my brains\alpha",
            r#"C:\brains\a "quoted" brain\"#,
            r"C:\trailing\\",
            r#"\\"#,
            r#"x\\\"y"#,
            "",
            "plain",
        ];
        let line = args.iter().map(|a| quoted(a)).collect::<Vec<_>>().join(" ");
        assert_eq!(split(&line), args, "{line}");
    }
}
