//! `kluris status [--brain NAME|all] [--json]`.

use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::status::{BrainStatus, brain_status};

use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

/// How many commits the text lists per brain.
const TEXT_COMMITS: usize = 5;

pub fn run(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, true, as_json)?;
    let git = SystemGit::new();
    let painter = Painter::stdout();
    let mut results = Vec::new();
    for brain in &brains {
        let store = LocalGitStore::open(Path::new(&py_path_str(&brain.entry.path)));
        let status = brain_status(&store, &git).map_err(|e| CliError::new(e.to_string()))?;
        if !as_json {
            // Printed brain by brain, as v2 does.
            human::print(&text(painter, &brain.name, &status));
        }
        results.push(to_json(&brain.name, &status));
    }
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(results)),
        ])));
    }
    Ok(())
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

/// v2's wording (`cli.py:1400-1407`).
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
