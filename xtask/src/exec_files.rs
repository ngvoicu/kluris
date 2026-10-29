//! Every process xtask starts, started while no file it runs is open for
//! writing.
//!
//! A process started while one of this binary's threads has a file open for
//! writing holds that descriptor until its exec, and Linux runs no file open
//! for writing (ETXTBSY): a test that writes a fake program and runs it can
//! be refused. Writing a file this binary runs takes [`EXEC_FILES`] alone;
//! starting a process shares it until `spawn` returns, when the child has
//! exec'd and its inherited descriptors are closed.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{PoisonError, RwLock};

static EXEC_FILES: RwLock<()> = RwLock::new(());

/// Writes a file this binary then runs (`write` opens, writes and closes it)
/// while no process is being started.
#[cfg(test)]
pub fn writing_exe<T>(write: impl FnOnce() -> T) -> T {
    let _no_spawn = EXEC_FILES.write().unwrap_or_else(PoisonError::into_inner);
    write()
}

/// `cmd.spawn()`, while no file this binary runs is open for writing.
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let _no_write = EXEC_FILES.read().unwrap_or_else(PoisonError::into_inner);
    cmd.spawn()
}

/// `cmd.output()`, started through [`spawn`].
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(cmd)?.wait_with_output()
}

/// `cmd.status()`, started through [`spawn`].
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn(cmd)?.wait()
}
