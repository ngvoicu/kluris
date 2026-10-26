//! Putting a release's new binary in place of the installed one so that a
//! failure at any step leaves a working `kluris` (T-111):
//!
//! 1. Stage: copy the new binary to `.kluris(.exe).new-<pid>-<n>` beside
//!    the installed one (same folder, so the renames below stay on one
//!    file system), with the installed binary's mode, synced.
//! 2. Probe: run the staged copy with `--version` under a timeout; it must
//!    exit 0 and print exactly `kluris <version>`.
//! 3. Swap. Unix: rename the staged copy over the installed one (atomic).
//!    Windows: rename the installed binary (it may be running; renaming is
//!    allowed) to `.kluris.exe.old-<pid>-<n>`, then the staged copy to
//!    `kluris.exe`; when that fails, rename the old one back.
//!
//! Nothing is deleted before the new binary is in place; a staged copy that
//! is not used is removed. A running exe cannot be deleted on Windows, so
//! the old one stays until its process ends: the caller schedules that, and
//! [`remove_swap_leftovers`] (run by `self-update` and `doctor`) sweeps any
//! left, with the staged copies an interrupted update left.
//!
//! Every step goes through [`SwapSteps`], so tests can fail each one.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::APP_NAME;

/// The file operations and the probe a swap is made of.
pub trait SwapSteps {
    /// Write `new_binary`'s bytes to the new file `staged` (it must not
    /// exist), with `like`'s permissions, and sync it.
    fn stage(&mut self, new_binary: &Path, staged: &Path, like: &Path) -> io::Result<()>;
    /// Run `exe --version`.
    fn probe(&mut self, exe: &Path) -> io::Result<Probe>;
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&mut self, path: &Path) -> io::Result<()>;
}

/// What `--version` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The exit code; `None` when it ended on a signal or was stopped.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    /// It was stopped at the timeout.
    pub timed_out: bool,
}

/// The real file system and processes.
#[derive(Debug, Clone, Copy)]
pub struct RealSteps {
    /// How long `--version` may take.
    pub timeout: Duration,
}

impl Default for RealSteps {
    fn default() -> RealSteps {
        RealSteps {
            timeout: Duration::from_secs(30),
        }
    }
}

/// At most this much of `--version`'s output is read.
const PROBE_OUTPUT_MAX: u64 = 64 * 1024;

impl SwapSteps for RealSteps {
    fn stage(&mut self, new_binary: &Path, staged: &Path, like: &Path) -> io::Result<()> {
        let mut source = File::open(new_binary)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staged)?;
        io::copy(&mut source, &mut file)?;
        let permissions = match fs::metadata(like) {
            Ok(meta) => meta.permissions(),
            Err(_) => fs::metadata(new_binary)?.permissions(),
        };
        file.set_permissions(permissions)?;
        file.sync_all()
    }

    fn probe(&mut self, exe: &Path) -> io::Result<Probe> {
        let mut child = spawn_version(exe)?;
        let stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(out) = stdout {
                let _ = out.take(PROBE_OUTPUT_MAX).read_to_end(&mut bytes);
            }
            bytes
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(match status {
            Some(status) => Probe {
                code: status.code(),
                stdout: reader.join().unwrap_or_default(),
                timed_out: false,
            },
            // Something it started may still hold the pipe: leave the
            // reader, it ends with the pipe.
            None => Probe {
                code: None,
                stdout: Vec::new(),
                timed_out: true,
            },
        })
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
}

/// Start `exe --version`. A file just written can be briefly "busy" on
/// Unix while another thread's fork still holds it open: retried.
fn spawn_version(exe: &Path) -> io::Result<std::process::Child> {
    let mut tries = 0;
    loop {
        let started = Command::new(exe)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

/// What to swap.
#[derive(Debug, Clone, Copy)]
pub struct SwapRequest<'a> {
    /// The installed binary (`<install_prefix>/kluris(.exe)`).
    pub installed: &'a Path,
    /// The release's binary, extracted and checked against its checksums.
    pub new_binary: &'a Path,
    /// The release's version: `--version` must print `kluris <version>`.
    pub version: &'a str,
    /// Windows' algorithm (rename aside, then in) or Unix's (rename over).
    pub windows: bool,
}

/// A swap that went through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swapped {
    /// Windows: where the old binary went (it is deleted once its process
    /// ends, or swept by [`remove_swap_leftovers`]).
    pub old: Option<PathBuf>,
}

/// Why the new binary is not in place.
#[derive(Debug)]
pub enum SwapError {
    /// The staged copy could not be written.
    Stage(io::Error),
    /// The staged copy does not run here, or not as that release.
    Unusable { version: String, why: String },
    /// The installed binary could not be renamed aside (Windows).
    Aside(io::Error),
    /// The staged copy could not be renamed into place (the old binary is
    /// back where it was).
    Place(io::Error),
    /// Neither the staged copy into place nor the old binary back: the old
    /// binary is at `old`, nothing was deleted.
    Restore {
        place: io::Error,
        restore: io::Error,
        old: PathBuf,
        installed: PathBuf,
    },
}

impl SwapError {
    /// The installed binary is where it was, as it was.
    pub fn kept_installed(&self) -> bool {
        !matches!(self, SwapError::Restore { .. })
    }
}

impl fmt::Display for SwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const KEPT: &str = "the installed kluris is unchanged";
        match self {
            SwapError::Stage(e) => {
                write!(
                    f,
                    "could not write the new binary beside the installed one ({e}); {KEPT}"
                )
            }
            SwapError::Unusable { version, why } => {
                write!(
                    f,
                    "the new kluris {version} does not work here ({why}); {KEPT}"
                )
            }
            SwapError::Aside(e) => {
                write!(f, "could not move the installed kluris aside ({e}); {KEPT}")
            }
            SwapError::Place(e) => {
                write!(f, "could not put the new binary in place ({e}); {KEPT}")
            }
            SwapError::Restore {
                place,
                restore,
                old,
                installed,
            } => write!(
                f,
                "could not put the new binary in place ({place}) nor move the installed kluris \
                 back ({restore}): it is at {}; rename it to {} to use it again",
                old.display(),
                installed.display()
            ),
        }
    }
}

impl std::error::Error for SwapError {}

/// Stage, probe and swap ([module docs](self)).
pub fn swap(req: &SwapRequest, steps: &mut dyn SwapSteps) -> Result<Swapped, SwapError> {
    let (staged, old) = side_names(req.installed);
    let discard = |steps: &mut dyn SwapSteps| {
        if fs::symlink_metadata(&staged).is_ok() {
            let _ = steps.remove(&staged);
        }
    };

    if let Err(e) = steps.stage(req.new_binary, &staged, req.installed) {
        discard(steps);
        return Err(SwapError::Stage(e));
    }
    let checked = match steps.probe(&staged) {
        Ok(probe) => check_probe(&probe, req.version),
        Err(e) => Err(format!("it does not start: {e}")),
    };
    if let Err(why) = checked {
        discard(steps);
        return Err(SwapError::Unusable {
            version: req.version.to_string(),
            why,
        });
    }

    if !req.windows {
        if let Err(e) = steps.rename(&staged, req.installed) {
            discard(steps);
            return Err(SwapError::Place(e));
        }
        return Ok(Swapped { old: None });
    }
    if let Err(e) = steps.rename(req.installed, &old) {
        discard(steps);
        return Err(SwapError::Aside(e));
    }
    if let Err(place) = steps.rename(&staged, req.installed) {
        let restored = steps.rename(&old, req.installed);
        discard(steps);
        return Err(match restored {
            Ok(()) => SwapError::Place(place),
            Err(restore) => SwapError::Restore {
                place,
                restore,
                old,
                installed: req.installed.to_path_buf(),
            },
        });
    }
    Ok(Swapped { old: Some(old) })
}

/// `Ok` when `--version` exited 0 and printed exactly `kluris <version>`
/// (one line); otherwise why not.
pub fn check_probe(probe: &Probe, version: &str) -> Result<(), String> {
    if probe.timed_out {
        return Err("it did not answer `--version` in time".into());
    }
    match probe.code {
        Some(0) => {}
        Some(code) => return Err(format!("`--version` exited {code}")),
        None => return Err("`--version` ended on a signal".into()),
    }
    let text = String::from_utf8_lossy(&probe.stdout);
    let line = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(&text);
    let want = format!("{APP_NAME} {version}");
    if line == want {
        Ok(())
    } else {
        let shown: String = line.chars().take(80).collect();
        Err(format!("`--version` printed {shown:?}, not {want:?}"))
    }
}

/// `(staged, old)` beside `installed`: `.<name>.new-<pid>-<n>` and
/// `.<name>.old-<pid>-<n>`, `n` the first for which neither exists.
fn side_names(installed: &Path) -> (PathBuf, PathBuf) {
    let dir = installed.parent().unwrap_or(Path::new(""));
    let name = installed
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| APP_NAME.to_string());
    let pid = std::process::id();
    let mut n: u64 = 0;
    loop {
        let staged = dir.join(format!(".{name}.new-{pid}-{n}"));
        let old = dir.join(format!(".{name}.old-{pid}-{n}"));
        if fs::symlink_metadata(&staged).is_err() && fs::symlink_metadata(&old).is_err() {
            return (staged, old);
        }
        n += 1;
    }
}

/// `.kluris(.exe).<kind>-<pid>-<n>`, nothing more: `kind` when `name` is
/// one, with `<pid>` and `<n>` decimal digits.
fn side_kind(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('.')?.strip_prefix(APP_NAME)?;
    let rest = rest.strip_prefix(".exe").unwrap_or(rest);
    let (kind, rest) = rest.strip_prefix('.')?.split_once('-')?;
    let (pid, n) = rest.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(n)).then_some(kind)
}

/// Whether `name` is an old binary a Windows swap left:
/// `.kluris.old-<pid>-<n>` or `.kluris.exe.old-<pid>-<n>`.
pub fn is_stale_old(name: &str) -> bool {
    side_kind(name) == Some("old")
}

/// Whether `name` is a staged copy: `.kluris(.exe).new-<pid>-<n>`.
pub fn is_staging_name(name: &str) -> bool {
    side_kind(name) == Some("new")
}

/// A staged copy this old was left by an interrupted update, not made by
/// one running now.
pub const STAGING_MAX_AGE: Duration = Duration::from_secs(3600);

/// Remove what earlier swaps left in `dir`, regular files only (never a
/// symlink): every old binary ([`is_stale_old`]; one still running on
/// Windows stays for a later sweep), and every staged copy
/// ([`is_staging_name`]) last written over [`STAGING_MAX_AGE`] ago. What
/// went.
pub fn remove_swap_leftovers(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = std::time::SystemTime::now();
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // DirEntry::metadata does not follow a symlink.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let gone = if is_stale_old(&name) {
            true
        } else if is_staging_name(&name) {
            meta.modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > STAGING_MAX_AGE)
        } else {
            false
        };
        let path = entry.path();
        if gone && fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(code: Option<i32>, out: &str) -> Probe {
        Probe {
            code,
            stdout: out.as_bytes().to_vec(),
            timed_out: false,
        }
    }

    #[test]
    fn the_probe_wants_exactly_kluris_and_the_version() {
        assert_eq!(
            check_probe(&probe(Some(0), "kluris 3.1.0\n"), "3.1.0"),
            Ok(())
        );
        assert_eq!(
            check_probe(&probe(Some(0), "kluris 3.1.0\r\n"), "3.1.0"),
            Ok(())
        );
        assert_eq!(
            check_probe(&probe(Some(0), "kluris 3.1.0"), "3.1.0"),
            Ok(())
        );
        for (p, why) in [
            (probe(Some(0), "kluris 3.1.0\n\n"), "printed"),
            (probe(Some(0), "kluris 3.1.1\n"), "printed"),
            (probe(Some(0), " kluris 3.1.0\n"), "printed"),
            (probe(Some(0), ""), "printed"),
            (probe(Some(1), "kluris 3.1.0\n"), "exited 1"),
            (probe(None, "kluris 3.1.0\n"), "signal"),
            (
                Probe {
                    code: None,
                    stdout: Vec::new(),
                    timed_out: true,
                },
                "in time",
            ),
        ] {
            let err = check_probe(&p, "3.1.0").unwrap_err();
            assert!(err.contains(why), "{p:?}: {err}");
        }
    }

    #[test]
    fn stale_old_names_are_exact() {
        for yes in [".kluris.old-1-0", ".kluris.exe.old-123-45"] {
            assert!(is_stale_old(yes), "{yes}");
        }
        for no in [
            "kluris.old-1-0",
            ".kluris.old-1-0.bak",
            ".kluris.old-x-0",
            ".kluris.old-1-",
            ".kluris.old--1",
            ".kluris.new-1-0",
            ".kluris.exe.exe.old-1-0",
            ".klurisx.old-1-0",
            ".kluris.old-1-2-3",
        ] {
            assert!(!is_stale_old(no), "{no}");
        }
    }

    #[test]
    fn staging_names_are_exact() {
        for yes in [".kluris.new-1-0", ".kluris.exe.new-123-45"] {
            assert!(is_staging_name(yes), "{yes}");
        }
        for no in [
            "kluris.new-1-0",
            ".kluris.new-1-0.part",
            ".kluris.new-x-0",
            ".kluris.new-1-",
            ".kluris.old-1-0",
            ".klurisx.new-1-0",
            ".kluris.new-1-2-3",
        ] {
            assert!(!is_staging_name(no), "{no}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_hangs_is_stopped_at_the_timeout() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slow");
        fs::write(&exe, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        let p = RealSteps {
            timeout: Duration::from_millis(300),
        }
        .probe(&exe)
        .unwrap();
        assert!(p.timed_out);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(check_probe(&p, "1").unwrap_err().contains("in time"));
    }
}
