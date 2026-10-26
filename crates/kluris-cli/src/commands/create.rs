//! `kluris create [NAME] [--description TEXT] [--path DIR] [--type TYPE]
//! [--remote URL] [--branch NAME] [--no-git] [--from-config FILE] [--json]`
//! (research-03 §1.1).

use kluris_core::companions::prompt::pick_to_add;
use kluris_core::create::{CreateError, CreateInput, CreateRequest, create, wizard};
use kluris_core::git::SystemGit;
use kluris_core::scaffold::DEFAULT_BRAIN_TYPE;

use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::prompt::{ask, can_prompt, on_terminal};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, skill_note, with_problems};

/// The message for a missing NAME without a terminal (v2 asked, or raised
/// under `--json`).
pub const NAME_REQUIRED: &str =
    "Brain name is required in non-interactive mode. Run 'kluris create <name>'.";

pub struct Options {
    pub input: CreateInput,
    pub from_config: Option<String>,
    pub json: bool,
}

fn cli_error(err: CreateError) -> CliError {
    match err {
        CreateError::Aborted => CliError::aborted(),
        other => CliError::new(other.to_string()),
    }
}

pub fn run(options: Options) -> Result<(), CliError> {
    let Options {
        input: mut given,
        from_config,
        json: as_json,
    } = options;
    let prompt = can_prompt(as_json);
    let companion_prompt = prompt && given.wants_companion_prompt();
    if prompt {
        let cwd = std::env::current_dir()
            .map_err(|e| CliError::new(format!("Cannot read the current directory: {e}")))?;
        given = ask(|input, out| wizard(given, &cwd, input, out))?;
    } else if as_json {
        if given.name.is_none() {
            return Err(CliError::new(NAME_REQUIRED));
        }
    } else {
        if given.name.as_deref().is_none_or(str::is_empty) {
            return Err(CliError::new(NAME_REQUIRED));
        }
        // v2 ends its (here empty) wizard with a blank line.
        human::print("\n");
    }

    let name = given.name.unwrap_or_default();
    let brain_type = given
        .brain_type
        .unwrap_or_else(|| DEFAULT_BRAIN_TYPE.to_string());
    let branch = given.branch.unwrap_or_else(|| "main".to_string());
    let home = home()?;
    let config_path = config_path()?;
    let request = CreateRequest {
        name: &name,
        description: given.description.as_deref(),
        path: given.path.as_deref(),
        brain_type: &brain_type,
        remote: given.remote.as_deref(),
        branch: &branch,
        no_git: given.no_git,
        from_config: from_config.as_deref(),
        config_path: &config_path,
        home: &home,
        today: today()?,
    };
    let pick = || {
        if companion_prompt {
            on_terminal(|input, out| pick_to_add(input, out))
        } else {
            Ok(Vec::new())
        }
    };
    let created = create(&request, &SystemGit::new(), pick).map_err(cli_error)?;
    for warning in &created.warnings {
        eprintln!("{warning}");
    }
    let skills = refresh_skills(&home)?;
    print_failures(&skills, &home);

    if as_json {
        let companions = created.companions.iter().map(|c| Json::Str(c.clone()));
        human::print(&json::to_line(&with_problems(
            Json::Object(vec![
                ("ok".into(), Json::Bool(true)),
                ("name".into(), Json::Str(created.name.clone())),
                ("path".into(), Json::Str(created.path.clone())),
                ("type".into(), Json::Str(created.brain_type.clone())),
                (
                    "lobes".into(),
                    Json::Int(i64::try_from(created.lobes).unwrap_or(i64::MAX)),
                ),
                ("companions".into(), Json::Array(companions.collect())),
            ]),
            &skills,
        )));
    } else {
        let painter = Painter::stdout();
        human::print(&format!(
            "Brain created: {} ({})\n  Path: {}\n  Lobes: {}\n\n{}",
            painter.bold(&created.name),
            created.brain_type,
            created.path,
            created.lobes,
            skill_note(painter, &created.name, &skills)
        ));
    }
    Ok(())
}
