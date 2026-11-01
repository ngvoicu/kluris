//! `kluris status [--brain NAME|all] [--json]`.

use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::status::{BrainStatus, brain_status};
use kluris_core::sync::session::Intent;

use crate::brains::{self, Target};
use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

/// How many commits the text lists per brain.
const TEXT_COMMITS: usize = 5;

pub fn run(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, true, as_json)?;
    let targets: Vec<Target> = brains
        .iter()
        .map(|b| Target::new(&b.entry.path, &b.name, &b.entry.description))
        .collect();
    let opened = brains::open_today(&targets, Intent::Read)?;
    let git = SystemGit::new();
    // Read brain by brain, up to the first that cannot be read.
    let mut statuses = Vec::new();
    let mut failure = None;
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        let store = LocalGitStore::open(Path::new(&root));
        match brain_status(&store, &git) {
            Ok(status) => statuses.push(status),
            Err(e) => {
                failure = Some(CliError::new(e.to_string()));
                break;
            }
        }
    }
    let closed = brains::close(opened);
    let read = brains.iter().zip(&statuses).zip(&closed);
    if as_json {
        if let Some(failure) = failure {
            return Err(failure);
        }
        let results = read
            .map(|((brain, status), closed)| {
                brains::with_sync(to_json(&brain.name, status), closed.report.as_ref())
            })
            .collect();
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(results)),
        ])));
        return Ok(());
    }
    let painter = Painter::stdout();
    let mut lines = Vec::new();
    for ((brain, status), closed) in read {
        human::print(&text(painter, &brain.name, status));
        lines.push((brain.name.as_str(), closed.report.as_ref()));
    }
    brains::print_lines(lines);
    failure.map_or(Ok(()), Err)
}

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

/// `{"name", "lobes", "neurons", "uncommitted", "git_enabled", "recent_commits"}`.
fn to_json(name: &str, s: &BrainStatus) -> Json {
    Json::Object(vec![
        ("name".into(), Json::Str(name.to_string())),
        ("lobes".into(), count(s.lobes)),
        ("neurons".into(), count(s.neurons)),
        ("uncommitted".into(), Json::Str(s.uncommitted.clone())),
        ("git_enabled".into(), Json::Bool(s.git_enabled)),
        (
            "recent_commits".into(),
            Json::Array(
                s.recent
                    .iter()
                    .map(|c| Json::Str(c.subject.clone()))
                    .collect(),
            ),
        ),
    ])
}

/// The text report.
fn text(painter: Painter, name: &str, s: &BrainStatus) -> String {
    let mut out = format!("\n{}\n", painter.bold(name));
    out.push_str(&format!("  Lobes: {}, Neurons: {}\n", s.lobes, s.neurons));
    if !s.git_enabled {
        out.push_str("  Git: disabled\n");
    }
    if !s.uncommitted.is_empty() {
        out.push_str(&format!("  Uncommitted:\n{}\n", s.uncommitted));
    }
    for commit in s.recent.iter().take(TEXT_COMMITS) {
        let day: String = commit.date.chars().take(10).collect();
        out.push_str(&format!("  {day} {}\n", commit.subject));
    }
    out
}
