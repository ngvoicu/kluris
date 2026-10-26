//! The operating system's side of the sync services
//! (`kluris_core::service`): the environment a service is built from, the
//! service manager's commands run for real, and the signals that stop
//! `kluris sync --watch`.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use kluris_core::process::{RunError, run_until};
use kluris_core::service::{Manager, Ran, Runner, ServiceEnv};

use crate::error::CliError;
use crate::resolve::home;

/// The time one `launchctl`, `systemctl` or `schtasks` may take.
const COMMAND_TIME_LIMIT: Duration = Duration::from_secs(60);

/// Runs the service manager's programs, found on PATH, each within
/// [`COMMAND_TIME_LIMIT`] (with whatever it starts).
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Ran> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        match run_until(cmd, Instant::now() + COMMAND_TIME_LIMIT) {
            Ok(done) => Ok(Ran {
                code: done.status.code(),
                stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
            }),
            Err(RunError::Spawn(e)) => Err(e),
            Err(RunError::TimedOut) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{program} took longer than {COMMAND_TIME_LIMIT:?}"),
            )),
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// An absolute form of `path` (relative to the current folder).
fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(&path))
            .unwrap_or(path)
    }
}

/// The current account's SID (`whoami /user`), which a task's `UserId`
/// takes: `USERDOMAIN` is not the account's domain in every session (over
/// ssh a local account's is `WORKGROUP`, which no account maps from).
fn account_sid() -> Option<String> {
    let ran = SystemRunner
        .run("whoami", &["/user", "/fo", "csv", "/nh"])
        .ok()?;
    if ran.code != Some(0) {
        return None;
    }
    let line = ran.stdout.lines().next()?;
    let sid = line.rsplit(',').next()?.trim().trim_matches('"');
    sid.starts_with("S-1-").then(|| sid.to_string())
}

/// The service environment of this user and this kluris.
pub fn env() -> Result<ServiceEnv, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::new(format!("Cannot tell where this kluris is: {e}")))?;
    let kluris = std::fs::canonicalize(&exe).unwrap_or(exe);
    let user = if cfg!(windows) {
        account_sid().or_else(|| var("USERNAME"))
    } else {
        None
    };
    Ok(ServiceEnv {
        manager: Manager::current(),
        home: home()?,
        xdg_config_home: var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute()),
        local_app_data: var("LOCALAPPDATA").map(PathBuf::from),
        uid: kluris_core::service::current_uid(),
        user,
        kluris,
        path_var: var("PATH"),
        kluris_config: var("KLURIS_CONFIG").map(|p| absolute(PathBuf::from(p))),
        temp_dir: std::env::temp_dir(),
    })
}

/// Set by SIGINT and SIGTERM (Unix), or by Ctrl-C, Ctrl-Break and the
/// console closing (Windows): `kluris sync --watch` ends once the sync it
/// is in finishes.
pub static STOP: AtomicBool = AtomicBool::new(false);

/// Route the stop signals to [`STOP`]; they no longer end the process.
#[cfg(unix)]
pub fn catch_stop_signals() {
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        // SAFETY: the handler only stores to an atomic, which is
        // async-signal-safe.
        let _ = unsafe {
            signal_hook::low_level::register(signal, || STOP.store(true, Ordering::SeqCst))
        };
    }
}

#[cfg(windows)]
pub fn catch_stop_signals() {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        SetConsoleCtrlHandler,
    };
    use windows_sys::core::BOOL;

    unsafe extern "system" fn handler(event: u32) -> BOOL {
        match event {
            CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT => {
                STOP.store(true, Ordering::SeqCst);
                TRUE
            }
            _ => FALSE,
        }
    }
    // SAFETY: registers a handler that only stores to an atomic.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), TRUE);
    }
}
