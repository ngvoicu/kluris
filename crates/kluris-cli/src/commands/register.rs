//! `kluris register [SOURCE] [--json]`: the brain is
//! found, its session brings teammates' commits in, and then it is read
//! and registered. A brain whose branch has no upstream says so
//! (`no_upstream`) and gets none.

use std::io::Write;
use std::path::PathBuf;

use kluris_core::agents::install::InstallReport;
use kluris_core::companions::prompt::pick_to_add;
use kluris_core::create::prompt_text;
use kluris_core::register::{RegisterRequest, Registered, locate, register};
use kluris_core::scaffold::{fallback_name, read_brain_identity};
use kluris_core::sync::SyncReport;
use kluris_core::sync::session::{Alone, Intent};

use crate::brains::{self, Closed, Target};
use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::prompt::{ask, can_prompt, on_terminal};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, skill_note, with_problems};

/// The message for a missing SOURCE when no prompt may run (`--json`,
/// `KLURIS_NO_PROMPT=1`, stdin not a terminal).
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
    let root = locate(&request).map_err(|e| CliError::new(e.to_string()))?;
    let dir_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (name, description) = read_brain_identity(&root, &fallback_name(&dir_name));
    let targets = [Target::new(
        &root.display().to_string(),
        &name,
        &description,
    )];
    let opened = brains::open_today(&targets, Intent::Read)?;
    // Read and registered, the skills refreshed, all while the session is
    // open.
    let done = register(&request, &root, pick)
        .map_err(|e| CliError::new(e.to_string()))
        .and_then(|registered| {
            Ok(match registered {
                Registered::Already { name, path } => Done::Already { name, path },
                Registered::New {
                    name,
                    path,
                    companions,
                } => Done::New {
                    name,
                    path,
                    companions,
                    skills: refresh_skills(&home)?,
                },
            })
        });
    let report = said(brains::close(opened));
    let report = report.as_ref();
    let painter = Painter::stdout();
    match done? {
        Done::Already { name, path } => {
            if as_json {
                let doc = Json::Object(vec![
                    ("ok".into(), Json::Bool(true)),
                    ("name".into(), Json::Str(name)),
                    ("path".into(), Json::Str(path)),
                    ("already_registered".into(), Json::Bool(true)),
                ]);
                human::print(&json::to_line(&brains::with_sync(doc, report)));
            } else {
                human::print(&format!(
                    "Brain {} is already registered at {path}\n",
                    painter.bold(&name)
                ));
                brains::print_lines([(name.as_str(), report)]);
            }
        }
        Done::New {
            name,
            path,
            companions,
            skills,
        } => {
            print_failures(&skills, &home);
            if as_json {
                let companions = companions.into_iter().map(Json::Str).collect();
                let doc = Json::Object(vec![
                    ("ok".into(), Json::Bool(true)),
                    ("name".into(), Json::Str(name)),
                    ("path".into(), Json::Str(path)),
                    ("companions".into(), Json::Array(companions)),
                ]);
                let doc = brains::with_sync(with_problems(doc, &skills), report);
                human::print(&json::to_line(&doc));
            } else {
                human::print(&format!(
                    "Brain registered: {}\n  Path: {path}\n\n{}",
                    painter.bold(&name),
                    skill_note(painter, &name, &skills)
                ));
                brains::print_lines([(name.as_str(), report)]);
            }
        }
    }
    Ok(())
}

/// What `register` did, with the skill install a new brain gets.
enum Done {
    Already {
        name: String,
        path: String,
    },
    New {
        name: String,
        path: String,
        companions: Vec<String>,
        skills: InstallReport,
    },
}

/// What the brain's sync says, a branch without an upstream included: a
/// brain is registered to be shared.
fn said(closed: Vec<Closed>) -> Option<SyncReport> {
    let closed = closed.into_iter().next()?;
    match (closed.report, closed.alone) {
        (Some(report), _) => Some(report),
        (None, Some(Alone::NoUpstream(left))) => Some(SyncReport {
            left_as_is: Some(left),
            ..SyncReport::default()
        }),
        (None, _) => None,
    }
}
