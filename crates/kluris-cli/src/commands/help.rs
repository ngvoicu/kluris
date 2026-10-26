//! `kluris help [COMMAND] [--json]`.

use kluris_core::config::read_global_config;
use kluris_core::help::{COMMANDS, CommandHelp, find, unknown_command};
use kluris_core::pyrepr::py_path_str;

use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::config_path;

pub fn run(command: Option<&str>, as_json: bool) -> Result<(), CliError> {
    // v2 reads the config before anything else, so a config that does not
    // read fails `help` too (cleanly here; v2 ends in a traceback).
    let path = config_path()?;
    let config = read_global_config(&path).map_err(|e| CliError::new(e.to_string()))?;
    let rows: Vec<&CommandHelp> = match command {
        None => COMMANDS.iter().collect(),
        Some(name) => vec![find(name).ok_or_else(|| CliError::new(unknown_command(name)))?],
    };

    if as_json {
        let commands = rows
            .iter()
            .map(|c| {
                Json::Object(vec![
                    ("name".into(), Json::Str(c.name.into())),
                    ("description".into(), Json::Str(c.description.into())),
                ])
            })
            .collect();
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("commands".into(), Json::Array(commands)),
        ])));
        return Ok(());
    }

    let text = match (command, rows.as_slice()) {
        (Some(_), [c]) => format!(
            "kluris {name} — {desc}\n\nRun 'kluris {name} --help' for full usage.\n",
            name = c.name,
            desc = c.description
        ),
        _ => {
            let mut out = String::from(
                "kluris — Turn AI agents into team subject matter experts\n\nCommands:\n",
            );
            for c in &rows {
                out.push_str(&format!("  {:<10} {}\n", c.name, c.description));
            }
            out.push_str(&format!(
                "\nConfig: {}\nBrains: {} registered\n",
                py_path_str(&path.to_string_lossy()),
                config.brains.len()
            ));
            out
        }
    };
    human::print(&text);
    Ok(())
}
