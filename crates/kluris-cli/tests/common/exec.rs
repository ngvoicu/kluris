//! Files a test binary writes and then runs, and the processes it starts
//! meanwhile.
//!
//! A process this test binary starts while one of its threads has a file
//! open for writing holds that descriptor until its exec, and Linux runs no
//! file open for writing (ETXTBSY): a test that writes a binary and runs it
//! can be refused. Writing a file this binary runs takes [`EXEC_FILES`]
//! alone; starting a process shares it until `spawn` returns, when the child
//! has exec'd and its inherited descriptors are closed.

use std::process::{Command, Output, Stdio};
use std::sync::{PoisonError, RwLock};

/// Held alone while a file this binary runs is open for writing, shared
/// while a process is being started.
pub static EXEC_FILES: RwLock<()> = RwLock::new(());

/// Write a file this test binary then runs (`write` opens, writes and
/// closes it) while no process is being started.
pub fn writing_exe<T>(write: impl FnOnce() -> T) -> T {
    let _no_spawn = EXEC_FILES.write().unwrap_or_else(PoisonError::into_inner);
    write()
}

/// Run `start` (which starts processes) while no file this binary runs is
/// open for writing.
pub fn starting<T>(start: impl FnOnce() -> T) -> T {
    let _no_write = EXEC_FILES.read().unwrap_or_else(PoisonError::into_inner);
    start()
}

/// `cmd.output()`, started while no file this binary runs is open for
/// writing.
pub fn output(cmd: &mut Command) -> std::io::Result<Output> {
    let child = starting(|| {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })?;
    child.wait_with_output()
}
