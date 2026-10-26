//! `kluris status [--brain NAME|all] [--json]`.

use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::status::{BrainStatus, brain_status};
use kluris_core::sync::resolve::open_conflicts;
use kluris_core::sync::{Conflict, conflict_json};

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
        let root = py_path_str(&brain.entry.path);
        let store = LocalGitStore::open(Path::new(&root));
        let status = brain_status(&store, &git).map_err(|e| CliError::new(e.to_string()))?;
        // An open sync conflict (PAR-076); nothing when none is open.
        let open = open_conflicts(Path::new(&root), &git);
        if !as_json {
            // Printed brain by brain, as v2 does.
            human::print(&text(painter, &brain.name, &status, open.as_deref()));
        }
        results.push(to_json(&brain.name, &status, open.as_deref()));
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

/// `{"name", "lobes", "neurons", "uncommitted", "git_enabled", "recent_commits"}`,
/// then `"sync": {"blocked": true, "conflicts": [...]}` while a sync conflict
/// is open.
fn to_json(name: &str, s: &BrainStatus, open: Option<&[Conflict]>) -> Json {
    let mut fields = vec![
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
    ];
    if let Some(conflicts) = open {
        fields.push((
            "sync".into(),
            Json::Object(vec![
                ("blocked".into(), Json::Bool(true)),
                (
                    "conflicts".into(),
                    Json::Array(conflicts.iter().map(conflict_json).collect()),
                ),
            ]),
        ));
    }
    Json::Object(fields)
}

/// v2's wording (`cli.py:1400-1407`).
fn text(painter: Painter, name: &str, s: &BrainStatus, open: Option<&[Conflict]>) -> String {
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
    if let Some(conflicts) = open {
        let (noun, them) = if conflicts.len() == 1 {
            ("conflict", "it")
        } else {
            ("conflicts", "them")
        };
        out.push_str(&format!(
            "  Sync: blocked by {} {noun}; settle {them} with kluris resolve\n",
            conflicts.len()
        ));
        for c in conflicts {
            out.push_str(&format!("    {}: {}\n", c.kind.as_str(), c.path));
        }
    }
    out
}
