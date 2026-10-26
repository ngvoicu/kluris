//! The agent skills every brain-registering command refreshes (v2's
//! `_do_install`), what those commands say when a skill could not be
//! written (T-106: stderr and a `--json` `skills` object; the clean case is
//! v2's output), and the note `create` and `register` print about the new
//! skill.

use std::path::Path;

use kluris_core::agents::install::{
    DestinationReport, DestinationRole, InstallReport, InstallRequest, install_skills,
    read_brain_configs,
};
use kluris_core::config::read_global_config;
use kluris_core::doctor::{home_relative, slashed};

use crate::error::CliError;
use crate::output::human::{Painter, Style};
use crate::output::json::Json;
use crate::resolve::{config_path, copilot_home};

/// Re-render every registered brain's agent skills under `home`.
pub fn refresh_skills(home: &Path) -> Result<InstallReport, CliError> {
    let config = read_global_config(&config_path()?).map_err(|e| CliError::new(e.to_string()))?;
    let brains = read_brain_configs(&config);
    let copilot_home = copilot_home();
    Ok(install_skills(&InstallRequest {
        config: &config,
        brains: &brains,
        home,
        copilot_home: copilot_home.as_deref(),
    }))
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

fn role(role: &DestinationRole) -> String {
    match role {
        DestinationRole::Universal => "universal".into(),
        DestinationRole::Agent(a) => format!("agent:{a}"),
        DestinationRole::Deselected(a) => format!("deselected:{a}"),
        DestinationRole::Migration => "migration".into(),
    }
}

/// One folder of the install report, as `doctor --json` lists it:
/// `{"path", "role", "alias_of", "written", "removed", "kept", "failed"}`.
pub fn destination_json(d: &DestinationReport) -> Json {
    Json::Object(vec![
        ("path".into(), Json::Str(slashed(&d.path))),
        ("role".into(), Json::Str(role(&d.role))),
        (
            "alias_of".into(),
            d.alias_of
                .as_ref()
                .map_or(Json::Null, |p| Json::Str(slashed(p))),
        ),
        ("written".into(), strings(&d.written)),
        ("removed".into(), strings(&d.removed)),
        ("kept".into(), strings(&d.kept)),
        ("failed".into(), strings(&d.failed)),
    ])
}

/// `doc` (a command's `--json` object) with `"skills": {"failed": [...],
/// "kept": [...]}` appended, the folders that failed and the folders that
/// kept a near miss, when there are any; otherwise `doc` as it is (v2's
/// output).
pub fn with_problems(doc: Json, report: &InstallReport) -> Json {
    let pick = |want: fn(&DestinationReport) -> bool| -> Vec<Json> {
        report
            .destinations
            .iter()
            .filter(|d| want(d))
            .map(destination_json)
            .collect()
    };
    let failed = pick(|d| !d.failed.is_empty());
    let kept = pick(|d| !d.kept.is_empty());
    match doc {
        Json::Object(mut fields) if !failed.is_empty() || !kept.is_empty() => {
            fields.push((
                "skills".into(),
                Json::Object(vec![
                    ("failed".into(), Json::Array(failed)),
                    ("kept".into(), Json::Array(kept)),
                ]),
            ));
            Json::Object(fields)
        }
        doc => doc,
    }
}

/// Where a failure message does not say how to fix it, the fix.
fn with_fix(message: &str) -> String {
    if message.contains("kluris doctor") {
        message.to_string()
    } else {
        format!("{message}; fix it, then run `kluris doctor`")
    }
}

/// The failed folders, one line per failure, for stderr; empty when none
/// failed.
pub fn failures_text(report: &InstallReport, home: &Path) -> String {
    let mut out = String::new();
    for dest in report.destinations.iter().filter(|d| !d.failed.is_empty()) {
        for message in &dest.failed {
            out.push_str(&format!(
                "  {}: {}\n",
                home_relative(&dest.path, home),
                with_fix(message)
            ));
        }
    }
    if out.is_empty() {
        out
    } else {
        format!("Kluris skills could not be written everywhere:\n{out}")
    }
}

/// Print [`failures_text`] on stderr.
pub fn print_failures(report: &InstallReport, home: &Path) {
    let text = failures_text(report, home);
    if !text.is_empty() {
        eprint!("{text}");
    }
}

/// v2's `_print_skill_usage` when `~/.agents/skills` got brain `name`'s
/// skill; otherwise that the skill was not installed, and why.
pub fn skill_note(painter: Painter, name: &str, report: &InstallReport) -> String {
    let skill = format!("kluris-{name}");
    let universal = report
        .destinations
        .iter()
        .find(|d| d.role == DestinationRole::Universal);
    if universal.is_some_and(|d| d.written.contains(&skill)) {
        return skill_usage(painter, name);
    }
    let title = painter.paint(
        Style::Red,
        &painter.bold(&format!("Agent skill not installed: {skill}")),
    );
    let why: Vec<String> = universal
        .map(|d| {
            d.failed
                .iter()
                .map(|m| format!("  {}\n", with_fix(m)))
                .collect()
        })
        .unwrap_or_default();
    let why = if why.is_empty() {
        "  Run `kluris doctor` to see why.\n".to_string()
    } else {
        why.concat()
    };
    format!("{title}\n{why}")
}

/// v2's `_print_skill_usage`.
pub fn skill_usage(painter: Painter, name: &str) -> String {
    let skill = format!("kluris-{name}");
    let title = painter.paint(
        Style::Green,
        &painter.bold(&format!("Agent skill installed: {skill}")),
    );
    format!(
        "{title}\n  Claude/Codex/Gemini/etc: /{skill}\n  Hermes: /skill {skill}  (or start with \
         `hermes -s {skill}`)\n  Created a new Hermes profile later? Run `kluris doctor` to \
         refresh Kluris skills.\n"
    )
}
