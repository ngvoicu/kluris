//! `kluris doctor [--no-refresh] [--json]` (research-03 §1.13; the checks
//! are `kluris_core::doctor`'s). Every registered brain is read through its
//! session: a brain's check carries what its sync did (`sync`), and warns
//! when the brain was left as it is or a conflict waits; the sync never
//! fails it.

use std::path::Path;

use kluris_core::agents::install::{DestinationReport, InstallReport};
use kluris_core::config::read_global_config;
use kluris_core::doctor::{
    self, Check, CompanionRefresh, DoctorReport, DoctorRequest, Status, home_relative,
};
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::SyncReport;
use kluris_core::sync::session::Intent;

use crate::brains::{self, Target};
use crate::error::CliError;
use crate::output::human::{self, Painter, Style};
use crate::output::json::{self, Json};
use crate::resolve::{config_path, copilot_home, home};
use crate::skills::destination_json;

pub fn run(no_refresh: bool, as_json: bool) -> Result<(), CliError> {
    let config_path = config_path()?;
    let home = home()?;
    let copilot_home = copilot_home();
    let path_var = std::env::var_os("PATH");
    let current_exe = std::env::current_exe().ok();
    // Old binaries a Windows self-update left beside this one, and staged
    // copies an interrupted update left (over an hour old).
    if let Some(dir) = current_exe.as_deref().and_then(|exe| exe.parent()) {
        kluris_core::dist::swap::remove_swap_leftovers(dir);
    }
    // Every registered brain whose folder is there.
    let (names, targets): (Vec<String>, Vec<Target>) = read_global_config(&config_path)
        .map(|config| {
            config
                .brains
                .iter()
                .filter(|(_, entry)| Path::new(&py_path_str(&entry.path)).is_dir())
                .map(|(name, entry)| {
                    let target = Target::new(&entry.path, name, &entry.description);
                    (name.clone(), target)
                })
                .unzip()
        })
        .unwrap_or_default();
    let opened = brains::open_today(&targets, Intent::Read)?;
    let mut report = doctor::run(&DoctorRequest {
        config_path: &config_path,
        home: &home,
        copilot_home: copilot_home.as_deref(),
        path_var: path_var.as_deref(),
        current_exe: current_exe.as_deref(),
        refresh: !no_refresh,
    });
    if !no_refresh {
        // The sync services earlier releases installed go with the refresh.
        report
            .checks
            .extend(crate::commands::upgrade::services_check());
    }
    let closed = brains::close(opened);
    let synced: Vec<(String, Option<SyncReport>)> = names
        .into_iter()
        .zip(closed)
        .map(|(name, closed)| (name, closed.report))
        .collect();
    for (name, sync) in &synced {
        if let Some(sync) = sync {
            warn_for(&mut report.checks, name, sync);
        }
    }
    if as_json {
        human::print(&json::to_line(&report_json(&report, &synced)));
    } else {
        human::print(&report_text(&report, &home, Painter::stdout()));
        brains::print_lines(synced.iter().map(|(n, s)| (n.as_str(), s.as_ref())));
    }
    if report.ok() {
        Ok(())
    } else {
        Err(CliError::exit_only())
    }
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

/// The check of brain `name` warns when its sync left it as it is or a
/// conflict waits, its detail then the path, `; ` and what the sync said.
/// A check that fails stays failed.
fn warn_for(checks: &mut [Check], name: &str, sync: &SyncReport) {
    let stuck = sync.left_as_is.is_some() || !sync.conflicts.is_empty();
    let check = checks
        .iter_mut()
        .find(|c| c.name == format!("brain:{name}"));
    if let (true, Some(check), Some(said)) = (stuck, check, brains::summary(sync))
        && check.status == Status::Pass
    {
        check.status = Status::Warn;
        check.detail = format!("{}; {said}", check.detail);
    }
}

/// `{"ok", "checks", "companions", "skills"}`: v2's first three, then the
/// install report (`null` when it did not run).
fn report_json(report: &DoctorReport, synced: &[(String, Option<SyncReport>)]) -> Json {
    let checks = report
        .checks
        .iter()
        .map(|check| {
            let sync = synced
                .iter()
                .find(|(name, _)| check.name == format!("brain:{name}"))
                .and_then(|(_, sync)| sync.as_ref());
            check_json(check, sync)
        })
        .collect();
    let companions = report.companions.iter().map(companion_json).collect();
    Json::Object(vec![
        ("ok".into(), Json::Bool(report.ok())),
        ("checks".into(), Json::Array(checks)),
        ("companions".into(), Json::Array(companions)),
        (
            "skills".into(),
            report.skills.as_ref().map_or(Json::Null, skills_json),
        ),
    ])
}

/// `{"name", "passed", "detail"}`, then a brain's `sync` object when its
/// sync has something to say, then `"warning": true` for a warning (which
/// passes).
fn check_json(check: &Check, sync: Option<&SyncReport>) -> Json {
    let mut fields = vec![
        ("name".into(), Json::Str(check.name.clone())),
        ("passed".into(), Json::Bool(check.status != Status::Fail)),
        ("detail".into(), Json::Str(check.detail.clone())),
    ];
    fields.extend(brains::sync_field(sync));
    if check.status == Status::Warn {
        fields.push(("warning".into(), Json::Bool(true)));
    }
    Json::Object(fields)
}

/// v2: `{"name", "refreshed"[, "error"]}`.
fn companion_json(refresh: &CompanionRefresh) -> Json {
    let mut fields = vec![
        ("name".into(), Json::Str(refresh.name.clone())),
        ("refreshed".into(), Json::Bool(refresh.error.is_none())),
    ];
    if let Some(error) = &refresh.error {
        fields.push(("error".into(), Json::Str(error.clone())));
    }
    Json::Object(fields)
}

/// `{"agents", "destinations": [{"path", "role", "alias_of", "written",
/// "removed", "kept", "failed"}], "brain_errors": [{"brain", "error"}]}`.
fn skills_json(skills: &InstallReport) -> Json {
    let destinations = skills.destinations.iter().map(destination_json).collect();
    let brain_errors = skills
        .brain_errors
        .iter()
        .map(|(brain, error)| {
            Json::Object(vec![
                ("brain".into(), Json::Str(brain.clone())),
                ("error".into(), Json::Str(error.clone())),
            ])
        })
        .collect();
    Json::Object(vec![
        ("agents".into(), strings(&skills.agents)),
        ("destinations".into(), Json::Array(destinations)),
        ("brain_errors".into(), Json::Array(brain_errors)),
    ])
}

/// v2's `  PASS|FAIL <name>: <detail>` per check (`WARN` for a warning);
/// under the skills line, one line per folder something happened in.
fn report_text(report: &DoctorReport, home: &std::path::Path, painter: Painter) -> String {
    let mut out = String::new();
    for check in &report.checks {
        let label = match check.status {
            Status::Pass => painter.paint(Style::Green, "PASS"),
            Status::Warn => painter.paint(Style::Yellow, "WARN"),
            Status::Fail => painter.paint(Style::Red, "FAIL"),
        };
        out.push_str(&format!("  {label} {}: {}\n", check.name, check.detail));
        if check.name == "skills"
            && let Some(skills) = &report.skills
        {
            for dest in &skills.destinations {
                if let Some(line) = destination_line(dest, home) {
                    out.push_str(&line);
                }
            }
        }
    }
    out
}

/// `    ~/.agents/skills: written a, b; removed c; kept d; failed: <message>`,
/// or `    ~/.codex/skills: same folder as ~/.agents/skills`.
fn destination_line(dest: &DestinationReport, home: &std::path::Path) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(first) = &dest.alias_of {
        parts.push(format!("same folder as {}", home_relative(first, home)));
    }
    if !dest.written.is_empty() {
        parts.push(format!("written {}", dest.written.join(", ")));
    }
    if !dest.removed.is_empty() {
        parts.push(format!("removed {}", dest.removed.join(", ")));
    }
    if !dest.kept.is_empty() {
        parts.push(format!("kept {}", dest.kept.join(", ")));
    }
    if !dest.failed.is_empty() {
        parts.push(format!("failed: {}", dest.failed.join("; ")));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "    {}: {}\n",
        home_relative(&dest.path, home),
        parts.join("; ")
    ))
}
