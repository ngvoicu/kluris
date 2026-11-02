//! The sync services earlier releases installed (`kluris_core::upgrade::
//! services`): the hidden `kluris sync --watch` arm such a service runs, and
//! the one check `doctor` reports them in.

use std::path::PathBuf;

use kluris_core::doctor::{Check, Status};
use kluris_core::upgrade::services::{
    self, Env, Manager, Removed, Runner, SystemRunner, current_uid,
};

use crate::error::CliError;
use crate::resolve::home;

/// This user's services, asked through `runner`.
pub fn env(runner: &dyn Runner) -> Result<Env<'_>, CliError> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    Ok(Env {
        manager: Manager::current(),
        home: home()?,
        xdg_config_home: var("XDG_CONFIG_HOME").filter(|p| p.is_absolute()),
        local_app_data: var("LOCALAPPDATA"),
        uid: current_uid(),
        runner,
    })
}

/// `kluris sync --watch [--brain NAME]`, what an earlier release's service
/// runs: it removes that brain's service and log (every service without a
/// brain or with `all`), says so on stderr and always exits 0, which ends
/// launchd's and systemd's restarts. It reads no config: the brain need not
/// be registered.
pub fn watch(brain: Option<&str>) -> Result<(), CliError> {
    eprintln!("kluris sync --watch does not watch: run `kluris sync` to sync a brain.");
    let env = match env(&SystemRunner) {
        Ok(env) => env,
        Err(e) => {
            eprintln!("{}", e.message());
            return Ok(());
        }
    };
    let removed = match brain {
        Some(brain) if brain != "all" => vec![services::remove_one(brain, &env)],
        _ => services::sweep(&env),
    };
    for done in &removed {
        if !done.removed.is_empty() {
            eprintln!(
                "Removed the sync service {} an earlier release installed.",
                done.name
            );
        }
        if let Some(problem) = &done.problem {
            eprintln!(
                "Could not finish removing the sync service {}: {problem}",
                done.name
            );
        }
    }
    Ok(())
}

/// Remove every service an earlier release installed, reported as one
/// `sync-services` check: passed once each is gone, a warning naming what
/// failed; none without services.
pub fn services_check() -> Option<Check> {
    let env = env(&SystemRunner).ok()?;
    check(&services::sweep(&env))
}

fn check(removed: &[Removed]) -> Option<Check> {
    if removed.is_empty() {
        return None;
    }
    let names: Vec<&str> = removed.iter().map(|r| r.name.as_str()).collect();
    let problems: Vec<String> = removed
        .iter()
        .filter_map(|r| r.problem.as_ref().map(|p| format!("{}: {p}", r.name)))
        .collect();
    let detail = format!(
        "removed the sync services an earlier release installed: {}",
        names.join(", ")
    );
    Some(if problems.is_empty() {
        Check {
            name: "sync-services".into(),
            status: Status::Pass,
            detail,
        }
    } else {
        Check {
            name: "sync-services".into(),
            status: Status::Warn,
            detail: format!("{detail}; not all of it went: {}", problems.join("; ")),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use kluris_core::upgrade::services::Ran;

    /// Lists one task, `\Kluris\sync-acme`, and refuses to delete it in
    /// another language's words; every other call answers 0.
    struct Refusing;

    impl Runner for Refusing {
        fn run(&self, _program: &str, args: &[&str]) -> io::Result<Ran> {
            let (code, stdout, stderr) = match args {
                ["/Query", "/FO", "CSV", "/NH"] => {
                    (0, "\"\\Kluris\\sync-acme\",\"N/A\",\"Bereit\"\r\n", "")
                }
                ["/Delete", ..] => (1, "", "FEHLER: Zugriff verweigert.\r\n"),
                _ => (0, "", ""),
            };
            Ok(Ran {
                code: Some(code),
                stdout: stdout.into(),
                stderr: stderr.into(),
            })
        }
    }

    /// Whatever words Windows gives the refusal, `doctor`'s warning says
    /// how to remove the task.
    #[test]
    fn a_task_doctor_cannot_delete_is_a_warning_that_says_how_to_remove_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let env = Env {
            manager: Manager::TaskScheduler,
            home: tmp.path().to_path_buf(),
            xdg_config_home: None,
            local_app_data: Some(tmp.path().join("lad")),
            uid: 0,
            runner: &Refusing,
        };
        let check = check(&services::sweep(&env)).unwrap();
        assert_eq!(check.status, Status::Warn);
        assert_eq!(
            check.detail,
            "removed the sync services an earlier release installed: \\Kluris\\sync-acme; \
             not all of it went: \\Kluris\\sync-acme: `schtasks /Delete /TN \\Kluris\\sync-acme \
             /F`: FEHLER: Zugriff verweigert. (run `kluris doctor` once from a terminal opened \
             with \"Run as administrator\" to remove it)"
        );
    }
}
