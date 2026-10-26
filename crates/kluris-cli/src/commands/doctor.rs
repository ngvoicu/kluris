//! `kluris doctor [--no-refresh] [--json]` (research-03 §1.13; the checks
//! are `kluris_core::doctor`'s).

use kluris_core::agents::install::{DestinationReport, InstallReport};
use kluris_core::doctor::{
    self, Check, CompanionRefresh, DoctorReport, DoctorRequest, Status, home_relative,
};

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
    // Old binaries a Windows self-update left beside this one.
    if let Some(dir) = current_exe.as_deref().and_then(|exe| exe.parent()) {
        kluris_core::dist::swap::remove_stale_old(dir);
    }
    let report = doctor::run(&DoctorRequest {
        config_path: &config_path,
        home: &home,
        copilot_home: copilot_home.as_deref(),
        path_var: path_var.as_deref(),
        current_exe: current_exe.as_deref(),
        refresh: !no_refresh,
    });
    if as_json {
        human::print(&json::to_line(&report_json(&report)));
    } else {
        human::print(&report_text(&report, &home, Painter::stdout()));
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

/// `{"ok", "checks", "companions", "skills"}`: v2's first three, then the
/// install report (`null` when it did not run).
fn report_json(report: &DoctorReport) -> Json {
    let checks = report.checks.iter().map(check_json).collect();
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

/// `{"name", "passed", "detail"}`, plus `"warning": true` for a warning
/// (which passes).
fn check_json(check: &Check) -> Json {
    let mut fields = vec![
        ("name".into(), Json::Str(check.name.clone())),
        ("passed".into(), Json::Bool(check.status != Status::Fail)),
        ("detail".into(), Json::Str(check.detail.clone())),
    ];
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
