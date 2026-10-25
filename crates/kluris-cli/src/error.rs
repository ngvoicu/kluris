//! The JSON error envelope and exit codes (research-03 §1.0).
//!
//! - `--json` reaching a subcommand: any error prints
//!   `{"ok": false, "error": "<message>"}` on stdout and exits 1.
//! - Otherwise a Kluris error prints `Error: <message>` on stderr and exits 1,
//!   and a usage error prints usage on stderr and exits 2.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};

use crate::output::json::{self, Json};

/// An error a command returns: printed as `Error: <message>` (exit 1), or as
/// the JSON envelope under `--json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// `{"ok": false, "error": <message>}` on stdout, exit 1.
fn print_envelope(message: &str) -> ExitCode {
    let envelope = Json::Object(vec![
        ("ok".into(), Json::Bool(false)),
        ("error".into(), Json::Str(message.into())),
    ]);
    let _ = io::stdout().write_all(json::to_line(&envelope).as_bytes());
    ExitCode::from(1)
}

/// Report a command's error.
pub fn report(err: &CliError, json: bool) -> ExitCode {
    if json {
        return print_envelope(&err.0);
    }
    let _ = writeln!(io::stderr(), "Error: {}", err.0);
    ExitCode::from(1)
}

/// Report an argument-parsing error: help and version print normally and
/// exit 0, the envelope applies under `--json`, anything else is a usage
/// error on stderr with exit 2.
pub fn report_parse(err: &clap::Error, json: bool) -> ExitCode {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = err.print();
            return ExitCode::SUCCESS;
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = err.print();
            return ExitCode::from(2);
        }
        _ => {}
    }
    let message = usage_message(err);
    if json {
        return print_envelope(&message);
    }
    let rendered = err.render().to_string();
    let usage = rendered.lines().find(|l| l.starts_with("Usage:"));
    let mut text = String::new();
    if let Some(usage) = usage {
        text.push_str(usage);
        text.push('\n');
        text.push_str(&format!(
            "Try '{} --help' for help.\n\n",
            command_path(usage)
        ));
    }
    text.push_str(&format!("Error: {message}\n"));
    let _ = io::stderr().write_all(text.as_bytes());
    ExitCode::from(2)
}

/// The command path in a clap usage line (`Usage: kluris list [OPTIONS]`
/// gives `kluris list`).
fn command_path(usage: &str) -> String {
    let words: Vec<&str> = usage
        .trim_start_matches("Usage:")
        .split_whitespace()
        .take_while(|w| !w.starts_with(['[', '<']) && *w != "COMMAND")
        .collect();
    if words.is_empty() {
        "kluris".into()
    } else {
        words.join(" ")
    }
}

/// Click's wording for the usage errors agents are likely to hit; clap's
/// own message for the rest.
fn usage_message(err: &clap::Error) -> String {
    let context = |kind| match err.get(kind) {
        Some(ContextValue::String(s)) => Some(s.clone()),
        _ => None,
    };
    match err.kind() {
        ErrorKind::InvalidSubcommand => {
            if let Some(name) = context(ContextKind::InvalidSubcommand) {
                return format!("No such command '{name}'.");
            }
        }
        ErrorKind::UnknownArgument => {
            if let Some(arg) = context(ContextKind::InvalidArg) {
                let option = arg.split_once('=').map_or(arg.as_str(), |(o, _)| o);
                return format!("No such option '{option}'.");
            }
        }
        _ => {}
    }
    let rendered = err.render().to_string();
    let first = rendered.lines().next().unwrap_or_default();
    first.trim_start_matches("error: ").to_string()
}

/// Whether errors go out as the JSON envelope: a literal `--json` token, and
/// the root options before the first command token parse (v2's Click group
/// only reaches its envelope once a command name is being dispatched; a bare
/// `kluris --json` is a root usage error, exit 2).
pub fn json_requested(args: &[OsString]) -> bool {
    if !args.iter().any(|a| a == "--json") {
        return false;
    }
    for arg in args {
        let arg = arg.to_string_lossy();
        if !arg.starts_with('-') {
            return true;
        }
        if !ROOT_FLAGS.contains(&arg.as_ref()) {
            return false;
        }
    }
    false
}

/// The options the root command accepts before a command name.
const ROOT_FLAGS: [&str; 2] = ["--help", "--version"];

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn json_after_a_command_token() {
        assert!(json_requested(&args(&["list", "--json"])));
        assert!(json_requested(&args(&["bogus", "--json"])));
        assert!(json_requested(&args(&["list", "--bogus", "--json"])));
        assert!(json_requested(&args(&["companion", "add", "x", "--json"])));
    }

    #[test]
    fn no_json_token() {
        assert!(!json_requested(&args(&[])));
        assert!(!json_requested(&args(&["list"])));
        assert!(!json_requested(&args(&["list", "--json=1"])));
    }

    #[test]
    fn root_level_json_is_a_usage_error() {
        assert!(!json_requested(&args(&["--json"])));
        assert!(!json_requested(&args(&["--json", "list"])));
        assert!(!json_requested(&args(&["--bogus", "list", "--json"])));
    }
}
