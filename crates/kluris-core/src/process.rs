//! Running a program with a deadline: past it the program is killed with
//! every process it started (no orphan), and the caller goes on.
//!
//! The program runs in a process group of its own on Unix and in a job
//! object on Windows, so a timeout ends the whole tree (git's
//! `git-remote-https`, `ssh` and `upload-pack` included) and a normal exit
//! ends whatever the program left running.

use std::io::{self, Read};
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
    /// The deadline passed; the program and its children were killed.
    TimedOut,
}

/// Run `cmd` with stdin closed and its output captured, killing it and its
/// children once `deadline` passes. Whatever the program leaves running when
/// it exits is killed too.
pub fn run_until(cmd: Command, deadline: Instant) -> Result<Finished, RunError> {
    run(cmd, Some(deadline))
}

/// [`run_until`] without a deadline: for work that must never be stopped
/// halfway (a checkout), the program runs to its end. Whatever it leaves
/// running when it exits is still killed.
pub fn run_to_end(cmd: Command) -> Result<Finished, RunError> {
    run(cmd, None)
}

fn run(mut cmd: Command, deadline: Option<Instant>) -> Result<Finished, RunError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    tree::isolate(&mut cmd);
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let tree = tree::Tree::adopt(&child);
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut status = None;
    let mut out = None;
    let mut err = None;
    loop {
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
        if deadline.is_some_and(|d| Instant::now() >= d) {
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
            thread::spawn(move || {
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
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    use rustix::process::{Pid, Signal, kill_process_group};

    /// The program leads a new process group.
    pub fn isolate(cmd: &mut Command) {
        cmd.process_group(0);
    }

    /// The process group of a started program.
    pub struct Tree(Option<Pid>);

    impl Tree {
        pub fn adopt(child: &Child) -> Tree {
            Tree(i32::try_from(child.id()).ok().and_then(Pid::from_raw))
        }

        /// Kill every process left in the group. The group's id cannot be
        /// reused while one of its processes lives, and an empty group is
        /// gone (ESRCH, ignored).
        pub fn end(&self) {
            if let Some(pid) = self.0 {
                let _ = kill_process_group(pid, Signal::KILL);
            }
        }
    }
}

#[cfg(windows)]
mod tree {
    use std::os::windows::io::AsRawHandle;
    use std::process::{Child, Command};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    /// Nothing to set before the start: the job is joined right after it.
    pub fn isolate(_: &mut Command) {}

    /// A job object holding the program (and so everything it starts),
    /// killed when the job's handle closes.
    pub struct Tree(Option<HANDLE>);

    impl Tree {
        pub fn adopt(child: &Child) -> Tree {
            // SAFETY: plain Win32 calls on a handle created here and the
            // child's process handle, which `child` keeps open; every
            // pointer is to a live local of the stated size.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Tree(None);
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
                    return Tree(None);
                }
                Tree(Some(job))
            }
        }

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

    #[cfg(unix)]
    #[test]
    fn without_a_deadline_the_program_runs_to_its_end() {
        let started = Instant::now();
        let done = run_to_end(sh("sleep 1; echo finished")).unwrap();
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert_eq!(done.stdout, b"finished\n");
        assert!(done.status.success());
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
