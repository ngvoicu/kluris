//! `kluris sync [--brain NAME|all] [--no-verify] [--json]`: pull and push
//! each brain through the sync engine.
//!
//! git >= 2.38 is checked once, before any brain. A brain the engine does
//! not open (nested in a larger repo, not a work tree, no kluris.yml) is a
//! clean error with `--brain NAME`; under `--brain all` its `sync` object is
//! left as it is, `refused`, and the other brains still sync. Each brain's
//! entry is `{name, sync}`; in text, stdout says what sync did and stderr
//! has one line per brain that has something to say. Exit 1 when any brain
//! has a conflict or was left as it is for anything but having no network
//! or no upstream.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::claim::COMMAND_BOUND;
use kluris_core::sync::engine::{self, check_git};
use kluris_core::sync::{LeftAsIs, LeftReason, NETWORK_TIME_LIMIT, SyncReport, SyncRequest};

use crate::commands::dream::today;
use crate::commands::resolve::command;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One brain's line for stderr, `kluris: NAME: ...`; `None` when it has
/// nothing to say.
fn summary(name: &str, report: &SyncReport) -> Option<String> {
    let mut parts = Vec::new();
    if report.pulled > 0 {
        parts.push(format!(
            "pulled {}",
            plural(report.pulled, "commit", "commits")
        ));
    }
    if !report.auto_merged.is_empty() {
        parts.push(format!("auto-merged {}", report.auto_merged.join(", ")));
    }
    if !report.conflicts.is_empty() {
        let paths: Vec<&str> = report.conflicts.iter().map(|c| c.path.as_str()).collect();
        parts.push(format!(
            "{} ({})",
            plural(report.conflicts.len(), "conflict", "conflicts"),
            paths.join(", ")
        ));
    }
    if report.behind_conflict > 0 {
        parts.push(format!(
            "{} behind the conflict",
            plural(report.behind_conflict, "newer commit", "newer commits")
        ));
    }
    if let Some(left) = &report.left_as_is {
        parts.push(match (left.reason, &left.detail) {
            (LeftReason::Uncommitted, _) => {
                format!("left as is (uncommitted): {}", left.paths.join(", "))
            }
            (reason, Some(detail)) => format!("left as is ({}): {detail}", reason.as_str()),
            (reason, None) => format!("left as is ({})", reason.as_str()),
        });
    }
    if report.pushed {
        parts.push("pushed".into());
    } else if report.to_push > 0 {
        parts.push(format!(
            "{} to push",
            plural(report.to_push, "commit", "commits")
        ));
    }
    (!parts.is_empty()).then(|| format!("kluris: {name}: {}\n", parts.join("; ")))
}

/// One brain's short text block.
fn text(name: &str, report: &SyncReport) -> String {
    let done = |report: &SyncReport| {
        let mut done = Vec::new();
        if report.pulled > 0 {
            done.push("pulled");
        }
        if report.pushed {
            done.push("pushed");
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
    if !report.conflicts.is_empty() {
        let mut out = format!(
            "{name}: {} conflict(s), nothing changed; settle them with `kluris resolve`:\n",
            report.conflicts.len()
        );
        for c in &report.conflicts {
            out.push_str(&format!("  {}: {}\n", c.kind.as_str(), c.path));
            out.push_str(&format!("    {}\n", command(name, &c.path, &c.merge)));
        }
        return out;
    }
    let Some(left) = &report.left_as_is else {
        return done(report);
    };
    match left.reason {
        LeftReason::Uncommitted => format!(
            "{name}: not synced, uncommitted changes are in the way: {}; commit or stash them, then run kluris sync again\n",
            left.paths.join(", ")
        ),
        LeftReason::Offline if report.pulled == 0 => format!("{name}: offline, nothing changed\n"),
        LeftReason::Offline => done(report).replacen('\n', ", offline, not pushed\n", 1),
        LeftReason::NoUpstream => format!("{name}: no upstream, nothing changed; {left}\n"),
        _ => format!("{name}: refused: {left}\n"),
    }
}

/// Whether the brain makes `kluris sync` fail.
fn failed(report: &SyncReport) -> bool {
    !report.conflicts.is_empty()
        || report
            .left_as_is
            .as_ref()
            .is_some_and(|left| left.reason.fails())
}

pub fn run(brain: Option<&str>, no_verify: bool, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, true, as_json)?;
    let git = SystemGit::new();
    check_git(&git).map_err(|e| CliError::new(e.to_string()))?;
    let today = today()?;
    // `--brain all`, or `all` picked from the interactive list.
    let all = brain == Some("all") || brains.len() > 1;
    let mut reports = Vec::new();
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        let outcome = match read_brain_config(Path::new(&root)) {
            Err(e) if e.kind == ConfigErrorKind::NotFound => {
                Err(format!("it has no kluris.yml: {}", e.path.display()))
            }
            Err(e) => Err(e.to_string()),
            Ok(config) => {
                let request = SyncRequest {
                    no_verify,
                    maps: DreamOptions {
                        name: &config.name,
                        description: &config.description,
                        today,
                    },
                    network_time_limit: NETWORK_TIME_LIMIT,
                    tree_bound: COMMAND_BOUND,
                };
                engine::open(Path::new(&root), &git, &request)
                    .map(|ctx| engine::sync(&ctx))
                    .map_err(|e| e.to_string())
            }
        };
        let report = match outcome {
            Ok(report) => report,
            Err(why) if all => SyncReport {
                left_as_is: Some(LeftAsIs::because(LeftReason::Refused, why)),
                ..SyncReport::default()
            },
            Err(why) => return Err(CliError::new(format!("Brain {}: {why}", brain.name))),
        };
        reports.push((brain.name.clone(), report));
    }
    if as_json {
        let entries = reports
            .iter()
            .map(|(name, report)| {
                Json::Object(vec![
                    ("name".into(), Json::Str(name.clone())),
                    ("sync".into(), report.to_json()),
                ])
            })
            .collect();
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(entries)),
        ])));
    } else {
        let out: String = reports.iter().map(|(name, r)| text(name, r)).collect();
        human::print(&out);
        let lines: String = reports
            .iter()
            .filter_map(|(name, r)| summary(name, r))
            .collect();
        eprint!("{lines}");
    }
    if reports.iter().any(|(_, report)| failed(report)) {
        Err(CliError::exit_only())
    } else {
        Ok(())
    }
}
