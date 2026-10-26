//! `kluris wake-up [--brain NAME] [--json]`.

use std::io::{self, Write};
use std::path::Path;

use kluris_core::brain::deprecation::DeprecationIssue;
use kluris_core::brain::store::LocalGitStore;
use kluris_core::pyrepr::py_path_str;
use kluris_core::wake_up::{WakeUp, wake_up};

use crate::error::CliError;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

pub fn run(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let brains = resolve_brains(brain, false, as_json)?;
    let Some(brain) = brains.first() else {
        return Ok(());
    };
    let store = LocalGitStore::open(Path::new(&py_path_str(&brain.entry.path)));
    let payload = wake_up(
        &store,
        &brain.name,
        &brain.entry.path,
        &brain.entry.description,
    );
    let text = if as_json {
        json::to_line(&to_json(&payload))
    } else {
        human(&payload)
    };
    let _ = io::stdout().write_all(text.as_bytes());
    Ok(())
}

fn str(text: &str) -> Json {
    Json::Str(text.to_string())
}

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn path(p: &str) -> Json {
    Json::Str(p.to_string())
}

/// v2's deprecation entry (`kind` first), also in `dream --json`.
pub(crate) fn issue(issue: &DeprecationIssue) -> Json {
    let kind = ("kind".to_string(), str(issue.kind()));
    Json::Object(match issue {
        DeprecationIssue::ActiveLinksToDeprecated { source, target } => vec![
            kind,
            ("source".into(), path(source)),
            ("target".into(), path(target)),
        ],
        DeprecationIssue::DeprecatedWithoutReplacement { file } => {
            vec![kind, ("file".into(), path(file))]
        }
        DeprecationIssue::ReplacedByMissing { file, target }
        | DeprecationIssue::ReplacedByNotActive { file, target } => vec![
            kind,
            ("file".into(), path(file)),
            ("target".into(), str(target)),
        ],
    })
}

/// v2's payload, key for key.
fn to_json(w: &WakeUp) -> Json {
    let lobes = w
        .lobes
        .iter()
        .map(|l| {
            Json::Object(vec![
                ("name".into(), str(&l.name)),
                ("description".into(), str(&l.description)),
                ("neurons".into(), count(l.neurons)),
                ("yaml_count".into(), count(l.yaml_count)),
            ])
        })
        .collect();
    let recent = w
        .recent
        .iter()
        .map(|r| {
            Json::Object(vec![
                ("path".into(), str(&r.path)),
                ("updated".into(), str(&r.updated)),
                ("file_type".into(), str(r.kind.as_str())),
            ])
        })
        .collect();
    let glossary = w
        .glossary
        .iter()
        .map(|e| {
            Json::Object(vec![
                ("term".into(), str(&e.term)),
                ("definition".into(), str(&e.definition)),
            ])
        })
        .collect();
    Json::Object(vec![
        ("ok".into(), Json::Bool(true)),
        ("name".into(), str(&w.name)),
        ("path".into(), str(&w.path)),
        ("description".into(), str(&w.description)),
        ("brain_md".into(), str(&w.brain_md)),
        ("lobes".into(), Json::Array(lobes)),
        ("total_neurons".into(), count(w.total_neurons)),
        ("total_yaml_neurons".into(), count(w.total_yaml_neurons)),
        ("recent".into(), Json::Array(recent)),
        ("glossary".into(), Json::Array(glossary)),
        ("deprecation_count".into(), count(w.deprecation.len())),
        (
            "deprecation".into(),
            Json::Array(w.deprecation.iter().map(issue).collect()),
        ),
        ("parse_errors".into(), count(w.parse_errors)),
    ])
}

/// v2's wording (`cli.py:1183-1201`).
fn human(w: &WakeUp) -> String {
    let mut out = format!("\nBrain: {}\n  Path: {}\n", w.name, w.path);
    if !w.description.is_empty() {
        out.push_str(&format!("  {}\n", w.description));
    }
    out.push_str(&format!("\nLobes ({})\n", w.lobes.len()));
    for lobe in &w.lobes {
        out.push_str(&format!("  - {}/: {} neurons\n", lobe.name, lobe.neurons));
    }
    out.push_str(&format!("\nTotal neurons: {}\n", w.total_neurons));
    if !w.glossary.is_empty() {
        out.push_str(&format!("\nGlossary ({} terms)\n", w.glossary.len()));
    }
    if !w.deprecation.is_empty() {
        out.push_str(&format!(
            "\nDeprecation warnings: {} (run `kluris dream` for details)\n",
            w.deprecation.len()
        ));
    }
    if !w.recent.is_empty() {
        out.push_str(&format!("\nRecently updated ({})\n", w.recent.len()));
        for item in &w.recent {
            out.push_str(&format!("  {} {}\n", item.updated, item.path));
        }
    }
    out
}
