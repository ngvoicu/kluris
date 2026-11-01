//! `kluris create [NAME] [--description TEXT] [--path DIR] [--type TYPE]
//! [--remote URL] [--branch NAME] [--no-git] [--from-config FILE] [--json]`.
//!
//! With `--remote`, the new brain's branch gets its upstream, and an
//! explicit session pushes it to the remote when the remote has no branch
//! at all; a remote that has one is refused, before anything is merged or
//! pushed, and the brain is kept without an upstream. Offline or turned
//! away, the brain is kept with its upstream, and the next command that
//! opens it pushes it, under the same refusal. The `sync`
//! object says which; the brain was created, so the exit code is 0.

use std::path::Path;

use kluris_core::companions::prompt::pick_to_add;
use kluris_core::create::{CreateError, CreateInput, CreateRequest, Created, create, wizard};
use kluris_core::git::{Git, SystemGit};
use kluris_core::scaffold::DEFAULT_BRAIN_TYPE;
use kluris_core::sync::SyncReport;
use kluris_core::sync::engine;
use kluris_core::sync::session::Intent;
use kluris_core::yaml::Date;

use crate::brains::{self, Target};
use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human::{self, Painter};
use crate::output::json::{self, Json};
use crate::prompt::{ask, can_prompt, on_terminal};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, skill_note, with_problems};

/// The message for a missing NAME when no prompt may run (`--json`,
/// `KLURIS_NO_PROMPT=1`, stdin not a terminal).
pub const NAME_REQUIRED: &str =
    "Brain name is required in non-interactive mode. Run 'kluris create <name>'.";

pub struct Options {
    pub input: CreateInput,
    pub from_config: Option<String>,
    pub json: bool,
}

/// The new brain's first push, through an explicit session: what its sync
/// did. A remote with any branch is someone else's: its own branch of the
/// brain's name the open's pull refuses before any merge, another the push
/// refuses (`engine::occupied`); nothing is pushed, and the branch's
/// upstream goes, so no later command pushes it there either.
fn push_new_brain(created: &Created, branch: &str, today: Date) -> Option<SyncReport> {
    let targets = [Target::new(&created.path, &created.name, "")];
    let mut opened = brains::open(&targets, Intent::Explicit, today, false);
    let own_branch = matches!(opened.first()?.fetched(), Some(Some(_)));
    let mut report = brains::close(opened).into_iter().next()?.report?;
    if own_branch {
        report.left_as_is = Some(engine::occupied());
    }
    if report.left_as_is.as_ref().is_some_and(engine::is_occupied) {
        // Reads leave a brain without an upstream alone. Should this fail,
        // every push is still refused the same way.
        let _ = SystemGit::new().unset_upstream(Path::new(&created.path), branch);
    }
    Some(report)
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
        // The (here empty) wizard ends with a blank line.
        human::print("\n");
    }

    let name = given.name.unwrap_or_default();
    let brain_type = given
        .brain_type
        .unwrap_or_else(|| DEFAULT_BRAIN_TYPE.to_string());
    let branch = given.branch.unwrap_or_else(|| "main".to_string());
    let home = home()?;
    let config_path = config_path()?;
    let today = today()?;
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
        today,
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
    let remote = request.remote.filter(|r| !r.is_empty() && !request.no_git);
    let sync = remote.and_then(|_| push_new_brain(&created, request.branch, today));
    let skills = refresh_skills(&home)?;
    print_failures(&skills, &home);

    if as_json {
        let companions = created.companions.iter().map(|c| Json::Str(c.clone()));
        human::print(&json::to_line(&brains::with_sync(
            with_problems(
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
            ),
            sync.as_ref(),
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
        brains::print_lines([(created.name.as_str(), sync.as_ref())]);
    }
    Ok(())
}
