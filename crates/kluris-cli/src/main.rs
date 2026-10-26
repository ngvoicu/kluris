//! The `kluris` binary: parses arguments, calls `kluris-core`, prints.

mod commands;
mod error;
mod output;
mod resolve;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::error::CliError;

/// Kluris — Turn AI agents into team subject matter experts.
#[derive(Debug, Parser)]
#[command(name = "kluris", version = "3.0.0-dev", arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List registered brains.
    List {
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    /// Compact brain snapshot for agent session bootstrap.
    #[command(name = "wake-up", args_override_self = true)]
    WakeUp {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
}

/// `kluris __resolve [--brain NAME] [--allow-all] [--json]`: the resolver of
/// a fan-out command, for the parity tests until dream, status, mri and the
/// companion commands exist. Only with `KLURIS_TEST_HOOKS=1`; otherwise the
/// name is an unknown command, as in v2.
#[derive(Debug, Parser)]
#[command(name = "kluris __resolve", args_override_self = true)]
struct TestResolve {
    #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
    brain: Option<String>,
    #[arg(long)]
    allow_all: bool,
    #[arg(long)]
    json: bool,
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::List { json } => commands::list::run(json),
        Command::WakeUp { brain, json } => commands::wake_up::run(brain.as_deref(), json),
    }
}

fn test_hooks(args: &[OsString]) -> bool {
    args.get(1).is_some_and(|a| a == "__resolve")
        && std::env::var_os("KLURIS_TEST_HOOKS").is_some_and(|v| v == "1")
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let json = error::json_requested(args.get(1..).unwrap_or_default());
    if test_hooks(&args) {
        return match TestResolve::try_parse_from(args.get(1..).unwrap_or_default()) {
            Ok(t) => match resolve::print_resolved(t.brain.as_deref(), t.allow_all, t.json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => error::report(&err, json),
            },
            Err(err) => error::report_parse(&err, json),
        };
    }
    match Cli::try_parse_from(&args) {
        Ok(cli) => match run(cli) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => error::report(&err, json),
        },
        Err(err) => error::report_parse(&err, json),
    }
}
