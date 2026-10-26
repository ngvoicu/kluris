//! `kluris doctor` (v2 `cli.py:1900-2002`, research-03 §1.13, with the
//! interview-01 #8 changes: `parity/allowlist.md`).
//!
//! Checks, in order:
//! 1. `git`: `git --version` runs and exits 0.
//! 2. `path`: the first `kluris` on PATH is this binary; a warning
//!    otherwise, never a failure. The other file is only looked up, never
//!    read or run.
//! 3. `config`: the global config reads (a missing file is an empty config).
//! 4. `brain:<name>`: each registered brain's folder exists and its
//!    `kluris.yml` reads.
//!
//! Then, unless `--no-refresh` (and only when the config reads):
//! 5. `companion:<name>`: each known companion that is installed or that a
//!    brain opts into is refreshed (v2).
//! 6. `skills`: every brain's skill is installed
//!    ([`install_skills`], which also migrates a v2 install).

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use crate::agents::install::{InstallReport, InstallRequest, install_skills, read_brain_configs};
use crate::companions;
use crate::config::{GlobalConfig, read_global_config};

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    /// Worth fixing, but not a failure (exit 0).
    Warn,
    Fail,
}

/// One check: v2's `{name, passed, detail}`, with a warning state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn new(name: impl Into<String>, status: Status, detail: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// What [`run`] works from: everything the environment decides.
#[derive(Debug, Clone, Copy)]
pub struct DoctorRequest<'a> {
    pub config_path: &'a Path,
    /// The home directory (`config::home_dir`).
    pub home: &'a Path,
    /// `COPILOT_HOME` when set and non-empty.
    pub copilot_home: Option<&'a Path>,
    /// `PATH`, as this process has it.
    pub path_var: Option<&'a OsStr>,
    /// This binary (`std::env::current_exe`), when known.
    pub current_exe: Option<&'a Path>,
    /// Refresh the companions and skills (not `--no-refresh`).
    pub refresh: bool,
}

/// One companion refresh (v2's `companions[]` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionRefresh {
    pub name: String,
    /// `None` when it was refreshed.
    pub error: Option<String>,
}

/// The outcome of [`run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
    pub companions: Vec<CompanionRefresh>,
    /// The skill install; `None` when it did not run.
    pub skills: Option<InstallReport>,
}

impl DoctorReport {
    /// No check failed (warnings pass).
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.status != Status::Fail)
    }
}

/// Run every check, then the refresh.
pub fn run(req: &DoctorRequest) -> DoctorReport {
    let mut report = DoctorReport {
        checks: vec![
            check_git(OsStr::new("git")),
            check_path(req.path_var, req.current_exe),
        ],
        companions: Vec::new(),
        skills: None,
    };

    let config = match read_global_config(req.config_path) {
        Ok(config) => {
            let detail = req.config_path.display().to_string();
            report
                .checks
                .push(Check::new("config", Status::Pass, detail));
            Some(config)
        }
        Err(e) => {
            report
                .checks
                .push(Check::new("config", Status::Fail, e.to_string()));
            None
        }
    };
    let Some(config) = config else {
        if req.refresh {
            // An install from an empty config would remove every skill.
            report.checks.push(Check::new(
                "skills",
                Status::Fail,
                "refresh skipped: the config does not read",
            ));
        }
        return report;
    };

    let brains = read_brain_configs(&config);
    for (name, entry) in &config.brains {
        let path = Path::new(&entry.path);
        let (status, detail) = if !path.exists() {
            let detail = format!("brain path not found: {}", entry.path);
            (Status::Fail, detail)
        } else if !path.is_dir() {
            let detail = format!("brain path is not a directory: {}", entry.path);
            (Status::Fail, detail)
        } else if let Some(Err(e)) = brains.get(name) {
            (Status::Fail, e.to_string())
        } else {
            (Status::Pass, entry.path.clone())
        };
        report
            .checks
            .push(Check::new(format!("brain:{name}"), status, detail));
    }

    if !req.refresh {
        return report;
    }
    refresh_companions(&config, req.home, &mut report);
    let skills = install_skills(&InstallRequest {
        config: &config,
        brains: &brains,
        home: req.home,
        copilot_home: req.copilot_home,
    });
    report.checks.push(skills_check(&skills, &config, req.home));
    report.skills = Some(skills);
    report
}

/// `git`: `git --version` (`program`) must run and exit 0; v2 passed any
/// git that ran.
pub fn check_git(program: &OsStr) -> Check {
    let out = match Command::new(program).arg("--version").output() {
        Ok(out) => out,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Check::new(
                "git",
                Status::Fail,
                "git not found. Install: https://git-scm.com/downloads",
            );
        }
        Err(e) => return Check::new("git", Status::Fail, format!("cannot run git: {e}")),
    };
    if out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return Check::new("git", Status::Pass, stdout.trim());
    }
    let mut detail = format!("`git --version` failed ({})", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        detail.push_str(": ");
        detail.push_str(stderr.trim());
    }
    Check::new("git", Status::Fail, detail)
}

/// The file name a shell runs for `kluris`.
const PROGRAM: &str = if cfg!(windows) {
    "kluris.exe"
} else {
    "kluris"
};

/// `path`: the first `kluris` on `path_var` is `current_exe`. Otherwise a
/// warning naming the one that comes first (or saying none is on PATH).
pub fn check_path(path_var: Option<&OsStr>, current_exe: Option<&Path>) -> Check {
    let warn = |detail: String| Check::new("path", Status::Warn, detail);
    let Some(exe) = current_exe else {
        return warn("cannot tell where this kluris is installed".into());
    };
    let exe_dir = exe.parent().unwrap_or(exe);
    let first = std::env::split_paths(path_var.unwrap_or_default())
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(PROGRAM))
        .find(|candidate| is_program(candidate));
    match first {
        Some(found) if same_file(&found, exe) => {
            Check::new("path", Status::Pass, found.display().to_string())
        }
        Some(other) => {
            let other_dir = other.parent().unwrap_or(&other);
            warn(format!(
                "{} comes first on PATH, so shells and agents run it instead of this kluris ({}). \
                 Put {} before {} in PATH, or remove the other kluris (a Kluris 2 install: \
                 `pipx uninstall kluris`).",
                other.display(),
                exe.display(),
                exe_dir.display(),
                other_dir.display()
            ))
        }
        None => warn(format!(
            "this kluris ({}) is not on PATH, and agents' skills run `kluris`: add {} to PATH.",
            exe.display(),
            exe_dir.display()
        )),
    }
}

/// A file a shell would run: a regular file (through links), executable
/// on Unix.
fn is_program(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Whether two paths are the same file (links followed).
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(a), fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Windows paths compare without case.
        match (fs::canonicalize(a), fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => {
                a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
            }
            _ => false,
        }
    }
}

/// v2: refresh each known companion that is installed or referenced.
fn refresh_companions(config: &GlobalConfig, home: &Path, report: &mut DoctorReport) {
    let mut targets = companions::installed(home);
    targets.extend(companions::referenced(config));
    for name in companions::normalize(&targets) {
        let (status, detail, error) = match companions::refresh(&name, home) {
            Ok(()) => (Status::Pass, format!("refreshed {name}"), None),
            Err(e) => (
                Status::Fail,
                format!("refresh failed: {e}"),
                Some(e.to_string()),
            ),
        };
        report
            .checks
            .push(Check::new(format!("companion:{name}"), status, detail));
        report.companions.push(CompanionRefresh { name, error });
    }
}

/// `skills`: v2's detail, `<n> files written for <agents> agents across
/// <brains> brain(s)[ (failed: <folders>)]`, counted the v3 way.
fn skills_check(skills: &InstallReport, config: &GlobalConfig, home: &Path) -> Check {
    let mut detail = format!(
        "{} files written for {} agents across {} brain(s)",
        skills.total_written(),
        skills.agents.len(),
        config.brains.len()
    );
    let mut failed: Vec<String> = Vec::new();
    for dest in skills.destinations.iter().filter(|d| !d.failed.is_empty()) {
        let shown = home_relative(&dest.path, home);
        if !failed.contains(&shown) {
            failed.push(shown);
        }
    }
    if failed.is_empty() {
        return Check::new("skills", Status::Pass, detail);
    }
    detail.push_str(&format!(" (failed: {})", failed.join(", ")));
    Check::new("skills", Status::Fail, detail)
}

/// `path` with `/` separators, as `~/...` when it is under `home`.
pub fn home_relative(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => format!("~/{}", slashed(rel)),
        Err(_) => slashed(path),
    }
}

/// `path` with `/` between components (the JSON form, interview-01 #7).
pub fn slashed(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}
