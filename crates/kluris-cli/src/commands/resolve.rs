//! `kluris resolve [--brain NAME] PATH --ours|--theirs|--file FILE
//! [--no-verify] [--json]` and `kluris resolve --abort` (new in v3.1, SPEC
//! item 48): settle an open sync conflict path by path.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::resolve::{Choice, ResolveReport, ResolveRequest, abort, resolve};
use kluris_core::sync::{NETWORK_TIME_LIMIT, SyncRequest};

use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

pub struct Options<'a> {
    pub brain: Option<&'a str>,
    pub path: Option<&'a str>,
    pub ours: bool,
    pub theirs: bool,
    pub file: Option<&'a str>,
    pub abort: bool,
    pub no_verify: bool,
    pub json: bool,
}

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn conflicts(n: usize) -> &'static str {
    if n == 1 { "conflict" } else { "conflicts" }
}

pub fn run(options: &Options) -> Result<(), CliError> {
    let picked = [options.ours, options.theirs, options.file.is_some()]
        .iter()
        .filter(|on| **on)
        .count();
    if options.abort && (options.path.is_some() || picked > 0) {
        return Err(CliError::new(
            "--abort takes no PATH and no --ours, --theirs or --file",
        ));
    }
    let brains = resolve_brains(options.brain, false, options.json)?;
    let Some(brain) = brains.first() else {
        return Err(CliError::new("No brain to resolve in"));
    };
    let root = py_path_str(&brain.entry.path);
    let git = SystemGit::new();
    let failed = |e: &dyn std::fmt::Display| CliError::new(format!("Brain {}: {e}", brain.name));

    if options.abort {
        let dropped = abort(Path::new(&root), &git).map_err(|e| failed(&e))?;
        if options.json {
            human::print(&json::to_line(&Json::Object(vec![
                ("ok".into(), Json::Bool(true)),
                ("brain".into(), Json::Str(brain.name.clone())),
                ("aborted".into(), Json::Bool(true)),
                ("dropped".into(), count(dropped)),
            ])));
        } else {
            human::print(&format!(
                "Dropped the open sync conflict in {} ({dropped} {}); nothing else changed.\n",
                brain.name,
                conflicts(dropped)
            ));
        }
        return Ok(());
    }

    match picked {
        0 => return Err(CliError::new("Give one of --ours, --theirs or --file")),
        1 => {}
        _ => return Err(CliError::new("Give only one of --ours, --theirs or --file")),
    }
    let Some(path) = options.path else {
        return Err(CliError::new("Give the conflicted PATH to resolve"));
    };
    let choice = match options.file {
        Some(file) => Choice::File(
            std::fs::read(file).map_err(|e| CliError::new(format!("Cannot read {file}: {e}")))?,
        ),
        None if options.ours => Choice::Ours,
        None => Choice::Theirs,
    };
    let config = read_brain_config(Path::new(&root)).map_err(|e| match e.kind {
        ConfigErrorKind::NotFound => CliError::new(format!(
            "Brain {} has no kluris.yml: {}",
            brain.name,
            e.path.display()
        )),
        _ => CliError::new(e.to_string()),
    })?;
    let report = resolve(
        Path::new(&root),
        &git,
        &ResolveRequest {
            path,
            choice,
            sync: SyncRequest {
                no_verify: options.no_verify,
                maps: DreamOptions {
                    name: &config.name,
                    description: &config.description,
                    today: today()?,
                },
                network_time_limit: NETWORK_TIME_LIMIT,
            },
        },
    )
    .map_err(|e| failed(&e))?;

    if options.json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(brain.name.clone())),
            ("path".into(), Json::Str(report.path.clone())),
            ("choice".into(), Json::Str(report.choice.into())),
            (
                "remaining".into(),
                Json::Array(report.remaining.iter().cloned().map(Json::Str).collect()),
            ),
            ("committed".into(), Json::Bool(report.committed)),
            (
                "commit".into(),
                report.commit.clone().map_or(Json::Null, Json::Str),
            ),
            ("pushed".into(), Json::Bool(report.pushed)),
        ])));
    } else {
        human::print(&text(&report));
    }
    Ok(())
}

/// One line per call.
fn text(r: &ResolveReport) -> String {
    let mut line = format!("Resolved {} ({})", r.path, r.choice);
    match &r.commit {
        None => line.push_str(&format!(
            "; {} {} left: {}",
            r.remaining.len(),
            conflicts(r.remaining.len()),
            r.remaining.join(", ")
        )),
        Some(commit) => {
            let short: String = commit.chars().take(7).collect();
            line.push_str(&format!("; committed {short}"));
            if r.pushed {
                line.push_str(" and pushed");
            } else if !r.remaining.is_empty() {
                line.push_str(&format!(
                    ", not pushed: the remote moved and a new {} is open: {}; run kluris resolve again",
                    conflicts(r.remaining.len()),
                    r.remaining.join(", ")
                ));
            } else {
                line.push_str(&format!(
                    ", not pushed: {}",
                    r.sync_error.as_deref().unwrap_or("offline")
                ));
            }
        }
    }
    line.push('\n');
    line
}
