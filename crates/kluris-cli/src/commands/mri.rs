//! `kluris mri [--brain NAME|all] [--output FILE] [--json]` (v2
//! `cli.py:1556-1604`): the dream preflight, then the brain's page.

use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::config::read_brain_config;
use kluris_core::dream::{DreamOptions, sync_brain_state};
use kluris_core::git::SystemGit;
use kluris_core::mri::{generate, page_uri};
use kluris_core::pyrepr::py_path_str;

use crate::commands::dream::{fixes_json, today};
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

/// v2's `--output` refusal for more than one brain.
const OUTPUT_WITH_FAN_OUT: &str = "--output cannot be combined with multi-brain fan-out (each brain \
would overwrite the others' output). Pass --output without --brain all, or pick brains one at a time.";

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

pub fn run(brain: Option<&str>, output: Option<&str>, as_json: bool) -> Result<(), CliError> {
    // Click passes `--output ""` on as a falsy value: the default page.
    let output = output.filter(|o| !o.is_empty());
    let brains = resolve_brains(brain, true, as_json)?;
    if output.is_some() && brains.len() > 1 {
        return Err(CliError::new(OUTPUT_WITH_FAN_OUT));
    }
    let git = SystemGit::new();
    let today = today()?;
    let mut results = Vec::new();
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        let config =
            read_brain_config(Path::new(&root)).map_err(|e| CliError::new(e.to_string()))?;
        let mut store = LocalGitStore::open(Path::new(&root));
        let sync = sync_brain_state(
            &mut store,
            &git,
            &DreamOptions {
                name: &config.name,
                description: &config.description,
                today,
            },
        );
        // `str(Path(--output))`, or `str(brain_path / "brain-mri.html")`.
        let out = match output {
            Some(o) => py_path_str(o),
            None => py_path_str(&format!("{root}/brain-mri.html")),
        };
        let stats = generate(&store, &brain.name, Path::new(&out))
            .map_err(|e| CliError::new(format!("Cannot write {out}: {e}")))?;
        if !as_json {
            let uri = page_uri(Path::new(&out));
            let mut text = format!(
                "MRI complete — {out}\n  Open: {uri}\n  {} nodes, {} edges\n",
                stats.nodes, stats.edges
            );
            if sync.fixes.total > 0 {
                text.push_str(&format!(
                    "  MRI preflight applied {} automatic fixes\n",
                    sync.fixes.total
                ));
            }
            human::print(&text);
        }
        results.push(Json::Object(vec![
            ("name".into(), Json::Str(brain.name.clone())),
            ("output_path".into(), Json::Str(out)),
            ("preflight_fixes".into(), fixes_json(&sync.fixes)),
            ("nodes".into(), count(stats.nodes)),
            ("edges".into(), count(stats.edges)),
        ]));
    }
    if as_json {
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(results)),
        ])));
    }
    Ok(())
}
