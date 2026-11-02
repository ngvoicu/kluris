//! `kluris commit --brain NAME [--paths a.md,b.md] [-m MSG] [--merge ID]
//! [--no-verify] [--json]`: commit the approved files, the maps they change
//! and the reverse links they call for, through an explicit session that
//! pulls teammates' commits first and pushes once it is done; with a merge
//! waiting, `--merge` finishes it from the settled files
//! (`kluris_core::sync::settle`).
//!
//! The exit code is the commit's: 0 whenever it landed (or there was
//! nothing to commit), whatever the sync did, which the `sync` object says;
//! 1 when nothing was committed. Once HEAD moved the commit landed: what
//! fails after it (updating the index, listing what is left) is a
//! `warning`, with how to repair it, and the exit stays 0. However it ends
//! once the session opened,
//! the session closes as it does after a commit (it pushes what waits), and
//! the `sync` object says what it did, in a failure too; a refusal while a
//! merge waits carries that merge's conflicts, their texts and its id. A
//! brain with no remote is a brain of this machine alone: its commit says
//! nothing of a sync.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::commit::{CommitError, CommitReport, CommitRequest};
use kluris_core::sync::engine;
use kluris_core::sync::session::{Alone, Intent, Session};
use kluris_core::sync::settle::commit_in;
use kluris_core::sync::{ConflictText, LeftAsIs, LeftReason, SyncReport};

use crate::brains::{self, Target};
use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

pub struct Options<'a> {
    pub brain: Option<&'a str>,
    /// `--paths`: comma-separated, each trimmed, empty items dropped;
    /// empty when left out (with `--merge`).
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
        CommitError::MergeWaits(_)
            | CommitError::WrongMerge(_)
            | CommitError::Unsettled { .. }
            | CommitError::Unrelated { .. }
            | CommitError::Structure { .. }
            | CommitError::Unwritable { .. }
            | CommitError::InvalidFile { .. }
    )
}

/// The waiting merge's conflicts, for a refusal of it to say (an open that
/// did not fetch reads none of its own).
fn waiting_conflicts(session: &Session, error: &CommitError) -> Vec<ConflictText> {
    match session.context() {
        Some(ctx) if about_the_merge(error) => engine::waiting(ctx).unwrap_or_default(),
        _ => Vec::new(),
    }
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
        brain: &brain.name,
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
    let waiting = match &committed {
        Err(e) => waiting_conflicts(session, e),
        Ok(_) => Vec::new(),
    };
    if committed
        .as_ref()
        .is_ok_and(|report| report.commit.is_some())
    {
        session.committed();
    }
    let closed = brains::close(opened).into_iter().next().unwrap_or_default();
    let mut sync = match closed.alone {
        Some(Alone::NoRemote) => None,
        _ => closed.report,
    };
    let sync = sync.as_mut().map(|sync| {
        if sync.conflicts.is_empty() {
            sync.conflicts = waiting;
        }
        &*sync
    });
    let report = match committed {
        Ok(report) => report,
        Err(e) => {
            // Nothing was committed; what the sync did is said.
            let error = CliError::new(message(&brain.name, &root, &e));
            if !options.json {
                brains::print_lines([(brain.name.as_str(), sync)]);
            }
            return Err(match sync {
                Some(sync) => error.with_fields(vec![("sync".into(), sync.to_json())]),
                None => error,
            });
        }
    };

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
        let doc = match (doc, &report.warning) {
            (Json::Object(mut fields), Some(warning)) => {
                fields.push(("warning".into(), Json::Str(warning.clone())));
                Json::Object(fields)
            }
            (doc, _) => doc,
        };
        human::print(&json::to_line(&brains::with_sync(doc, sync)));
    } else {
        human::print(&text(&brain.name, &report));
        brains::print_lines([(brain.name.as_str(), sync)]);
        if let Some(warning) = &report.warning {
            eprintln!("Warning: {warning}");
        }
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
