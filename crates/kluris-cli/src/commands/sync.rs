//! `kluris sync [--brain NAME|all] [--no-verify] [--json]`: pull and push
//! each brain through an explicit session (`brains::open`, several brains
//! at once for `--brain all`), closed once every brain is open.
//!
//! git >= 2.38 is checked once, before any brain. A brain the session
//! cannot sync (nested in a larger repo, not a work tree, a shallow clone,
//! no kluris.yml) is a clean error with `--brain NAME`; under `--brain all`
//! its `sync` object is left as it is, `refused`, and the other brains
//! still sync. A brain with no remote is left as it is, `no_upstream`, with
//! the commands that add one. Each brain's entry is `{name, sync}`; in
//! text, stdout says what sync did and stderr has one line per brain that
//! has something to say. Exit 1 when any brain has a conflict or was left
//! as it is for anything but having no network or no upstream.

use std::path::{Path, PathBuf};

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::engine::check_git;
use kluris_core::sync::session::Intent;
use kluris_core::sync::{ConflictKind, LeftAsIs, LeftReason, SyncReport};

use crate::brains::{self, Target};
use crate::commands::dream::today;
use crate::commands::resolve::{command, shell_word};
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One brain's short text block.
fn text(name: &str, report: &SyncReport) -> String {
    let done = |report: &SyncReport| {
        let mut done = Vec::new();
        if report.pulled > 0 {
            done.push("pulled".to_string());
        }
        if report.pushed {
            done.push("pushed".to_string());
            // A commit made while the push ran.
            if report.to_push > 0 {
                done.push(format!(
                    "{} still to push",
                    plural(report.to_push, "commit", "commits")
                ));
            }
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
        // Every conflict belongs to the one waiting merge, whose id the
        // settle commands name; the session summary says the rest on
        // stderr.
        let merge = &report.conflicts[0].merge;
        let mut out = format!(
            "{name}: {} in merge {merge}:\n",
            plural(report.conflicts.len(), "conflict", "conflicts")
        );
        // `kluris commit --merge` settles text paths from the work tree;
        // a `structure` path takes a side with `kluris resolve` alone. A
        // merge holding a structure or a binary path is shown settling
        // with `kluris resolve`, path by path, which settles every kind.
        let all_text = report
            .conflicts
            .iter()
            .all(|c| !c.binary && c.kind != ConflictKind::Structure);
        for c in &report.conflicts {
            out.push_str(&format!("  {}: {}\n", c.kind.as_str(), c.path));
            if !all_text {
                out.push_str(&format!("    {}\n", command(name, &c.path, &c.merge)));
            }
        }
        if all_text {
            let paths = report
                .conflicts
                .iter()
                .map(|c| shell_word(&c.path))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(
                "    write the version you want to each file (a deleted file is a deletion), then:\n",
            );
            out.push_str(&format!(
                "    kluris commit --brain {name} --merge {merge} --paths {paths}\n"
            ));
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
        LeftReason::Busy => format!("{name}: busy: {left}\n"),
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

/// `kluris sync --background --path <root>...`: push each brain's waiting
/// commits, detached from the command that started it; what happened goes
/// to each brain's log and record, never to this process's output.
pub fn background(roots: &[PathBuf]) -> Result<(), CliError> {
    kluris_core::sync::background::run(roots, &SystemGit::new());
    Ok(())
}

pub fn run(brain: Option<&str>, no_verify: bool, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, true, as_json)?;
    check_git(&SystemGit::new()).map_err(|e| CliError::new(e.to_string()))?;
    // `--brain all`, or `all` picked from the interactive list.
    let all = brain == Some("all") || brains.len() > 1;
    // Each brain's report, or why it cannot be synced; the brains with a
    // kluris.yml are opened together.
    let mut outcomes: Vec<Result<Option<SyncReport>, String>> = Vec::new();
    let mut targets = Vec::new();
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        match read_brain_config(Path::new(&root)) {
            Err(e) if e.kind == ConfigErrorKind::NotFound => {
                outcomes.push(Err(format!("it has no kluris.yml: {}", e.path.display())));
            }
            Err(e) => outcomes.push(Err(e.to_string())),
            Ok(config) => {
                targets.push(Target::new(
                    &brain.entry.path,
                    &config.name,
                    &config.description,
                ));
                outcomes.push(Ok(None));
            }
        }
    }
    let opened = brains::open(&targets, Intent::Explicit, today()?, no_verify);
    let mut closed = brains::close(opened).into_iter();
    for outcome in outcomes.iter_mut().filter(|o| o.is_ok()) {
        let Some(closed) = closed.next() else { break };
        *outcome = match closed.alone.as_ref().and_then(|why| why.refusal()) {
            Some(why) => Err(why),
            None => Ok(closed.report),
        };
    }
    let mut reports = Vec::new();
    for (brain, outcome) in brains.iter().zip(outcomes) {
        let report = match outcome {
            Ok(report) => report.unwrap_or_default(),
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
        brains::print_lines(reports.iter().map(|(name, r)| (name.as_str(), Some(r))));
    }
    if reports.iter().any(|(_, report)| failed(report)) {
        Err(CliError::exit_only())
    } else {
        Ok(())
    }
}
