//! The per-user sync services earlier releases installed, each running
//! `<kluris> sync --watch --brain <b>` for the user from login on:
//!
//! - macOS: the launchd agent `~/Library/LaunchAgents/
//!   dev.ngvoicu.kluris.sync.<b>.plist`, loaded in `gui/<uid>`, logging to
//!   `~/Library/Logs/kluris/sync-<b>.log`; launchd restarts it after any
//!   failing exit (`KeepAlive {SuccessfulExit: false}`).
//! - Linux: the systemd user unit `$XDG_CONFIG_HOME/systemd/user/
//!   kluris-sync-<b>.service` (`~/.config` without it), enabled, logging to
//!   the journal, restarted on failure.
//! - Windows: the Task Scheduler task `\Kluris\sync-<b>`, run at logon
//!   under `conhost.exe --headless` with `--log-file
//!   %LOCALAPPDATA%\kluris\logs\sync-<b>.log`.
//!
//! After an upgrade such a service starts this kluris, whose hidden `sync
//! --watch` arm calls [`remove_one`] for its own brain and exits 0: a 0
//! ends both restart loops, and the definition gone ends the next login's
//! start. [`sweep`] (`doctor`, `self-uninstall`) finds every one by name
//! and removes it, stopping what runs. Everything runs as the user, through
//! [`Runner`], so tests assert the commands with a fake.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::process::{RunError, run_until};

/// The service manager of the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Launchd,
    Systemd,
    TaskScheduler,
}

impl Manager {
    /// This build's.
    pub fn current() -> Manager {
        if cfg!(windows) {
            Manager::TaskScheduler
        } else if cfg!(target_os = "macos") {
            Manager::Launchd
        } else {
            Manager::Systemd
        }
    }
}

/// The user id launchd's `gui/<uid>` domain names (0 where there is none).
pub fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        rustix::process::getuid().as_raw()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// One command's end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Its words: stderr, else stdout, else its exit.
    fn words(&self) -> String {
        [self.stderr.trim(), self.stdout.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| match self.code {
                Some(code) => format!("exit {code}"),
                None => "killed".to_string(),
            })
    }
}

/// Runs the service manager's commands.
pub trait Runner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Ran>;
}

/// The time one `launchctl`, `systemctl` or `schtasks` may take: a stop
/// waits for a sync in progress.
const COMMAND_TIME_LIMIT: Duration = Duration::from_secs(60);

/// Runs the programs found on PATH (Windows finds `schtasks` in System32
/// whatever PATH holds), each within [`COMMAND_TIME_LIMIT`] with whatever
/// it starts.
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

/// Where the services are, and how their manager is asked.
pub struct Env<'a> {
    pub manager: Manager,
    pub home: PathBuf,
    /// `XDG_CONFIG_HOME` when set, non-empty and absolute (systemd).
    pub xdg_config_home: Option<PathBuf>,
    /// `LOCALAPPDATA` when set and non-empty (Task Scheduler's logs).
    pub local_app_data: Option<PathBuf>,
    /// launchd's `gui/<uid>` domain.
    pub uid: u32,
    pub runner: &'a dyn Runner,
}

impl std::fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("manager", &self.manager)
            .field("home", &self.home)
            .field("xdg_config_home", &self.xdg_config_home)
            .field("local_app_data", &self.local_app_data)
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}

/// A service found by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub brain: String,
    /// The label, unit or task name.
    pub name: String,
    /// The file that defines it (none for a task).
    pub definition: Option<PathBuf>,
    /// Its log file, when there is one.
    pub log: Option<PathBuf>,
}

/// What became of one brain's service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub brain: String,
    /// The label, unit or task name.
    pub name: String,
    /// What went, in order: the definition file (a task by its name), the
    /// log.
    pub removed: Vec<PathBuf>,
    /// The first step that failed, in its words; the steps after it ran.
    pub problem: Option<String>,
}

const LABEL_PREFIX: &str = "dev.ngvoicu.kluris.sync.";
const UNIT_PREFIX: &str = "kluris-sync-";
const TASK_PREFIX: &str = r"\Kluris\sync-";

/// A name `kluris create` accepts, as every service's brain was; nothing
/// else reaches a file name, a label or a task name.
fn is_brain_name(brain: &str) -> bool {
    let mut chars = brain.chars();
    brain.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The brain of a definition file or task name, if it is a service's.
fn brain_of<'a>(name: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let brain = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    is_brain_name(brain).then_some(brain)
}

impl Env<'_> {
    /// The label, unit or task name of the brain's service.
    fn name(&self, brain: &str) -> String {
        match self.manager {
            Manager::Launchd => format!("{LABEL_PREFIX}{brain}"),
            Manager::Systemd => format!("{UNIT_PREFIX}{brain}.service"),
            Manager::TaskScheduler => format!("{TASK_PREFIX}{brain}"),
        }
    }

    /// The folder of the definition files.
    fn location(&self) -> Option<PathBuf> {
        match self.manager {
            Manager::Launchd => Some(self.home.join("Library").join("LaunchAgents")),
            Manager::Systemd => Some(
                self.xdg_config_home
                    .clone()
                    .unwrap_or_else(|| self.home.join(".config"))
                    .join("systemd")
                    .join("user"),
            ),
            Manager::TaskScheduler => None,
        }
    }

    fn definition(&self, brain: &str) -> Option<PathBuf> {
        let file = match self.manager {
            Manager::Launchd => format!("{}.plist", self.name(brain)),
            _ => self.name(brain),
        };
        self.location().map(|dir| dir.join(file))
    }

    /// The log file the service wrote (none: the journal).
    fn log(&self, brain: &str) -> Option<PathBuf> {
        let file = format!("sync-{brain}.log");
        match self.manager {
            Manager::Launchd => Some(self.home.join("Library").join("Logs").join("kluris")),
            Manager::Systemd => None,
            Manager::TaskScheduler => self
                .local_app_data
                .as_ref()
                .map(|lad| lad.join("kluris").join("logs")),
        }
        .map(|dir| dir.join(file))
    }

    fn found(&self, brain: &str, definition: Option<PathBuf>) -> Found {
        Found {
            brain: brain.to_string(),
            name: self.name(brain),
            definition,
            log: self.log(brain).filter(|p| exists(p)),
        }
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Ran, String> {
        self.runner.run(program, args).map_err(|e| {
            let line = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            format!("`{line}` did not run: {e}")
        })
    }

    /// Run a command that must succeed.
    fn must(&self, program: &str, args: &[&str]) -> Result<(), String> {
        let ran = self.run(program, args)?;
        if ran.ok() {
            Ok(())
        } else {
            Err(format!("`{program} {}`: {}", args.join(" "), ran.words()))
        }
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Every service an earlier release installed, by its name, sorted by
/// brain. Only reads: the definition folder (macOS, Linux), or `schtasks
/// /Query` (Windows). What cannot be read holds none.
pub fn find(env: &Env) -> Vec<Found> {
    let mut found = match env.manager {
        Manager::Launchd | Manager::Systemd => {
            let suffix = if env.manager == Manager::Launchd {
                ".plist"
            } else {
                ".service"
            };
            let prefix = if env.manager == Manager::Launchd {
                LABEL_PREFIX
            } else {
                UNIT_PREFIX
            };
            let Some(dir) = env.location() else {
                return Vec::new();
            };
            let Ok(entries) = fs::read_dir(&dir) else {
                return Vec::new();
            };
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name();
                    let brain = brain_of(name.to_str()?, prefix, suffix)?.to_string();
                    Some(env.found(&brain, Some(entry.path())))
                })
                .collect::<Vec<_>>()
        }
        Manager::TaskScheduler => {
            let Ok(ran) = env.run("schtasks", &["/Query", "/FO", "CSV", "/NH"]) else {
                return Vec::new();
            };
            if !ran.ok() {
                return Vec::new();
            }
            // Task names are never translated; the other columns are.
            let mut brains: Vec<String> = ran
                .stdout
                .lines()
                .filter_map(|row| {
                    let name = csv_fields(row.trim()).into_iter().next()?;
                    brain_of(&name, TASK_PREFIX, "").map(str::to_string)
                })
                .collect();
            brains.sort();
            brains.dedup();
            brains.iter().map(|b| env.found(b, None)).collect()
        }
    };
    found.sort_by(|a, b| a.brain.cmp(&b.brain));
    found
}

/// Remove every service an earlier release installed ([`find`]), stopping
/// what runs: for `doctor` and `self-uninstall`.
pub fn sweep(env: &Env) -> Vec<Removed> {
    find(env)
        .iter()
        .map(|found| remove(env, &found.brain, true))
        .collect()
}

/// Remove `brain`'s service from inside it: the hidden `sync --watch` arm.
/// A name no service could carry removes nothing and runs nothing.
pub fn remove_one(brain: &str, env: &Env) -> Removed {
    remove(env, brain, false)
}

/// Remove `brain`'s service, its definition and its log; `stop` it when
/// it is not this process.
///
/// - launchd: a loaded job is booted out, which stops it, then the plist
///   and the log go; a job that will not go keeps its plist. From inside
///   the job the plist and the log go first, as the bootout ends this
///   process with SIGTERM, so nothing may come after it.
/// - systemd: `disable` (`--now` with `stop`), the unit file (and the
///   `default.target.wants` link a failed disable leaves), `daemon-reload`.
///   From inside, the unit is never stopped: that would end this process
///   before its file is gone; its exit 0 ends it.
/// - Task Scheduler: with `stop`, `/End` (ending its conhost, which the
///   watch it runs follows), then `/Delete`, then the log.
fn remove(env: &Env, brain: &str, stop: bool) -> Removed {
    let mut out = Removed {
        brain: brain.to_string(),
        name: env.name(brain),
        removed: Vec::new(),
        problem: None,
    };
    if !is_brain_name(brain) {
        return out;
    }
    match env.manager {
        Manager::Launchd => {
            let target = format!("gui/{}/{}", env.uid, out.name);
            let boot_out = || -> Result<(), String> {
                let loaded = env
                    .run("launchctl", &["print", &target])
                    .is_ok_and(|ran| ran.ok());
                if loaded {
                    env.must("launchctl", &["bootout", &target])
                } else {
                    Ok(())
                }
            };
            // From outside the job, a plist is kept while its job will not
            // go, so the next sweep finds it and tries again.
            if stop && let Err(e) = boot_out() {
                note(&mut out, Err(e));
                return out;
            }
            if let Some(plist) = env.definition(brain) {
                let r = delete(&plist, &mut out.removed);
                note(&mut out, r);
            }
            let r = delete_log(env, brain, &mut out.removed);
            note(&mut out, r);
            if !stop {
                let r = boot_out();
                note(&mut out, r);
            }
        }
        Manager::Systemd => {
            let Some(unit) = env.definition(brain) else {
                return out;
            };
            if !exists(&unit) {
                return out;
            }
            let name = out.name.clone();
            let mut disable = vec!["--user", "disable"];
            if stop {
                disable.push("--now");
            }
            disable.push(&name);
            let disabled = env.must("systemctl", &disable);
            let failed = disabled.is_err();
            note(&mut out, disabled);
            let r = delete(&unit, &mut out.removed);
            note(&mut out, r);
            if failed {
                if let Some(dir) = env.location() {
                    let wants = dir.join("default.target.wants").join(&name);
                    let _ = fs::remove_file(wants);
                }
            } else {
                let r = env.must("systemctl", &["--user", "daemon-reload"]);
                note(&mut out, r);
            }
        }
        Manager::TaskScheduler => {
            let task = out.name.clone();
            let there = env
                .run("schtasks", &["/Query", "/TN", &task])
                .is_ok_and(|ran| ran.ok());
            if there {
                if stop {
                    // Not running is fine.
                    let _ = env.run("schtasks", &["/End", "/TN", &task]);
                }
                match env.must("schtasks", &["/Delete", "/TN", &task, "/F"]) {
                    Ok(()) => out.removed.push(PathBuf::from(&task)),
                    Err(e) => note(&mut out, Err(e)),
                }
            }
            let r = delete_log(env, brain, &mut out.removed);
            note(&mut out, r);
        }
    }
    out
}

/// Keep the first failure.
fn note(out: &mut Removed, result: Result<(), String>) {
    if let (Err(e), None) = (result, &out.problem) {
        out.problem = Some(e);
    }
}

/// Remove `path` when it is there, noting it.
fn delete(path: &Path, removed: &mut Vec<PathBuf>) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => {
            removed.push(path.to_path_buf());
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Remove the brain's log, then its folder if that leaves it empty.
fn delete_log(env: &Env, brain: &str, removed: &mut Vec<PathBuf>) -> Result<(), String> {
    let Some(log) = env.log(brain) else {
        return Ok(());
    };
    delete(&log, removed)?;
    if let Some(dir) = log.parent() {
        // Only an empty folder goes.
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

/// The fields of one `schtasks /FO CSV` row.
fn csv_fields(row: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => fields.push(std::mem::take(&mut field)),
            (c, _) => field.push(c),
        }
    }
    fields.push(field);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_rows_split_on_commas_outside_quotes() {
        assert_eq!(
            csv_fields(r#""\Kluris\sync-a","27/10/2026, 08:00","Ready""#),
            [r"\Kluris\sync-a", "27/10/2026, 08:00", "Ready"]
        );
        assert_eq!(csv_fields(r#""a ""b""",c"#), [r#"a "b""#, "c"]);
    }

    #[test]
    fn only_service_shaped_names_are_brains() {
        assert_eq!(
            brain_of("kluris-sync-acme.service", UNIT_PREFIX, ".service"),
            Some("acme")
        );
        for name in [
            "kluris-sync-.service",
            "kluris-sync-A.service",
            "kluris-sync-../x.service",
            "other.service",
        ] {
            assert_eq!(brain_of(name, UNIT_PREFIX, ".service"), None, "{name}");
        }
    }
}
