//! A run with stdin on a pseudo-terminal and stdout and stderr as pipes, for
//! the pickers, which prompt only when stdin is a terminal. Unix only:
//! Windows has no pseudo-terminal of this kind.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal, kill_process};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

use super::golden::decode;
use super::sandbox::Run;

/// What the terminal does once the lines are typed.
#[derive(Clone, Copy, Debug)]
pub enum End {
    /// Nothing more: the run ends on its own.
    Open,
    /// End of input (Ctrl-D at the start of a line).
    Eof,
    /// Once the prompt `Choice: ` is on stdout, SIGINT (what Ctrl-C sends).
    Interrupt,
}

/// The longest a run may take; past it the run is killed and the test fails.
const RUN_BOUND: Duration = Duration::from_secs(60);

/// Runs `cmd` with stdin on a fresh pseudo-terminal: types `lines`, one per
/// line, then does what `end` says, and collects how the run ended.
pub fn run_on_terminal(mut cmd: Command, lines: &[&str], end: End) -> Run {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let name = ptsname(&master, Vec::new()).unwrap();
    let terminal = rustix::fs::open(
        std::ffi::OsStr::from_bytes(name.as_bytes()),
        OFlags::RDWR | OFlags::NOCTTY,
        Mode::empty(),
    )
    .unwrap();
    cmd.stdin(Stdio::from(terminal))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    // The command holds the parent's copy of the terminal end.
    drop(cmd);
    let stdout = collect(child.stdout.take().unwrap());
    let stderr = collect(child.stderr.take().unwrap());

    let mut master = File::from(master);
    // The terminal echoes what is typed; read it away, or the run's last
    // close of the terminal waits for that output to drain.
    let echo = collect(master.try_clone().unwrap());
    let mut typed: Vec<u8> = lines
        .iter()
        .flat_map(|l| [l.as_bytes(), b"\n"].concat())
        .collect();
    if let End::Eof = end {
        typed.push(0x04);
    }
    // A run that already ended has closed the terminal; that is its answer.
    let _ = master.write_all(&typed);
    if let End::Interrupt = end {
        let started = Instant::now();
        while !stdout
            .0
            .lock()
            .unwrap()
            .windows(8)
            .any(|w| w == b"Choice: ")
        {
            assert!(started.elapsed() < RUN_BOUND, "no prompt to interrupt");
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(300));
        kill_process(Pid::from_child(&child), Signal::INT).unwrap();
    }

    let status = wait(&mut child);
    drop(master);
    finish(echo);
    Run {
        code: status.code().unwrap_or(-1),
        stdout: decode(&finish(stdout), "stdout"),
        stderr: decode(&finish(stderr), "stderr"),
    }
}

type Collected = (Arc<Mutex<Vec<u8>>>, thread::JoinHandle<()>);

/// Reads `stream` to its end on a thread of its own.
fn collect(mut stream: impl Read + Send + 'static) -> Collected {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buffer);
    let reader = thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stream.read(&mut chunk) {
            if n == 0 {
                break;
            }
            sink.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });
    (buffer, reader)
}

fn finish((buffer, reader): Collected) -> Vec<u8> {
    reader.join().unwrap();
    std::mem::take(&mut *buffer.lock().unwrap())
}

/// The run's exit status, within [`RUN_BOUND`].
fn wait(child: &mut Child) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if started.elapsed() > RUN_BOUND {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the run took longer than {RUN_BOUND:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
