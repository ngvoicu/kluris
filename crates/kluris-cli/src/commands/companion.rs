//! `kluris companion list|add|remove [NAME] [--brain NAME|all] [--json]`
//! (research-03 §1.7-1.9).

use kluris_core::companions::prompt::{pick_to_add, pick_to_remove};
use kluris_core::companions::{self, BrainOptIns, KNOWN, OptInBrain};
use kluris_core::config::read_global_config;
use kluris_core::resolve::ResolvedBrain;

use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::prompt::{ask, can_prompt};
use crate::resolve::{config_path, home, resolve_brains};
use crate::skills::refresh_skills;

/// The NAME argument's value name: Click shows a `Choice`'s choices.
pub const NAME_CHOICES: &str = "specmint-core|specmint-tdd|specmint-core-html|specmint-tdd-html";

/// NAME as Click's `Choice(KNOWN)` converts it.
pub fn parse_name(value: &str) -> Result<String, String> {
    if KNOWN.contains(&value) {
        return Ok(value.to_string());
    }
    let choices: Vec<String> = KNOWN.iter().map(|n| format!("'{n}'")).collect();
    Err(format!("is not one of {}", choices.join(", ")))
}

fn name_required() -> CliError {
    CliError::new(format!(
        "Companion name is required in non-interactive mode. Pass one of: {}",
        KNOWN.join(", ")
    ))
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

fn names(brains: &[ResolvedBrain]) -> Vec<String> {
    brains.iter().map(|b| b.name.clone()).collect()
}

fn cli_error(err: impl std::fmt::Display) -> CliError {
    CliError::new(err.to_string())
}

// --- list ---------------------------------------------------------------------

pub fn list(brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let config = read_global_config(&config_path()?).map_err(cli_error)?;
    let brains = match brain.filter(|b| !b.is_empty()) {
        Some(_) => resolve_brains(brain, true, as_json)?,
        None => config
            .brains
            .iter()
            .map(|(name, entry)| ResolvedBrain {
                name: name.clone(),
                entry: entry.clone(),
            })
            .collect(),
    };
    let installed = companions::installed(&home()?);
    let rows = companions::listing(&brains);
    if as_json {
        human::print(&json::to_line(&list_json(&installed, &rows)));
    } else {
        human::print(&list_text(&installed, &rows));
    }
    Ok(())
}

/// `{"ok", "known", "installed", "brains": [{"name", "path", "companions"}]}`.
fn list_json(installed: &[String], rows: &[BrainOptIns]) -> Json {
    let brains = rows
        .iter()
        .map(|row| {
            Json::Object(vec![
                ("name".into(), Json::Str(row.name.clone())),
                ("path".into(), Json::Str(row.path.clone())),
                (
                    "companions".into(),
                    row.companions.as_deref().map_or(Json::Null, strings),
                ),
            ])
        })
        .collect();
    Json::Object(vec![
        ("ok".into(), Json::Bool(true)),
        (
            "known".into(),
            Json::Array(KNOWN.iter().map(|n| Json::Str(n.to_string())).collect()),
        ),
        ("installed".into(), strings(installed)),
        ("brains".into(), Json::Array(brains)),
    ])
}

fn list_text(installed: &[String], rows: &[BrainOptIns]) -> String {
    let painter = Painter::stdout();
    let known: Vec<Vec<String>> = KNOWN
        .iter()
        .map(|name| {
            let yes = installed.iter().any(|i| i == name);
            vec![name.to_string(), if yes { "yes" } else { "no" }.to_string()]
        })
        .collect();
    let mut out = human::table(painter, "Known Companions", &["Name", "Installed"], &known);
    if rows.is_empty() {
        out.push_str("No brains registered. Run 'kluris create <name>' to create one.\n");
        return out;
    }
    let brains: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let cell = match &row.companions {
                None => "(unknown)".to_string(),
                Some(list) if list.is_empty() => "(none)".to_string(),
                Some(list) => list.join(", "),
            };
            vec![row.name.clone(), cell]
        })
        .collect();
    out.push_str(&human::table(
        painter,
        "Brain Companion Opt-ins",
        &["Brain", "Companions"],
        &brains,
    ));
    out
}

// --- add and remove --------------------------------------------------------------

/// What `add` and `remove` print: `{"ok", "names", "brains", <flags>[,
/// "name"]}`, `"name"` only when exactly one companion was selected.
fn result_json(selected: &[String], brains: &[String], flags: [(&str, bool); 2]) -> Json {
    let mut fields = vec![
        ("ok".into(), Json::Bool(true)),
        ("names".into(), strings(selected)),
        ("brains".into(), strings(brains)),
    ];
    for (key, value) in flags {
        fields.push((key.into(), Json::Bool(value)));
    }
    if let [only] = selected {
        fields.push(("name".into(), Json::Str(only.clone())));
    }
    Json::Object(fields)
}

fn nothing_to_do(
    as_json: bool,
    resolved: &[ResolvedBrain],
    flags: [(&str, bool); 2],
) -> Result<(), CliError> {
    if as_json {
        human::print(&json::to_line(&result_json(&[], &names(resolved), flags)));
    } else {
        human::print("No companion selected. Nothing to do.\n");
    }
    Ok(())
}

const ADDED: [(&str, bool); 2] = [("opted_in", true), ("files_copied", true)];
const NOT_ADDED: [(&str, bool); 2] = [("opted_in", true), ("files_copied", false)];
const REMOVED: [(&str, bool); 2] = [("opted_in", false), ("files_kept", true)];

pub fn add(name: Option<&str>, brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let resolved = resolve_brains(brain, true, as_json)?;
    let selected = match name {
        Some(name) => vec![name.to_string()],
        None if can_prompt(as_json) => ask(|input, out| pick_to_add(input, out))?,
        None => return Err(name_required()),
    };
    if selected.is_empty() {
        return nothing_to_do(as_json, &resolved, NOT_ADDED);
    }
    let brains = companions::load_brains(&resolved).map_err(cli_error)?;
    let home = home()?;
    let changed = companions::add(&brains, &selected, &home).map_err(cli_error)?;
    refresh_skills(&home)?;
    if as_json {
        human::print(&json::to_line(&result_json(&selected, &changed, ADDED)));
    } else {
        let label = Painter::stdout().bold(&selected.join(", "));
        human::print(&format!(
            "Added companion(s) {label} to: {}\n",
            changed.join(", ")
        ));
    }
    Ok(())
}

pub fn remove(name: Option<&str>, brain: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let resolved = resolve_brains(brain, true, as_json)?;
    let selected = match name {
        Some(name) => vec![name.to_string()],
        None if can_prompt(as_json) => {
            // v2 `_enabled_union`: a brain whose kluris.yml does not read
            // counts as none.
            let readable: Vec<OptInBrain> = resolved
                .iter()
                .filter_map(|b| companions::load_brains(std::slice::from_ref(b)).ok())
                .flatten()
                .collect();
            let enabled = companions::enabled(&readable);
            if enabled.is_empty() {
                return Err(CliError::new(
                    "No companions are enabled on the selected brain(s).",
                ));
            }
            ask(|input, out| pick_to_remove(&enabled, input, out))?
        }
        None => return Err(name_required()),
    };
    if selected.is_empty() {
        return nothing_to_do(as_json, &resolved, REMOVED);
    }
    let brains = companions::load_brains(&resolved).map_err(cli_error)?;
    let changed = companions::remove(&brains, &selected).map_err(cli_error)?;
    refresh_skills(&home()?)?;
    if as_json {
        human::print(&json::to_line(&result_json(&selected, &changed, REMOVED)));
    } else {
        let label = Painter::stdout().bold(&selected.join(", "));
        let brains = if changed.is_empty() {
            "(none)".to_string()
        } else {
            changed.join(", ")
        };
        human::print(&format!(
            "Removed companion(s) {label} from: {brains}\n\
             Runtime companion files were kept under ~/.kluris/companions.\n"
        ));
    }
    Ok(())
}
