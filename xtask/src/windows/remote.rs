//! The programs a run starts on this side (`ssh`, `scp`), behind a trait so the
//! tests can answer for them. Every call carries its bound: a program still
//! running when it passes is killed.

use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// A program, its arguments and how long it may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    pub bound: Duration,
}

impl Call {
    pub fn new(program: &str, args: Vec<String>, bound: Duration) -> Self {
        Self {
            program: program.into(),
            args,
            bound,
        }
    }
}

/// What a captured program said. `code` is none when a signal or the bound ended it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Captured {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// How a streamed program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Streamed {
    /// By itself, with its status (none for a signal).
    Exited(Option<i32>),
    /// Killed at its bound.
    TimedOut,
    /// Killed because this program was asked to stop.
    Stopped,
}

pub trait Runner {
    /// Runs `call` to its end or its bound and keeps what it says.
    fn capture(&mut self, call: &Call) -> Result<Captured, String>;

    /// Runs `call`, handing what it says to `sink` as it comes, until it ends,
    /// its bound passes or `stop` answers true.
    fn stream(
        &mut self,
        call: &Call,
        sink: &mut dyn FnMut(Stream, &[u8]) -> io::Result<()>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Streamed, String>;
}

/// The real programs.
pub struct System;

/// How often a wait looks at the clock and the stop.
const LOOK: Duration = Duration::from_millis(50);

impl Runner for System {
    fn capture(&mut self, call: &Call) -> Result<Captured, String> {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let ended = self.stream(
            call,
            &mut |stream, bytes| {
                match stream {
                    Stream::Out => out.extend_from_slice(bytes),
                    Stream::Err => err.extend_from_slice(bytes),
                }
                Ok(())
            },
            &|| false,
        )?;
        let (code, timed_out) = match ended {
            Streamed::Exited(code) => (code, false),
            Streamed::TimedOut | Streamed::Stopped => (None, true),
        };
        Ok(Captured {
            code,
            stdout: String::from_utf8_lossy(&out).into_owned(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
            timed_out,
        })
    }

    fn stream(
        &mut self,
        call: &Call,
        sink: &mut dyn FnMut(Stream, &[u8]) -> io::Result<()>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Streamed, String> {
        let started = |e: io::Error| format!("could not start {}: {e}", call.program);
        let mut child = Command::new(&call.program)
            .args(&call.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(started)?;
        let deadline = Instant::now() + call.bound;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            end(&mut child);
            return Err(format!("{}'s output could not be read", call.program));
        };
        let (to, arriving) = mpsc::channel();
        let pumps = [
            pump(stdout, Stream::Out, to.clone()),
            pump(stderr, Stream::Err, to),
        ];
        loop {
            if stop() {
                end(&mut child);
                return Ok(Streamed::Stopped);
            }
            if Instant::now() >= deadline {
                end(&mut child);
                return Ok(Streamed::TimedOut);
            }
            match arriving.recv_timeout(LOOK) {
                Ok((stream, bytes)) => {
                    if let Err(e) = sink(stream, &bytes) {
                        end(&mut child);
                        return Err(format!("could not write what {} said: {e}", call.program));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        for pump in pumps {
            let _ = pump.join();
        }
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(Streamed::Exited(status.code())),
                Ok(None) if stop() => {
                    end(&mut child);
                    return Ok(Streamed::Stopped);
                }
                Ok(None) if Instant::now() >= deadline => {
                    end(&mut child);
                    return Ok(Streamed::TimedOut);
                }
                Ok(None) => thread::sleep(LOOK),
                Err(e) => {
                    end(&mut child);
                    return Err(format!("could not wait for {}: {e}", call.program));
                }
            }
        }
    }
}

fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads `source` to its end on a thread of its own, handing on what it reads.
fn pump(
    mut source: impl Read + Send + 'static,
    stream: Stream,
    to: mpsc::Sender<(Stream, Vec<u8>)>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            match source.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if to.send((stream, buffer[..count].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    })
}
