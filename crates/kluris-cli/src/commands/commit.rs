//! `kluris commit --brain NAME --paths a.md,b.md [-m MSG] [--json]` (new in
//! v3.1, SPEC item 44): commit the approved files and the maps they change.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::commit::{CommitError, CommitReport, CommitRequest, commit};

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
    let report = commit(
        Path::new(&root),
        &SystemGit::new(),
        &CommitRequest {
            paths: &paths,
            message: options.message,
            agent: agent.as_deref(),
            no_verify: options.no_verify,
            maps: DreamOptions {
                name: &config.name,
                description: &config.description,
                today: today()?,
            },
        },
    )
    .map_err(|e| CliError::new(message(&brain.name, &root, &e)))?;

    if options.json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(brain.name.clone())),
            ("committed".into(), Json::Bool(report.commit.is_some())),
            (
                "commit".into(),
                report.commit.clone().map_or(Json::Null, Json::Str),
            ),
            ("paths".into(), strings(&report.paths)),
            ("regenerated".into(), strings(&report.regenerated)),
            ("left_dirty".into(), strings(&report.left_dirty)),
        ])));
    } else {
        human::print(&text(&brain.name, &report));
    }
    Ok(())
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
