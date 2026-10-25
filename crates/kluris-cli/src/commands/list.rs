//! `kluris list [--json]`.

use std::io::{self, Write};

use kluris_core::config::global_config_path;
use kluris_core::list::{BrainListing, list_brains};

use crate::error::CliError;
use crate::output::json::{self, Json};
use crate::output::table;

pub fn run(as_json: bool) -> Result<(), CliError> {
    let path = global_config_path(
        std::env::var_os("KLURIS_CONFIG").as_deref(),
        std::env::var_os("HOME").as_deref(),
        std::env::home_dir(),
    )
    .ok_or_else(|| CliError::new("Cannot find the home directory; set HOME or KLURIS_CONFIG."))?;
    let brains = list_brains(&path).map_err(|e| CliError::new(e.to_string()))?;

    let text = if as_json {
        json::to_line(&to_json(&brains))
    } else if brains.is_empty() {
        "No brains registered. Run 'kluris create <name>' to create one.\n".to_string()
    } else {
        human(&brains)
    };
    let _ = io::stdout().write_all(text.as_bytes());
    Ok(())
}

/// `{"ok": true, "brains": [{"name", "path", "description", "companions"}]}`.
fn to_json(brains: &[BrainListing]) -> Json {
    let items = brains
        .iter()
        .map(|b| {
            let companions = match &b.companions {
                None => Json::Null,
                Some(names) => Json::Array(names.iter().map(|n| Json::Str(n.clone())).collect()),
            };
            Json::Object(vec![
                ("name".into(), Json::Str(b.name.clone())),
                ("path".into(), Json::Str(b.path.clone())),
                ("description".into(), Json::Str(b.description.clone())),
                ("companions".into(), companions),
            ])
        })
        .collect();
    Json::Object(vec![
        ("ok".into(), Json::Bool(true)),
        ("brains".into(), Json::Array(items)),
    ])
}

fn human(brains: &[BrainListing]) -> String {
    let rows: Vec<Vec<String>> = brains
        .iter()
        .map(|b| {
            let companions = match &b.companions {
                None => "(unknown)".to_string(),
                Some(names) if names.is_empty() => "(none)".to_string(),
                Some(names) => names.join(", "),
            };
            vec![
                b.name.clone(),
                b.path.clone(),
                b.description.clone(),
                companions,
            ]
        })
        .collect();
    table::render(
        "Registered Brains",
        &["Name", "Path", "Description", "Companions"],
        &rows,
    )
}
