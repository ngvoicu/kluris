//! `kluris commit --brain NAME --paths a.md,b.md [-m MSG] [--merge ID]
//! [--no-verify] [--json]`: commit the approved files, the maps they change
//! and the reverse links they call for, through an explicit session that
//! pulls teammates' commits first and pushes once it is done; with a merge
//! waiting, `--merge` finishes it from the settled files
//! (`kluris_core::sync::settle`).
//!
//! The exit code is the commit's: 0 whenever it landed (or there was
//! nothing to commit), whatever the sync did, which the `sync` object says;
//! 1 when nothing was committed. A refusal while a merge waits carries the
//! `sync` object with that merge's conflicts, their texts and its id.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::commit::{CommitError, CommitReport, CommitRequest};
use kluris_core::sync::engine;
use kluris_core::sync::session::{Intent, Session};
use kluris_core::sync::settle::commit_in;
use kluris_core::sync::{LeftAsIs, LeftReason, SyncReport};

use crate::brains::{self, Target};
use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

pub struct Options<'a> {
    pub brain: Option<&'a str>,
    /// `--paths`: comma-separated, each trimmed, empty items dropped.
    pub paths: &'a str,
    pub message: Option<&'a str>,
    /// `--merge`: the id of the waiting merge the settled files finish.
    pub merge: Option<&'a str>,
    pub no_verify: bool,
    pub json: bool,
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

fn message(name: &str, root: &str, error: &CommitError) -> String {
    match error {
        CommitError::NotARepo => format!("Brain {name} is not a git repository: {root}"),
        CommitError::InProgress(op) => format!(
            "Brain {name} is in the middle of a {op}; finish or abort it first (nothing was committed)"
        ),
        other => other.to_string(),
    }
}

/// Whether `error` is a refusal while a merge waits, which says that
/// merge's conflicts.
fn about_the_merge(error: &CommitError) -> bool {
    matches!(
        error,
        CommitError::MergeWaits { .. }
            | CommitError::WrongMerge(_)
            | CommitError::Unsettled(_)
            | CommitError::Unrelated { .. }
            | CommitError::Structure(_)
            | CommitError::InvalidFile { .. }
    )
}

/// The session's report so far, with the waiting merge's conflicts when it
/// has none of its own (an open that did not fetch reads none).
fn with_waiting(session: &Session) -> SyncReport {
    let mut report = session.report().cloned().unwrap_or_default();
    if report.conflicts.is_empty()
        && let Some(ctx) = session.context()
        && let Ok(conflicts) = engine::waiting(ctx)
    {
        report.conflicts = conflicts;
    }
    report
}

pub fn run(options: &Options) -> Result<(), CliError> {
    let brains = resolve_brains(options.brain, false, options.json)?;
    let Some(brain) = brains.first() else {
        return Err(CliError::new("No brain to commit in"));
    };
    let root = py_path_str(&brain.entry.path);
    let config = read_brain_config(Path::new(&root)).map_err(|e| match e.kind {
        ConfigErrorKind::NotFound => CliError::new(format!(
            "Brain {} has no kluris.yml: {}",
            brain.name,
            e.path.display()
        )),
        _ => CliError::new(e.to_string()),
    })?;
    let paths: Vec<String> = options
        .paths
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect();
    let agent = std::env::var("KLURIS_AGENT").ok();
    let today = today()?;
    let request = CommitRequest {
        paths: &paths,
        message: options.message,
        agent: agent.as_deref(),
        no_verify: options.no_verify,
        maps: DreamOptions {
            name: &config.name,
            description: &config.description,
            today,
        },
    };
    let targets = [Target::new(
        &brain.entry.path,
        &config.name,
        &config.description,
    )];
    let mut opened = brains::open(&targets, Intent::Explicit, today, options.no_verify);
    let Some(session) = opened.first() else {
        return Err(CliError::new("No brain to commit in"));
    };
    let plain = SystemGit::new();
    let committed = match (session.context(), session.claim()) {
        (Some(ctx), Some(claim)) => commit_in(
            Path::new(&root),
            ctx.git(),
            Some((ctx, claim)),
            &request,
            options.merge,
        ),
        // Another command holds the work tree past the bound.
        (Some(_), None) => Err(CommitError::Left(busy(session.report()))),
        (None, _) => commit_in(Path::new(&root), &plain, None, &request, options.merge),
    };
    let report = match committed {
        Ok(report) => report,
        Err(e) => {
            // Nothing was committed, and nothing is pushed.
            let sync = about_the_merge(&e).then(|| with_waiting(session));
            drop(opened);
            let error = CliError::new(message(&brain.name, &root, &e));
            if !options.json
                && let Some(sync) = &sync
            {
                brains::print_lines([(brain.name.as_str(), Some(sync))]);
            }
            return Err(match sync {
                Some(sync) => error.with_fields(vec![("sync".into(), sync.to_json())]),
                None => error,
            });
        }
    };
    if report.commit.is_some() {
        session.committed();
    }
    let closed = brains::close(opened);
    let sync = closed.first().and_then(|c| c.report.as_ref());

    if options.json {
        let doc = Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(brain.name.clone())),
            ("committed".into(), Json::Bool(report.commit.is_some())),
            (
                "commit".into(),
                report.commit.clone().map_or(Json::Null, Json::Str),
            ),
            ("paths".into(), strings(&report.paths)),
            ("regenerated".into(), strings(&report.regenerated)),
            ("linked".into(), strings(&report.linked)),
            ("left_dirty".into(), strings(&report.left_dirty)),
        ]);
        human::print(&json::to_line(&brains::with_sync(doc, sync)));
    } else {
        human::print(&text(&brain.name, &report));
        brains::print_lines([(brain.name.as_str(), sync)]);
    }
    Ok(())
}

/// Why nothing was committed while another command held the work tree:
/// the session's own words when it said so.
fn busy(report: Option<&SyncReport>) -> LeftAsIs {
    report
        .and_then(|r| r.left_as_is.clone())
        .filter(|left| left.reason == LeftReason::Busy)
        .unwrap_or_else(|| {
            LeftAsIs::because(
                LeftReason::Busy,
                "another kluris command is changing this brain's files; run again once it is done",
            )
        })
}

fn text(name: &str, report: &CommitReport) -> String {
    let mut out = match (&report.commit, &report.message) {
        (Some(hash), Some(message)) => {
            let short: String = hash.chars().take(7).collect();
            let subject = message.lines().next().unwrap_or_default();
            let mut out = format!("Committed {short} in {name}: {subject}\n");
            for path in &report.paths {
                out.push_str(&format!("  {path}\n"));
            }
            for path in &report.regenerated {
                out.push_str(&format!("  {path} (regenerated)\n"));
            }
            for path in &report.linked {
                out.push_str(&format!("  {path} (linked)\n"));
            }
            out
        }
        _ => format!("Nothing to commit in {name}.\n"),
    };
    if !report.left_dirty.is_empty() {
        out.push_str(&format!(
            "Left uncommitted: {}\n",
            report.left_dirty.join(", ")
        ));
    }
    out
}
