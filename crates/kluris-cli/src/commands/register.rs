//! `kluris register [SOURCE] [--json]` (research-03 §1.2).

use std::io::Write;
use std::path::PathBuf;

use kluris_core::companions::prompt::pick_to_add;
use kluris_core::create::prompt_text;
use kluris_core::register::{RegisterRequest, Registered, register};

use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::prompt::{ask, can_prompt, on_terminal};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, skill_note, with_problems};

/// The message for a missing SOURCE without a terminal (v2 asked, even
/// under `--json`).
pub const PATH_REQUIRED: &str =
    "Brain path is required in non-interactive mode. Run 'kluris register <path>'.";

/// The home `~` stands for, as Python's `expanduser` finds it: `HOME` on
/// POSIX, `USERPROFILE` (else `HOMEDRIVE` + `HOMEPATH`) on Windows.
fn user_home() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        let drive = std::env::var_os("HOMEDRIVE").unwrap_or_default();
        let path = std::env::var_os("HOMEPATH")?;
        let mut home = drive;
        home.push(path);
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
}

pub fn run(source: Option<&str>, as_json: bool) -> Result<(), CliError> {
    let prompt = can_prompt(as_json);
    let given = source.filter(|s| !s.is_empty());
    let companion_prompt = prompt && given.is_none();
    let source = match given {
        Some(source) => source.to_string(),
        None if prompt => ask(|input, out| {
            let _ = out.write_all(b"\nRegister a brain\n\n");
            let path = prompt_text(input, out, "  Path to brain directory", None)?;
            let _ = out.write_all(b"\n");
            Ok(path)
        })?,
        None => return Err(CliError::new(PATH_REQUIRED)),
    };

    let home = home()?;
    let config_path = config_path()?;
    let user_home = user_home();
    let request = RegisterRequest {
        source: &source,
        user_home: user_home.as_deref(),
        config_path: &config_path,
        home: &home,
    };
    let pick = || {
        if companion_prompt {
            on_terminal(|input, out| pick_to_add(input, out))
        } else {
            Ok(Vec::new())
        }
    };
    let painter = Painter::stdout();
    match register(&request, pick).map_err(|e| CliError::new(e.to_string()))? {
        Registered::Already { name, path } => {
            if as_json {
                human::print(&json::to_line(&Json::Object(vec![
                    ("ok".into(), Json::Bool(true)),
                    ("name".into(), Json::Str(name)),
                    ("path".into(), Json::Str(path)),
                    ("already_registered".into(), Json::Bool(true)),
                ])));
            } else {
                human::print(&format!(
                    "Brain {} is already registered at {path}\n",
                    painter.bold(&name)
                ));
            }
        }
        Registered::New {
            name,
            path,
            companions,
        } => {
            let skills = refresh_skills(&home)?;
            print_failures(&skills, &home);
            if as_json {
                let companions = companions.into_iter().map(Json::Str).collect();
                let doc = Json::Object(vec![
                    ("ok".into(), Json::Bool(true)),
                    ("name".into(), Json::Str(name)),
                    ("path".into(), Json::Str(path)),
                    ("companions".into(), Json::Array(companions)),
                ]);
                human::print(&json::to_line(&with_problems(doc, &skills)));
            } else {
                human::print(&format!(
                    "Brain registered: {}\n  Path: {path}\n\n{}",
                    painter.bold(&name),
                    skill_note(painter, &name, &skills)
                ));
            }
        }
    }
    Ok(())
}
