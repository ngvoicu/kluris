//! The `kluris` binary: parses arguments, calls `kluris-core`, prints.

mod commands;
mod error;
mod output;
mod resolve;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Arg, ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::error::CliError;

/// The root command. Its surface is Click's, as in v2 ([`command`]): `--help`
/// on every command and `--version` on the root, no `-h`, `-V` or automatic
/// `help` subcommand; `kluris help` is Kluris's own command.
#[derive(Debug, Parser)]
#[command(
    name = "kluris",
    version = "3.0.0-dev",
    about = "Kluris — Turn AI agents into team subject matter experts.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "List registered brains.")]
    List {
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        name = "wake-up",
        args_override_self = true,
        about = "Compact brain snapshot for agent session bootstrap.",
        long_about = WAKE_UP_HELP
    )]
    WakeUp {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Show brain status and recent changes."
    )]
    Status {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Show help for kluris commands.")]
    Help {
        command: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
}

/// v2's `wake-up` docstring.
const WAKE_UP_HELP: &str = "Compact brain snapshot for agent session bootstrap.

Emits a tight view of the brain's live state: the brain.md body, lobes with \
neuron counts, the 5 most recently updated neurons, the glossary, and any \
deprecation warnings. Designed to be called when the generated per-brain skill \
is loaded or invoked so the agent has a fast index without walking brain.md, \
glossary.md, and every map.md.

  kluris wake-up                 # only-brain, text output
  kluris wake-up --brain foo     # target a specific brain when 2+ are registered
  kluris wake-up --json          # machine-readable (for agents)";

/// The `--help` flag Click gives every command.
fn help_flag() -> Arg {
    Arg::new("help")
        .long("help")
        .help("Show this message and exit.")
        .action(ArgAction::Help)
}

/// The root command with Click's surface (see [`Cli`]).
fn command() -> clap::Command {
    Cli::command()
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("version")
                .long("version")
                .help("Show the version and exit.")
                .action(ArgAction::Version),
        )
        .arg(help_flag())
        .mut_subcommands(|sub| sub.disable_help_flag(true).arg(help_flag()))
}

fn parse(args: &[OsString]) -> Result<Cli, clap::Error> {
    let matches = command().try_get_matches_from(args)?;
    Cli::from_arg_matches(&matches)
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
        Command::Status { brain, json } => commands::status::run(brain.as_deref(), json),
        Command::Help { command, json } => commands::help::run(command.as_deref(), json),
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
    match parse(&args) {
        Ok(cli) => match run(cli) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => error::report(&err, json),
        },
        Err(err) => error::report_parse(&err, json),
    }
}
