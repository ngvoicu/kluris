//! `kluris sync [--brain NAME|all] [--no-verify] [--json]` (new in v3.1,
//! SPEC item 46): fetch, merge and push each brain.
//!
//! git >= 2.38 is checked once, before any brain. A brain that is refused
//! (nested in a larger repo, not a work tree, an operation or a sync
//! conflict in progress) or whose sync fails is a clean error with `--brain
//! NAME`; under `--brain all` its entry says why (`refused`) and the other
//! brains still sync. Exit 1 when any brain has a conflict, is blocked by
//! uncommitted changes, or was refused.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::{NETWORK_TIME_LIMIT, SyncReport, SyncRequest, check_git, sync};

use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

fn opt(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

/// One brain's entry: the report, or why it was refused.
pub(crate) fn entry(name: &str, outcome: &Result<SyncReport, String>) -> Json {
    let empty = SyncReport::default();
    let (report, refused) = match outcome {
        Ok(report) => (report, Json::Null),
        Err(why) => (&empty, Json::Str(why.clone())),
    };
    let conflicts = report
        .conflicts
        .iter()
        .map(|c| {
            Json::Object(vec![
                ("path".into(), Json::Str(c.path.clone())),
                ("kind".into(), Json::Str(c.kind.as_str().into())),
                ("base".into(), opt(&c.base)),
                ("ours".into(), opt(&c.ours)),
                ("theirs".into(), opt(&c.theirs)),
            ])
        })
        .collect();
    Json::Object(vec![
        ("name".into(), Json::Str(name.into())),
        ("fetched".into(), Json::Bool(report.fetched)),
        ("merged".into(), Json::Bool(report.merged)),
        ("pushed".into(), Json::Bool(report.pushed)),
        ("auto_merged".into(), strings(&report.auto_merged)),
        ("conflicts".into(), Json::Array(conflicts)),
        ("skipped_dirty".into(), strings(&report.skipped_dirty)),
        ("offline".into(), Json::Bool(report.offline)),
        ("blocked".into(), strings(&report.blocked)),
        ("refused".into(), refused),
    ])
}

/// One brain's short text block.
pub(crate) fn text(name: &str, outcome: &Result<SyncReport, String>) -> String {
    let report = match outcome {
        Ok(report) => report,
        Err(why) => return format!("{name}: refused: {why}\n"),
    };
    let mut out = if !report.conflicts.is_empty() {
        let mut out = format!(
            "{name}: {} conflict(s), nothing changed; settle them with `kluris resolve`:\n",
            report.conflicts.len()
        );
        for c in &report.conflicts {
            out.push_str(&format!("  {}: {}\n", c.kind.as_str(), c.path));
        }
        out
    } else if !report.blocked.is_empty() {
        format!(
            "{name}: not synced, uncommitted changes are in the way: {}; commit or stash them, then run kluris sync again\n",
            report.blocked.join(", ")
        )
    } else if report.offline && !report.merged {
        format!("{name}: offline, nothing changed\n")
    } else {
        let mut done = Vec::new();
        if report.merged {
            done.push("pulled");
        }
        if report.pushed {
            done.push("pushed");
        }
        if report.offline {
            done.push("offline, not pushed");
        }
        let mut out = if done.is_empty() {
            format!("{name}: up to date\n")
        } else {
            format!("{name}: {}\n", done.join(", "))
        };
        for path in &report.auto_merged {
            out.push_str(&format!("  auto-merged: {path}\n"));
        }
        out
    };
    if report.blocked.is_empty() && !report.skipped_dirty.is_empty() {
        out.push_str(&format!(
            "  left as is (uncommitted): {}\n",
            report.skipped_dirty.join(", ")
        ));
    }
    out
}

pub fn run(brain: Option<&str>, no_verify: bool, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, true, as_json)?;
    let git = SystemGit::new();
    check_git(&git).map_err(|e| CliError::new(e.to_string()))?;
    let today = today()?;
    // `--brain all`, or `all` picked from the interactive list.
    let all = brain == Some("all") || brains.len() > 1;
    let mut outcomes = Vec::new();
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        let outcome = match read_brain_config(Path::new(&root)) {
            Err(e) if e.kind == ConfigErrorKind::NotFound => {
                Err(format!("it has no kluris.yml: {}", e.path.display()))
            }
            Err(e) => Err(e.to_string()),
            Ok(config) => sync(
                Path::new(&root),
                &git,
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
        };
        if let (false, Err(why)) = (all, &outcome) {
            return Err(CliError::new(format!("Brain {}: {why}", brain.name)));
        }
        outcomes.push((brain.name.clone(), outcome));
    }
    let failed = outcomes.iter().any(|(_, outcome)| match outcome {
        Ok(report) => !report.conflicts.is_empty() || !report.blocked.is_empty(),
        Err(_) => true,
    });
    if as_json {
        let entries = outcomes.iter().map(|(name, o)| entry(name, o)).collect();
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(entries)),
        ])));
    } else {
        let out: String = outcomes.iter().map(|(name, o)| text(name, o)).collect();
        human::print(&out);
    }
    if failed {
        Err(CliError::exit_only())
    } else {
        Ok(())
    }
}
