//! The `kluris` binary: parses arguments, calls `kluris-core`, prints.

mod commands;
mod error;
mod output;

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
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::List { json } => commands::list::run(json),
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let json = error::json_requested(args.get(1..).unwrap_or_default());
    match Cli::try_parse_from(&args) {
        Ok(cli) => match run(cli) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => error::report(&err, json),
        },
        Err(err) => error::report_parse(&err, json),
    }
}
