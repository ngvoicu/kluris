//! `brain-dump`: the brain model of one brain as JSON, for
//! `parity/test_brain_model.py` (its v2 side is
//! `parity/tools/brain_model_oracle.py`, same shape).

use std::path::Path;

use kluris_core::brain::deprecation::{DeprecationIssue, detect_deprecation_issues};
use kluris_core::brain::store::{BrainStore, LocalGitStore};
use serde_json::{Value, json};

fn native(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn issue(issue: &DeprecationIssue) -> Value {
    let kind = issue.kind();
    match issue {
        DeprecationIssue::ActiveLinksToDeprecated { source, target } => {
            json!({"kind": kind, "source": native(source), "target": native(target)})
        }
        DeprecationIssue::DeprecatedWithoutReplacement { file } => {
            json!({"kind": kind, "file": native(file)})
        }
        DeprecationIssue::ReplacedByMissing { file, target }
        | DeprecationIssue::ReplacedByNotActive { file, target } => {
            json!({"kind": kind, "file": native(file), "target": target})
        }
    }
}

/// `brain-dump --path <brain>`: `{neurons, glossary, deprecation, skipped}`.
pub fn main(args: &[String]) -> Result<(), String> {
    let [flag, path] = args else {
        return Err("usage: brain-dump --path <brain>".into());
    };
    if flag != "--path" {
        return Err("usage: brain-dump --path <brain>".into());
    }
    let store = LocalGitStore::open(Path::new(path));
    let neurons: Vec<Value> = store
        .neurons()
        .iter()
        .map(|n| json!({"path": n.rel, "kind": n.kind.as_str()}))
        .collect();
    let glossary = match store.glossary() {
        Ok(entries) => Value::from(
            entries
                .iter()
                .map(|e| json!([e.term, e.definition]))
                .collect::<Vec<_>>(),
        ),
        Err(_) => Value::Null,
    };
    let report = detect_deprecation_issues(&store);
    let out = json!({
        "neurons": neurons,
        "glossary": glossary,
        "deprecation": report.issues.iter().map(issue).collect::<Vec<_>>(),
        "skipped": report.skipped.iter().map(|s| s.rel.as_str()).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
    );
    Ok(())
}
