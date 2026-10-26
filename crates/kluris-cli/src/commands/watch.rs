//! `kluris sync --watch [--brain NAME|all] [--interval N] [--no-verify]
//! [--json]` and `kluris sync --install-service|--uninstall-service --brain
//! NAME [--json]` (new in v3.1, SPEC items 51/52).
//!
//! The watch runs in the foreground until SIGINT/Ctrl-C or SIGTERM: one
//! `kluris sync` per change (`kluris_core::sync::watch`), each printed as
//! one line (with `--json`, the document `kluris sync --json` prints). git
//! runs in process groups of their own, so a Ctrl-C at the terminal lets a
//! sync in progress finish; the watch then exits 0. `--log-file` (the
//! Windows service's, hidden) appends the lines to a file instead.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::service::{self, InstallOutcome, ServiceEnv};
use kluris_core::sync::watch::{
    PROBE_TIME_LIMIT, Report, SystemClock, WatchRequest, Watched, watch,
};
use kluris_core::sync::{NETWORK_TIME_LIMIT, SyncReport, SyncRequest, check_git, sync};

use crate::commands::dream::today;
use crate::commands::sync::{entry, text};
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::platform::service::{STOP, SystemRunner, catch_stop_signals, env as service_env};
use crate::resolve::resolve_brains;

pub struct Options<'a> {
    pub brain: Option<&'a str>,
    pub interval: Duration,
    pub no_verify: bool,
    pub json: bool,
    pub log_file: Option<&'a str>,
}

/// Where the lines go: stdout, or the end of the log file.
enum Out {
    Stdout,
    File(fs::File),
}

impl Out {
    fn open(log_file: Option<&str>) -> Result<Out, CliError> {
        let Some(path) = log_file else {
            return Ok(Out::Stdout);
        };
        let path = Path::new(path);
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)
                .map_err(|e| CliError::new(format!("Cannot create {}: {e}", dir.display())))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(Out::File)
            .map_err(|e| CliError::new(format!("Cannot open {}: {e}", path.display())))
    }

    fn line(&mut self, text: &str) {
        match self {
            Out::Stdout => {
                let mut out = io::stdout().lock();
                let _ = writeln!(out, "{text}");
                let _ = out.flush();
            }
            Out::File(file) => {
                let _ = writeln!(file, "{text}");
                let _ = file.flush();
            }
        }
    }
}

/// Local time, for the lines.
fn stamp() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string()
}

/// `kluris sync`'s text for one brain as one line: the lines under a line
/// that ends in `:` follow it after a space, the others after `; `.
fn one_line(block: &str) -> String {
    let mut out = String::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(if out.ends_with(':') { " " } else { "; " });
        }
        out.push_str(line);
    }
    out
}

/// One brain's sync, as `kluris sync --brain NAME` runs it.
fn sync_brain(root: &str, git: &SystemGit, no_verify: bool) -> Result<SyncReport, String> {
    let today = today().map_err(|e| e.message().to_string())?;
    match read_brain_config(Path::new(root)) {
        Err(e) if e.kind == ConfigErrorKind::NotFound => {
            Err(format!("it has no kluris.yml: {}", e.path.display()))
        }
        Err(e) => Err(e.to_string()),
        Ok(config) => sync(
            Path::new(root),
            git,
            &SyncRequest {
                no_verify,
                maps: DreamOptions {
                    name: &config.name,
                    description: &config.description,
                    today,
                },
                network_time_limit: NETWORK_TIME_LIMIT,
            },
        )
        .map_err(|e| e.to_string()),
    }
}

pub fn run(options: &Options) -> Result<(), CliError> {
    let mut out = Out::open(options.log_file)?;
    let result = watch_brains(options, &mut out);
    if let (Err(e), Out::File(_)) = (&result, &out) {
        let message = e.message().to_string();
        if !message.is_empty() {
            out.line(&format!("{} Error: {message}", stamp()));
        }
    }
    result
}

fn watch_brains(options: &Options, out: &mut Out) -> Result<(), CliError> {
    let brains = resolve_brains(options.brain, true, options.json)?;
    let git = SystemGit::new().shielded();
    check_git(&git).map_err(|e| CliError::new(e.to_string()))?;
    catch_stop_signals();
    let roots: Vec<(String, PathBuf)> = brains
        .iter()
        .map(|b| (b.name.clone(), PathBuf::from(py_path_str(&b.entry.path))))
        .collect();
    let watched: Vec<Watched> = roots
        .iter()
        .map(|(name, root)| Watched { name, root })
        .collect();
    let as_json = options.json;
    watch(
        &watched,
        &git,
        &SystemClock,
        &STOP,
        &WatchRequest {
            interval: options.interval,
            probe_time_limit: PROBE_TIME_LIMIT,
        },
        &mut |brain| sync_brain(&brain.root.to_string_lossy(), &git, options.no_verify),
        &mut |brain, report| {
            let no_upstream = Ok(SyncReport {
                offline: true,
                ..SyncReport::default()
            });
            let outcome = match report {
                Report::Synced(outcome) => outcome,
                Report::NoUpstream => &no_upstream,
            };
            if as_json {
                out.line(
                    json::to_line(&Json::Object(vec![
                        ("ok".into(), Json::Bool(true)),
                        (
                            "brains".into(),
                            Json::Array(vec![entry(brain.name, outcome)]),
                        ),
                    ]))
                    .trim_end_matches('\n'),
                );
            } else {
                let line = match report {
                    Report::NoUpstream => format!(
                        "{}: not synced: its branch has no upstream (git push -u origin <branch> sets one)",
                        brain.name
                    ),
                    Report::Synced(outcome) => one_line(&text(brain.name, outcome)),
                };
                out.line(&format!("{} {line}", stamp()));
            }
        },
    );
    Ok(())
}

fn one_brain(flag: &str, brain: Option<&str>) -> Result<(), CliError> {
    if brain == Some("all") {
        return Err(CliError::new(format!(
            "{flag} needs one brain: pass --brain NAME (one service per brain)."
        )));
    }
    Ok(())
}

fn path_json(path: &Option<PathBuf>) -> Json {
    path.as_ref()
        .map_or(Json::Null, |p| Json::Str(p.display().to_string()))
}

/// `kluris sync --install-service --brain NAME`.
pub fn install_service(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    one_brain("--install-service", brain)?;
    let brains = resolve_brains(brain, false, as_json)?;
    let Some(brain) = brains.first() else {
        return Ok(());
    };
    let env = service_env()?;
    let done = service::install(&env, &brain.name, &SystemRunner)
        .map_err(|e| CliError::new(format!("Brain {}: {e}", brain.name)))?;
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(brain.name.clone())),
            ("service".into(), Json::Str(done.name.clone())),
            ("definition".into(), path_json(&done.definition)),
            ("log".into(), path_json(&done.log)),
            ("changed".into(), Json::Bool(done.changed)),
        ])));
    } else {
        human::print(&install_text(&brain.name, &done, &env));
    }
    Ok(())
}

fn install_text(brain: &str, done: &InstallOutcome, env: &ServiceEnv) -> String {
    let mut out = if done.changed {
        format!(
            "Installed the sync service {} for {brain} and started it: it runs `kluris sync --watch --brain {brain}` from login on.\n",
            done.name
        )
    } else {
        format!(
            "The sync service {} for {brain} is already installed and running.\n",
            done.name
        )
    };
    if let Some(definition) = &done.definition {
        out.push_str(&format!("  definition: {}\n", definition.display()));
    }
    match &done.log {
        Some(log) => out.push_str(&format!("  log: {}\n", log.display())),
        None => out.push_str(&format!("  log: journalctl --user -u {}\n", done.name)),
    }
    out.push_str(&format!("  kluris: {}\n", env.kluris.display()));
    out
}

/// `kluris sync --uninstall-service --brain NAME`: the brain need not be
/// registered any more.
pub fn uninstall_service(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    one_brain("--uninstall-service", brain)?;
    let name = match brain {
        Some(name) => name.to_string(),
        None => {
            let brains = resolve_brains(None, false, as_json)?;
            match brains.first() {
                Some(b) => b.name.clone(),
                None => return Ok(()),
            }
        }
    };
    let env = service_env()?;
    let done = service::uninstall(&env, &name, &SystemRunner)
        .map_err(|e| CliError::new(format!("Brain {name}: {e}")))?;
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(name)),
            ("service".into(), Json::Str(done.name)),
            ("removed".into(), Json::Bool(done.removed)),
        ])));
    } else if done.removed {
        human::print(&format!(
            "Stopped and removed the sync service {}.\n",
            done.name
        ));
    } else {
        human::print(&format!("No sync service for {name} is installed.\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_of_lines_becomes_one() {
        assert_eq!(
            one_line(
                "acme: 2 conflict(s), nothing changed; settle them with `kluris resolve`:\n  content: a.md\n  parent: b.md\n"
            ),
            "acme: 2 conflict(s), nothing changed; settle them with `kluris resolve`: content: a.md; parent: b.md"
        );
        assert_eq!(
            one_line(
                "acme: pulled, pushed\n  auto-merged: a.md\n  left as is (uncommitted): b.md\n"
            ),
            "acme: pulled, pushed; auto-merged: a.md; left as is (uncommitted): b.md"
        );
        assert_eq!(one_line("acme: up to date\n"), "acme: up to date");
    }
}
