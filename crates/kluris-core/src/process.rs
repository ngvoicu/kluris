//! Running a program with a deadline: past it the program is stopped with
//! every process it started (no orphan), and the caller goes on.
//!
//! The program runs in a process group of its own on Unix and in a job
//! object on Windows, so a timeout ends the whole tree (git's
//! `git-remote-https`, `ssh` and `upload-pack` included) and a normal exit
//! ends whatever the program left running. On Unix a timeout sends the
//! group TERM first and KILL once the program has exited or half a second
//! has passed, so git removes the lock files it holds; on Windows the job
//! is terminated at once.
//!
//! On Unix a SIGINT, SIGTERM or SIGHUP to the command ends every group
//! still running before the command dies of it ([`interrupts`]), so nothing
//! a run started outlives the claims the command holds. The threads the
//! runner makes (its output drains, its input writer) never take those
//! signals, so the handler runs on a thread whose progress the command
//! waits for; and once a signal came, no run returns to the command: each
//! waits for the handler, which ends the command within its own bounds:
//! up to [`COMMAND_BOUND`] for a run still starting its program to make its
//! group known (a Ctrl-C otherwise ends the command at once), then the
//! grace. A run whose start takes longer than that is not waited for, and
//! its program can outlive the command: a known limit. A SIGKILL to the
//! command cannot be handled: its groups are left running to their own end
//! or the next machine restart.
//!
//! [`COMMAND_BOUND`]: crate::sync::claim::COMMAND_BOUND
//!
//! On Windows the program starts suspended, joins its job, and only then
//! runs, so nothing it starts is outside the job. When the job cannot be
//! made or the program cannot join it, the program still runs, uncontained:
//! its deadline then ends the program alone, never what it started, and
//! the run is not failed for it.

use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// How often the program is polled.
const POLL: Duration = Duration::from_millis(5);

/// A program that ran to its end within the deadline.
#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Why [`run_until`] has no [`Finished`].
#[derive(Debug)]
pub enum RunError {
    /// The program could not be started.
    Spawn(io::Error),
    /// The deadline passed; the program and its children were stopped.
    TimedOut,
}

/// Run `cmd` with stdin closed and its output captured, stopping it and its
/// children once `deadline` passes (see the module documentation). Whatever
/// the program leaves running when it exits is killed.
pub fn run_until(cmd: Command, deadline: Instant) -> Result<Finished, RunError> {
    run(cmd, None, true, deadline)
}

/// [`run_until`], with `input` on the program's stdin.
pub fn run_until_fed(
    cmd: Command,
    input: Vec<u8>,
    deadline: Instant,
) -> Result<Finished, RunError> {
    run(cmd, Some(input), true, deadline)
}

/// [`run_until`], with the program's output going where `cmd` sends it
/// (its streams as they are set on `cmd`): how it exited.
pub fn run_until_shown(cmd: Command, deadline: Instant) -> Result<ExitStatus, RunError> {
    run(cmd, None, false, deadline).map(|done| done.status)
}

/// Run `cmd`, `input` on its stdin (closed without), its output captured
/// when `capture` (else where `cmd` sends it, and none comes back).
fn run(
    cmd: Command,
    input: Option<Vec<u8>>,
    capture: bool,
    deadline: Instant,
) -> Result<Finished, RunError> {
    let ran = run_to_its_end(cmd, input, capture, deadline);
    hold_if_interrupted();
    ran
}

/// Once the command is being interrupted, a run never returns to it: it
/// waits for the interrupt handler to end the command ([`interrupts`]).
fn hold_if_interrupted() {
    #[cfg(unix)]
    interrupts::hold_if_interrupted();
}

/// A thread of the runner's own for `work`, which never takes the signals
/// the interrupt handler handles: it starts with them blocked, as a new
/// thread takes the mask of the thread that makes it.
fn helper(work: impl FnOnce() + Send + 'static) {
    #[cfg(unix)]
    let _blocked = interrupts::Blocked::now();
    thread::spawn(work);
}

/// [`run`], up to the program's end or the deadline.
fn run_to_its_end(
    mut cmd: Command,
    input: Option<Vec<u8>>,
    capture: bool,
    deadline: Instant,
) -> Result<Finished, RunError> {
    // A program whose time is already up is never started.
    if Instant::now() >= deadline {
        return Err(RunError::TimedOut);
    }
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    if capture {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let (mut child, tree) = tree::Tree::start(&mut cmd).map_err(RunError::Spawn)?;
    // Written from a thread: the program may fill its output before it has
    // read all of its input. A program stopped first ends the write.
    if let (Some(mut stdin), Some(input)) = (child.stdin.take(), input) {
        helper(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut status = None;
    let mut out = None;
    let mut err = None;
    loop {
        hold_if_interrupted();
        if status.is_none() {
            status = child.try_wait().ok().flatten();
            if status.is_some() {
                // What the program left running would keep its pipes open.
                tree.end();
            }
        }
        if out.is_none() {
            out = stdout.try_recv().ok();
        }
        if err.is_none() {
            err = stderr.try_recv().ok();
        }
        if let (Some(status), Some(out), Some(err)) = (status, &mut out, &mut err) {
            return Ok(Finished {
                status,
                stdout: std::mem::take(out),
                stderr: std::mem::take(err),
            });
        }
        if Instant::now() >= deadline {
            if status.is_none() {
                // A program asked to stop first cleans up (git removes its
                // lock files on TERM); then whatever is left is killed.
                tree.interrupt();
                let grace = Instant::now() + tree::GRACE;
                while child.try_wait().ok().flatten().is_none() && Instant::now() < grace {
                    thread::sleep(POLL);
                }
            }
            tree.end();
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(RunError::TimedOut);
        }
        thread::sleep(POLL);
    }
}

/// A pipe read to its end on a thread of its own; the bytes arrive on the
/// receiver (empty when there is no pipe or it fails). A thread whose pipe a
/// stray process keeps open is left behind with it, never waited for.
fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    match pipe {
        Some(mut pipe) => {
            helper(move || {
                let mut bytes = Vec::new();
                let _ = pipe.read_to_end(&mut bytes);
                let _ = tx.send(bytes);
            });
        }
        None => {
            let _ = tx.send(Vec::new());
        }
    }
    rx
}

#[cfg(unix)]
mod tree {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::atomic::AtomicI32;
    use std::time::Duration;

    use rustix::process::{Pid, Signal, kill_process_group};

    use super::interrupts;

    /// How long a stopped program has to exit after TERM before KILL.
    pub const GRACE: Duration = Duration::from_millis(500);

    /// The process group of a started program, known to the interrupt
    /// handler ([`interrupts`]) until the run ends.
    pub struct Tree {
        group: Option<Pid>,
        registered: Option<&'static AtomicI32>,
    }

    impl Tree {
        /// Start `cmd` leading a new process group, with the group known to
        /// the interrupt handler before any signal it handles can be taken.
        /// No program starts once the command is being interrupted: the
        /// run waits for the handler to end the command.
        pub fn start(cmd: &mut Command) -> io::Result<(Child, Tree)> {
            cmd.process_group(0);
            let starting = interrupts::Starting::begin();
            let mask = starting.mask_before();
            // SAFETY: `sigprocmask` is async-signal-safe, and `mask` a copy.
            unsafe {
                cmd.pre_exec(move || {
                    // The program gets the mask this thread had before the
                    // handled signals were blocked to start it.
                    libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
                    Ok(())
                });
            }
            let child = cmd.spawn()?;
            let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
            let registered =
                group.and_then(|group| interrupts::register(group.as_raw_nonzero().get()));
            drop(starting);
            Ok((child, Tree { group, registered }))
        }

        /// Ask every process in the group to stop (TERM).
        pub fn interrupt(&self) {
            if let Some(group) = self.group {
                let _ = kill_process_group(group, Signal::TERM);
            }
        }

        /// Kill every process left in the group. The group's id cannot be
        /// reused while one of its processes lives, and an empty group is
        /// gone (ESRCH, ignored).
        pub fn end(&self) {
            if let Some(group) = self.group {
                let _ = kill_process_group(group, Signal::KILL);
            }
        }
    }

    impl Drop for Tree {
        /// The run is over and its program reaped: the group is forgotten.
        fn drop(&mut self) {
            if let Some(registered) = self.registered.take() {
                interrupts::forget(registered);
            }
        }
    }
}

/// What a SIGINT, SIGTERM or SIGHUP does to a command that runs programs
/// here: the groups they lead (see the module documentation) are outside
/// the command's own, so the terminal's Ctrl-C never reaches them, and a
/// command that died of the signal would leave them running past its
/// claims. The handler, installed on the first run (a signal the command
/// ignores stays ignored), ends every live group first: TERM, then KILL
/// once [`tree::GRACE`] has passed (none for a group gone sooner). Then the
/// command ends as the signal ends it by default, with the status the shell
/// expects. It calls only `kill`, `killpg`, `nanosleep`, `sigaction` and
/// `raise`, and reads atomics.
///
/// A prompt's own handler (the CLI's `AbortOnInterrupt`, through
/// signal-hook) is installed for the prompt's time on top of this one and
/// calls it first: with no run going it ends nothing, and the prompt still
/// prints `Aborted!` and exits 1 before the default action comes.
#[cfg(unix)]
mod interrupts {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering::SeqCst};

    use libc::c_int;

    use super::{POLL, tree::GRACE};

    /// The signals that end a command and are handled here.
    const SIGNALS: [c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    /// The groups of the runs going, one per slot (0: free). More runs at
    /// once than slots go unknown to the handler.
    static GROUPS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

    /// How many runs are between starting their program and registering
    /// its group.
    static STARTING: AtomicUsize = AtomicUsize::new(0);

    /// A handled signal arrived: no program starts any more.
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    /// How long the handler waits for a run starting its program to
    /// register its group: a command's bound, as a start held that long is
    /// stuck, not slow.
    const STARTING_BOUND: std::time::Duration = crate::sync::claim::COMMAND_BOUND;

    /// The handled signals blocked on this thread until dropped, when the
    /// mask it had comes back.
    pub struct Blocked(libc::sigset_t);

    impl Blocked {
        pub fn now() -> Blocked {
            // SAFETY: plain libc calls on locals of the stated types.
            unsafe {
                let mut block: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut block);
                for signal in SIGNALS {
                    libc::sigaddset(&mut block, signal);
                }
                let mut before: libc::sigset_t = std::mem::zeroed();
                libc::pthread_sigmask(libc::SIG_BLOCK, &block, &mut before);
                Blocked(before)
            }
        }
    }

    impl Drop for Blocked {
        fn drop(&mut self) {
            // SAFETY: the mask this thread had before `now`.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &self.0, std::ptr::null_mut());
            }
        }
    }

    /// A run starting its program: until dropped, the handled signals are
    /// blocked on this thread (the handler never waits for its own thread),
    /// and the handler waits for the run to register its group.
    pub struct Starting(Blocked);

    impl Starting {
        /// The start of a run; once the command is being interrupted, the
        /// wait for the handler to end it instead.
        pub fn begin() -> Starting {
            install();
            let blocked = Blocked::now();
            STARTING.fetch_add(1, SeqCst);
            if INTERRUPTED.load(SeqCst) {
                STARTING.fetch_sub(1, SeqCst);
                wait_for_the_end();
            }
            Starting(blocked)
        }

        /// This thread's signal mask before [`Starting::begin`].
        pub fn mask_before(&self) -> libc::sigset_t {
            self.0.0
        }
    }

    impl Drop for Starting {
        fn drop(&mut self) {
            STARTING.fetch_sub(1, SeqCst);
        }
    }

    /// Once a handled signal came, wait for the handler to end the
    /// command.
    pub fn hold_if_interrupted() {
        if INTERRUPTED.load(SeqCst) {
            wait_for_the_end();
        }
    }

    /// The handler always ends the command within its bounds
    /// ([`STARTING_BOUND`], then the grace): this thread waits for it.
    fn wait_for_the_end() -> ! {
        loop {
            std::thread::sleep(POLL);
        }
    }

    /// Make `group` known to the handler; its slot, or `None` when every
    /// slot is taken.
    pub fn register(group: i32) -> Option<&'static AtomicI32> {
        GROUPS
            .iter()
            .find(|slot| slot.compare_exchange(0, group, SeqCst, SeqCst).is_ok())
    }

    /// The group in `slot` is over.
    pub fn forget(slot: &AtomicI32) {
        slot.store(0, SeqCst);
    }

    /// The handler for [`SIGNALS`], once per process.
    fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            for signal in SIGNALS {
                // SAFETY: `on_signal` is an `extern "C" fn(c_int)`, and
                // every pointer is to a local of the stated type.
                unsafe {
                    let mut ours: libc::sigaction = std::mem::zeroed();
                    ours.sa_sigaction = on_signal as extern "C" fn(c_int) as libc::sighandler_t;
                    libc::sigemptyset(&mut ours.sa_mask);
                    for blocked in SIGNALS {
                        libc::sigaddset(&mut ours.sa_mask, blocked);
                    }
                    ours.sa_flags = libc::SA_RESTART;
                    let mut old: libc::sigaction = std::mem::zeroed();
                    if libc::sigaction(signal, &ours, &mut old) == 0
                        && old.sa_sigaction == libc::SIG_IGN
                    {
                        libc::sigaction(signal, &old, std::ptr::null_mut());
                    }
                }
            }
        });
    }

    /// Sleep for [`POLL`], in the handler.
    fn nap() {
        // SAFETY: a zeroed timespec (some targets pad it) given its
        // nanoseconds, and a null remainder.
        unsafe {
            let mut pause: libc::timespec = std::mem::zeroed();
            pause.tv_nsec = libc::c_long::try_from(POLL.as_nanos()).unwrap_or(5_000_000);
            libc::nanosleep(&pause, std::ptr::null_mut());
        }
    }

    /// Whether `group` (0: none) has a process left.
    fn alive(group: i32) -> bool {
        // SAFETY: a signal 0 sends nothing; it only asks.
        group > 0 && unsafe { libc::kill(-group, 0) } == 0
    }

    /// `signal` to every live group, each read once: a group is never 0
    /// here, which would be the command's own.
    fn send(signal: c_int) {
        for slot in &GROUPS {
            let group = slot.load(SeqCst);
            if alive(group) {
                // SAFETY: async-signal-safe; a gone group is ESRCH.
                unsafe {
                    libc::killpg(group, signal);
                }
            }
        }
    }

    /// End every live group, then end the command as `signal` does by
    /// default (see the module documentation).
    extern "C" fn on_signal(signal: c_int) {
        INTERRUPTED.store(true, SeqCst);
        let polls = |bound: std::time::Duration| bound.as_millis() / POLL.as_millis().max(1);
        let mut left = polls(STARTING_BOUND);
        while STARTING.load(SeqCst) > 0 && left > 0 {
            nap();
            left -= 1;
        }
        send(libc::SIGTERM);
        let mut left = polls(GRACE);
        while GROUPS.iter().any(|slot| alive(slot.load(SeqCst))) && left > 0 {
            nap();
            left -= 1;
        }
        send(libc::SIGKILL);
        // SAFETY: the default action back, then the signal again: it is
        // blocked while this handler runs, so it ends the command once the
        // handler returns.
        unsafe {
            let mut default: libc::sigaction = std::mem::zeroed();
            default.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut default.sa_mask);
            libc::sigaction(signal, &default, std::ptr::null_mut());
            libc::raise(signal);
        }
    }
}

#[cfg(windows)]
mod tree {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ProcessSnapshotting::{
        HPSS, HPSSWALK, PSS_CAPTURE_THREADS, PSS_THREAD_ENTRY, PSS_WALK_THREADS,
        PssCaptureSnapshot, PssFreeSnapshot, PssWalkMarkerCreate, PssWalkMarkerFree,
        PssWalkSnapshot,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, GetCurrentProcess, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    /// No grace: a job has no signal to ask its processes to stop with.
    pub const GRACE: Duration = Duration::ZERO;

    /// How many times a thread snapshot is taken before resuming fails.
    const SNAPSHOT_TRIES: usize = 5;

    #[cfg(test)]
    thread_local! {
        /// Makes the job fail on this thread: a job's failure has no other
        /// way to be made where nested jobs are allowed.
        pub static JOB_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// The job object holding a started program (and so everything it
    /// starts), killed when the job's handle closes; `None` when it could
    /// not be made or the program could not join it, and the program runs
    /// uncontained (see the module documentation).
    pub struct Tree(Option<HANDLE>);

    impl Tree {
        /// Start `cmd` suspended, put it in a new job, then let it run, so
        /// nothing it starts is ever outside the job. The flags `cmd` was
        /// created with are this one (`CREATE_SUSPENDED`).
        pub fn start(cmd: &mut Command) -> io::Result<(Child, Tree)> {
            cmd.creation_flags(CREATE_SUSPENDED);
            let mut child = cmd.spawn()?;
            let tree = Tree::containing(&child);
            if !resume(&child) {
                let error = io::Error::last_os_error();
                drop(tree);
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            Ok((child, tree))
        }

        /// Nothing: [`Tree::end`] terminates the job.
        pub fn interrupt(&self) {}

        /// Kill every process left in the job.
        pub fn end(&self) {
            if let Some(job) = self.0 {
                // SAFETY: `job` is this tree's open job handle.
                unsafe {
                    TerminateJobObject(job, 1);
                }
            }
        }
    }

    /// A job that kills its processes when closed, with `child` in it.
    impl Tree {
        /// A job holding `child`, owned: closed (and what is left in it
        /// killed) when the tree goes.
        pub fn containing(child: &Child) -> Tree {
            Tree(job_for(child))
        }
    }

    fn job_for(child: &Child) -> Option<HANDLE> {
        #[cfg(test)]
        if JOB_FAILS.with(std::cell::Cell::get) {
            return None;
        }
        // SAFETY: plain Win32 calls on a handle created here and the child's
        // process handle, which `child` keeps open; every pointer is to a
        // live local of the stated size.
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
            Some(job)
        }
    }

    /// Let the suspended `child` run: its one thread resumed, found in a
    /// snapshot of the child alone, else in one of every thread of the
    /// system, which costs tens of milliseconds where many threads run.
    /// Whether it was.
    fn resume(child: &Child) -> bool {
        resume_by_process_snapshot(child.as_raw_handle() as HANDLE)
            || resume_by_thread_snapshot(child.id())
    }

    /// Resume the thread with the id `thread`; whether it was.
    fn resume_thread(thread: u32) -> bool {
        // SAFETY: a handle opened and closed here.
        unsafe {
            let handle = OpenThread(THREAD_SUSPEND_RESUME, 0, thread);
            if handle.is_null() {
                return false;
            }
            let resumed = ResumeThread(handle) != u32::MAX;
            CloseHandle(handle);
            resumed
        }
    }

    /// Resume the first thread of `process`, found in a snapshot of that
    /// process alone (`PssCaptureSnapshot`); whether it was.
    pub fn resume_by_process_snapshot(process: HANDLE) -> bool {
        // SAFETY: plain Win32 calls on a snapshot and a walk marker made
        // and freed here; `entry` is a live local whose size is given.
        unsafe {
            let mut snapshot: HPSS = std::ptr::null_mut();
            if PssCaptureSnapshot(process, PSS_CAPTURE_THREADS, 0, &mut snapshot) != 0 {
                return false;
            }
            let mut resumed = false;
            let mut walk: HPSSWALK = std::ptr::null_mut();
            if PssWalkMarkerCreate(std::ptr::null(), &mut walk) == 0 {
                let mut entry: PSS_THREAD_ENTRY = std::mem::zeroed();
                let size = u32::try_from(std::mem::size_of::<PSS_THREAD_ENTRY>()).unwrap_or(0);
                if PssWalkSnapshot(
                    snapshot,
                    PSS_WALK_THREADS,
                    walk,
                    (&raw mut entry).cast(),
                    size,
                ) == 0
                {
                    resumed = resume_thread(entry.ThreadId);
                }
                PssWalkMarkerFree(walk);
            }
            PssFreeSnapshot(GetCurrentProcess(), snapshot);
            resumed
        }
    }

    /// Resume the thread of the process `pid`, found in a snapshot of every
    /// thread of the system; whether it was.
    pub fn resume_by_thread_snapshot(pid: u32) -> bool {
        // SAFETY: plain Win32 calls on handles opened and closed here;
        // `entry` is a live local whose size is given.
        unsafe {
            // A snapshot taken while threads come and go can fail at once
            // (ERROR_BAD_LENGTH): it is taken again, a few times at most.
            let mut snapshot = INVALID_HANDLE_VALUE;
            for _ in 0..SNAPSHOT_TRIES {
                snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                if snapshot != INVALID_HANDLE_VALUE {
                    break;
                }
            }
            if snapshot == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = u32::try_from(std::mem::size_of::<THREADENTRY32>()).unwrap_or(0);
            let mut resumed = false;
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    resumed = resume_thread(entry.th32ThreadID);
                    break;
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            resumed
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            if let Some(job) = self.0.take() {
                // SAFETY: closed once, here; kill-on-close ends what is left.
                unsafe {
                    CloseHandle(job);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[cfg(unix)]
    #[test]
    fn output_and_status_come_back() {
        let done = run_until(
            sh("echo out; echo err >&2; exit 3"),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(done.status.code(), Some(3));
        assert_eq!(done.stdout, b"out\n");
        assert_eq!(done.stderr, b"err\n");
    }

    #[cfg(unix)]
    #[test]
    fn input_reaches_the_program() {
        // More than a pipe holds: the program writes while it reads.
        let input = vec![b'x'; 1 << 20];
        let done = run_until_fed(
            sh("cat"),
            input.clone(),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        assert!(done.status.success());
        assert_eq!(done.stdout, input);
    }

    #[cfg(unix)]
    #[test]
    fn shown_output_goes_where_the_command_sends_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("out");
        let mut cmd = sh("echo out; echo err >&2; exit 4");
        cmd.stdout(std::fs::File::create(&file).unwrap())
            .stderr(Stdio::null());
        let status = run_until_shown(cmd, Instant::now() + Duration::from_secs(10)).unwrap();
        assert_eq!(status.code(), Some(4));
        assert_eq!(std::fs::read(&file).unwrap(), b"out\n");
    }

    #[cfg(unix)]
    #[test]
    fn shown_output_stops_at_the_deadline() {
        let started = Instant::now();
        let mut cmd = sh("sleep 30");
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let result = run_until_shown(cmd, started + Duration::from_millis(300));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// A child a test started, killed and reaped however the test ends.
    struct Owned(std::process::Child);

    impl Drop for Owned {
        fn drop(&mut self) {
            if let Ok(None) = self.0.try_wait() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    impl Owned {
        /// How the child ended, polled until `bound`; `None` past it.
        #[cfg(unix)]
        fn ended_by(&mut self, bound: Instant) -> Option<ExitStatus> {
            loop {
                if let Ok(Some(status)) = self.0.try_wait() {
                    return Some(status);
                }
                if Instant::now() > bound {
                    return None;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// `work` on a thread of its own, failing the test unless it ends
    /// within `bound`.
    fn within(bound: Duration, what: &str, work: fn()) {
        let (done, ended) = std::sync::mpsc::channel();
        thread::spawn(move || {
            work();
            let _ = done.send(());
        });
        assert!(
            ended.recv_timeout(bound).is_ok(),
            "{what}: not within {bound:?}"
        );
    }

    /// `cmd` starts with SIGINT, SIGTERM and SIGHUP at their default
    /// actions, however this test run was started (a background job of a
    /// shell starts with SIGINT ignored, and an ignored signal stays
    /// ignored across exec).
    #[cfg(unix)]
    fn with_default_signals(cmd: &mut Command) {
        use std::os::unix::process::CommandExt;
        // SAFETY: `signal` is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                Ok(())
            });
        }
    }

    /// The folder the `interrupted_child` writes its marker in.
    #[cfg(unix)]
    const INTERRUPTED_DIR: &str = "KLURIS_TEST_INTERRUPTED_DIR";

    /// How many commands [`a_sigint_during_a_run_ends_the_command_by_sigint`]
    /// interrupts.
    #[cfg(unix)]
    const INTERRUPTED_COMMANDS: usize = 20;

    /// A SIGINT to the command while a run goes ends the command by
    /// SIGINT, never with status 0, wherever the signal lands: the child
    /// runs its program on a thread that blocks the signal, so it lands on
    /// another one (one the runner made, or the test harness's).
    #[cfg(unix)]
    #[test]
    fn a_sigint_during_a_run_ends_the_command_by_sigint() {
        use rustix::process::{Pid, Signal, kill_process};
        use std::os::unix::process::ExitStatusExt;
        for command in 1..=INTERRUPTED_COMMANDS {
            let dir = tempfile::TempDir::new().unwrap();
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--ignored",
                "--exact",
                "process::tests::interrupted_child",
                "--quiet",
            ])
            .env(INTERRUPTED_DIR, dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            with_default_signals(&mut cmd);
            let mut child = Owned(cmd.spawn().unwrap());
            let bound = Instant::now() + Duration::from_secs(30);
            while !dir.path().join("running").exists() {
                assert!(
                    Instant::now() < bound && child.0.try_wait().ok().flatten().is_none(),
                    "command {command}: its run never started"
                );
                thread::sleep(Duration::from_millis(5));
            }
            let pid = Pid::from_raw(i32::try_from(child.0.id()).unwrap()).unwrap();
            kill_process(pid, Signal::INT).unwrap();
            let status = child
                .ended_by(bound)
                .unwrap_or_else(|| panic!("command {command}: it never ended"));
            assert_eq!(
                status.signal(),
                Some(Signal::INT.as_raw()),
                "command {command}: {status:?}"
            );
        }
    }

    /// The folder the `slow_start_child` holds its claim and writes its
    /// program's pid in.
    #[cfg(unix)]
    const SLOW_START_DIR: &str = "KLURIS_TEST_SLOW_START_DIR";

    /// How long the `slow_start_child`'s start takes past its fork.
    #[cfg(unix)]
    const SLOW_START: Duration = Duration::from_secs(3);

    /// A SIGINT while a run is still starting its program (its spawn held
    /// past the fork, the program already running): the command ends the
    /// program before it dies of SIGINT, so the program is gone before the
    /// tree claim the command held can be taken.
    #[cfg(unix)]
    #[test]
    fn a_sigint_during_a_slow_start_ends_the_program_before_the_command() {
        use rustix::process::{Pid, Signal, kill_process};
        use std::os::unix::process::ExitStatusExt;
        let dir = tempfile::TempDir::new().unwrap();
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--ignored",
            "--exact",
            "process::tests::slow_start_child",
            "--quiet",
        ])
        .env(SLOW_START_DIR, dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        with_default_signals(&mut cmd);
        let mut child = Owned(cmd.spawn().unwrap());
        let bound = Instant::now() + Duration::from_secs(60);
        let pid_file = dir.path().join("program.pid");
        let program = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .filter(|pid| pid.ends_with('\n'))
            {
                break pid.trim().to_string();
            }
            assert!(
                Instant::now() < bound && child.0.try_wait().ok().flatten().is_none(),
                "the program never started"
            );
            thread::sleep(Duration::from_millis(5));
        };
        let command = Pid::from_raw(i32::try_from(child.0.id()).unwrap()).unwrap();
        kill_process(command, Signal::INT).unwrap();
        let claimed = dir.path().join("claimed");
        while crate::sync::claim::TreeClaim::exclusive(&claimed, Duration::ZERO).is_err() {
            assert!(Instant::now() < bound, "the claim was never let go");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(gone(&program), "the program {program} outlived the command");
        let status = child.ended_by(bound).expect("the command ends");
        assert_eq!(status.signal(), Some(Signal::INT.as_raw()), "{status:?}");
    }

    /// Whether the process `pid` is gone: none runs under it, or it is a
    /// zombie, ended and waiting to be reaped.
    #[cfg(unix)]
    fn gone(pid: &str) -> bool {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        stat.trim().is_empty() || stat.trim_start().starts_with('Z')
    }

    /// The parent's side of every fork of the `slow_start_child`: held for
    /// [`SLOW_START`], while the child process already runs.
    #[cfg(unix)]
    extern "C" fn slow_start() {
        thread::sleep(SLOW_START);
    }

    /// This test binary run again as the command of
    /// [`a_sigint_during_a_slow_start_ends_the_program_before_the_command`]:
    /// it holds a tree claim, then starts, on a worker, a run whose program
    /// writes its pid and waits, its spawn held for [`SLOW_START`] past the
    /// fork. Without its variable it does nothing.
    #[cfg(unix)]
    #[test]
    #[ignore = "run only as the child of a_sigint_during_a_slow_start_ends_the_program_before_the_command"]
    fn slow_start_child() {
        let Some(dir) = std::env::var_os(SLOW_START_DIR) else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let claimed = dir.join("claimed");
        let _claim = crate::sync::claim::TreeClaim::exclusive(&claimed, Duration::ZERO).unwrap();
        // SAFETY: registers a handler that only sleeps.
        unsafe {
            assert_eq!(libc::pthread_atfork(None, Some(slow_start), None), 0);
        }
        let script = format!(
            "echo $$ > '{}'; exec sleep 30",
            dir.join("program.pid").display()
        );
        let run = thread::spawn(move || {
            let _ = run_until(sh(&script), Instant::now() + Duration::from_secs(20));
        });
        let _ = run.join();
        std::process::exit(0);
    }

    /// This test binary run again as the command of
    /// [`a_sigint_during_a_run_ends_the_command_by_sigint`]: on a thread
    /// that blocks SIGINT, a run whose program marks that it runs and then
    /// holds its output open; whatever the run gives back, the command then
    /// ends with status 0. Without its variable it does nothing.
    #[cfg(unix)]
    #[test]
    #[ignore = "run only as the child of a_sigint_during_a_run_ends_the_command_by_sigint"]
    fn interrupted_child() {
        let Some(dir) = std::env::var_os(INTERRUPTED_DIR) else {
            return;
        };
        let running = std::path::Path::new(&dir).join("running");
        // SAFETY: a libc call on locals of the stated type.
        unsafe {
            let mut block: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut block);
            libc::sigaddset(&mut block, libc::SIGINT);
            libc::pthread_sigmask(libc::SIG_BLOCK, &block, std::ptr::null_mut());
        }
        let script = format!("echo x > '{}'; exec sleep 30", running.display());
        let _ = run_until(sh(&script), Instant::now() + Duration::from_secs(10));
        std::process::exit(0);
    }

    #[cfg(unix)]
    #[test]
    fn past_the_deadline_the_whole_tree_is_killed() {
        let dir = tempfile::TempDir::new().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        let started = Instant::now();
        let result = run_until(sh(&script), started + Duration::from_millis(500));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let alive = || {
            Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        let until = Instant::now() + Duration::from_secs(2);
        while alive() && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(), "the grandchild {pid} outlived the deadline");
    }

    #[cfg(unix)]
    #[test]
    fn past_the_deadline_the_tree_gets_term_first() {
        // git removes its lock files on TERM: the program's TERM handler
        // runs before anything is killed.
        let dir = tempfile::TempDir::new().unwrap();
        let cleaned = dir.path().join("cleaned");
        let script = format!(
            "trap 'echo x > \"{}\"; exit 143' TERM; sleep 30 & wait",
            cleaned.display()
        );
        let started = Instant::now();
        let result = run_until(sh(&script), started + Duration::from_millis(300));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        assert!(cleaned.exists(), "the program was killed without a TERM");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn a_tree_that_ignores_term_is_killed_half_a_second_later() {
        let dir = tempfile::TempDir::new().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!(
            "trap '' TERM; sleep 30 & echo $! > '{}'; wait",
            pid_file.display()
        );
        let started = Instant::now();
        let result = run_until(sh(&script), started + Duration::from_millis(300));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(750), "no grace: {took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let alive = || {
            Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        let until = Instant::now() + Duration::from_secs(2);
        while alive() && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(), "the child {pid} outlived the kill");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_left_holding_the_pipe_does_not_hold_the_caller() {
        // The program exits at once; its background child keeps stdout open.
        let started = Instant::now();
        let result = run_until(
            sh("sleep 30 & echo started"),
            started + Duration::from_secs(10),
        );
        assert!(started.elapsed() < Duration::from_secs(5), "{result:?}");
    }

    #[cfg(windows)]
    #[test]
    fn past_the_deadline_the_whole_job_is_killed() {
        // outer.bat starts inner.bat in the background and waits; inner.bat
        // marks that it runs, then, 5 s later, that it survived.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("inner.bat"),
            "@echo off\r\necho x> \"%~dp0started\"\r\nping -n 6 127.0.0.1 >nul\r\necho y> \"%~dp0survived\"\r\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("outer.bat"),
            "@echo off\r\nstart \"\" /B cmd /C \"%~dp0inner.bat\"\r\nping -n 30 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(dir.path().join("outer.bat"));
        let started = Instant::now();
        let result = run_until(cmd, started + Duration::from_secs(2));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(4));
        thread::sleep(Duration::from_secs(7));
        assert!(
            dir.path().join("started").exists(),
            "inner.bat never ran: the test proves nothing"
        );
        assert!(
            !dir.path().join("survived").exists(),
            "the grandchild outlived the deadline"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_grandchild_started_at_once_is_ended_with_the_program() {
        // The program starts a child at once, which runs a grandchild
        // (ping) holding held.txt open: the file can be removed only once
        // neither runs.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("child.bat"),
            "@echo off\r\nping -n 60 127.0.0.1 >\"%~dp0held.txt\"\r\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("outer.bat"),
            "@echo off\r\nstart \"\" /B cmd /C \"%~dp0child.bat\"\r\nping -n 60 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(dir.path().join("outer.bat"));
        let started = Instant::now();
        let result = run_until(cmd, started + Duration::from_secs(2));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        let held = dir.path().join("held.txt");
        assert!(
            held.exists(),
            "the grandchild never ran: the test proves nothing"
        );
        let until = Instant::now() + Duration::from_secs(10);
        while std::fs::remove_file(&held).is_err() {
            assert!(
                Instant::now() < until,
                "the grandchild outlived the deadline"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_program_whose_job_fails_still_runs_and_ends_at_the_deadline() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("run.bat"),
            "@echo off\r\necho x> \"%~dp0ran\"\r\nping -n 10 127.0.0.1 >nul\r\n",
        )
        .unwrap();
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(dir.path().join("run.bat"));
        tree::JOB_FAILS.with(|fails| fails.set(true));
        let started = Instant::now();
        let result = run_until(cmd, started + Duration::from_secs(2));
        tree::JOB_FAILS.with(|fails| fails.set(false));
        assert!(matches!(result, Err(RunError::TimedOut)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(dir.path().join("ran").exists(), "it never ran");
    }

    /// What one bounded run's start costs, by each way to start it: 50
    /// `git --version` each, printed. A measurement, not a check.
    #[test]
    #[ignore = "a measurement: run it with --ignored --nocapture"]
    fn the_cost_of_a_runs_start() {
        within(
            Duration::from_secs(300),
            "the measurement",
            measure_a_runs_start,
        );
    }

    /// The measurement of [`the_cost_of_a_runs_start`].
    fn measure_a_runs_start() {
        const RUNS: u32 = 50;
        let git = || {
            let mut cmd = Command::new("git");
            cmd.arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd
        };
        let far = || Instant::now() + Duration::from_secs(60);
        let time = |what: &str, run: &mut dyn FnMut()| {
            run();
            let started = Instant::now();
            for _ in 0..RUNS {
                run();
            }
            println!("start cost: {what}: {:?} a run", started.elapsed() / RUNS);
        };
        time("run_until", &mut || {
            run_until(git(), far()).unwrap();
        });
        time("the runner's start, then wait", &mut || {
            let (child, _tree) = tree::Tree::start(&mut git()).unwrap();
            Owned(child).0.wait().unwrap();
        });
        #[cfg(windows)]
        time("spawn, then the job, then wait", &mut || {
            let mut child = Owned(git().spawn().unwrap());
            let _job = tree::Tree::containing(&child.0);
            child.0.wait().unwrap();
        });
        #[cfg(windows)]
        time("a snapshot of every thread, walked", &mut || {
            assert!(!tree::resume_by_thread_snapshot(u32::MAX));
        });
        #[cfg(windows)]
        time(
            "spawn, then a snapshot of its threads, then wait",
            &mut || {
                use std::os::windows::io::AsRawHandle;
                let mut child = Owned(git().spawn().unwrap());
                let _ = tree::resume_by_process_snapshot(child.0.as_raw_handle() as _);
                child.0.wait().unwrap();
            },
        );
        time("spawn, then wait", &mut || {
            Owned(git().spawn().unwrap()).0.wait().unwrap();
        });
        time("output()", &mut || {
            git().output().unwrap();
        });
        time("the poll's sleep", &mut || thread::sleep(POLL));
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let result = run_until(
            Command::new("kluris-no-such-program-here"),
            Instant::now() + Duration::from_secs(5),
        );
        assert!(matches!(result, Err(RunError::Spawn(_))), "{result:?}");
    }
}
