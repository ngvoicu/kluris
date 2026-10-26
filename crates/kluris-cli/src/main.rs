//! The `kluris` binary: parses arguments, calls `kluris-core`, prints.

mod commands;
mod error;
mod output;
mod resolve;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::error::CliError;
use crate::output::human;

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
        about = "Search a brain's neurons, glossary, and brain.md for a query string.",
        long_about = SEARCH_HELP
    )]
    Search {
        #[arg(value_name = "QUERY")]
        query: String,
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// Filter to neurons under this lobe
        #[arg(long, value_name = "LOBE", allow_hyphen_values = true)]
        lobe: Option<String>,
        /// Filter to neurons with this frontmatter tag
        #[arg(long, value_name = "TAG", allow_hyphen_values = true)]
        tag: Option<String>,
        /// Max number of results (default 10)
        #[arg(
            long,
            value_name = "INTEGER",
            default_value = "10",
            allow_hyphen_values = true,
            value_parser = commands::search::parse_limit
        )]
        limit: i64,
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

/// v2's `search` docstring.
const SEARCH_HELP: &str = "Search a brain's neurons, glossary, and brain.md for a query string.

Searches:
  - neuron file paths, titles, and frontmatter tags
  - neuron body text (with a 200-char snippet around the first match)
  - glossary terms and definitions
  - brain.md body content

Examples:
  kluris search \"oauth\"
  kluris search \"raw sql\" --lobe knowledge --json
  kluris search \"auth\" --tag oauth --limit 5";

/// The `--help` flag Click gives every command. It is a plain flag, not
/// clap's help action (which prints at once), so the whole line is parsed
/// first, as Click does ([`parse`]).
fn help_flag() -> Arg {
    Arg::new("help")
        .long("help")
        .help("Show this message and exit.")
        .action(ArgAction::SetTrue)
}

/// The root command with Click's surface (see [`Cli`]). Every option may
/// repeat, the last value winning, as in Click.
fn command() -> clap::Command {
    Cli::command()
        // Usage says `kluris` whatever argv[0] is (`kluris.exe` on Windows).
        .bin_name("kluris")
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args_override_self(true)
        .arg(
            Arg::new("version")
                .long("version")
                .help("Show the version and exit.")
                .action(ArgAction::SetTrue),
        )
        .arg(help_flag())
        .mut_subcommands(|sub| {
            sub.disable_help_flag(true)
                .args_override_self(true)
                .arg(help_flag())
        })
}

/// What the command line asks for.
enum Parsed {
    Run(Cli),
    /// `--help`: the help text of the command it was given to.
    Help(String),
    /// `--version`.
    Version(String),
}

/// Parse in Click's order: every token first (an unknown option, an option
/// without its value or a value given to a flag fail here, wherever
/// `--help` is), then the eager `--help` / `--version` in the order given,
/// and only then values are converted, required arguments checked and extra
/// arguments refused (so `--help` wins over those). The first step is a
/// parse with [`lenient`], which fails only where Click's parser fails.
fn parse(args: &[OsString]) -> Result<Parsed, clap::Error> {
    let mut first = lenient(command());
    let matches = first.try_get_matches_from_mut(args)?;
    if let Some(parsed) = eager(&mut first, &matches) {
        return Ok(parsed);
    }
    let mut root = command();
    let matches = root
        .try_get_matches_from_mut(args)
        .map_err(|e| with_usage(e, &mut root, args))?;
    Cli::from_arg_matches(&matches).map(Parsed::Run)
}

/// `cmd` with what Click checks after its parser left out: values are kept
/// as strings, nothing is required, every command takes extra arguments and
/// the root any command name.
fn lenient(cmd: clap::Command) -> clap::Command {
    fn relax(cmd: clap::Command) -> clap::Command {
        cmd.mut_args(|arg| {
            if arg.get_action().takes_values() {
                arg.value_parser(clap::builder::ValueParser::os_string())
                    .required(false)
            } else {
                arg
            }
        })
        .arg(
            Arg::new("extra-arguments")
                .num_args(1..)
                .action(ArgAction::Append)
                .value_parser(clap::builder::ValueParser::os_string())
                .hide(true),
        )
    }
    cmd.subcommand_required(false)
        .allow_external_subcommands(true)
        .mut_subcommands(relax)
}

fn flag(matches: &ArgMatches, id: &str) -> Option<usize> {
    match matches.try_get_one::<bool>(id) {
        Ok(Some(true)) => matches.index_of(id),
        _ => None,
    }
}

/// The root's `--help` or `--version` (the first given), else a command's
/// `--help`.
fn eager(root: &mut clap::Command, matches: &ArgMatches) -> Option<Parsed> {
    match (flag(matches, "help"), flag(matches, "version")) {
        (Some(h), Some(v)) if v < h => return Some(Parsed::Version(root.render_version())),
        (Some(_), _) => return Some(Parsed::Help(root.render_long_help().to_string())),
        (None, Some(_)) => return Some(Parsed::Version(root.render_version())),
        (None, None) => {}
    }
    let (name, sub_matches) = matches.subcommand()?;
    let sub = root.find_subcommand_mut(name)?;
    flag(sub_matches, "help")?;
    Some(Parsed::Help(sub.render_long_help().to_string()))
}

/// A value parser's error carries no usage line; Click prints the command's
/// (`Usage: kluris search [OPTIONS] QUERY`), so it is added.
fn with_usage(mut err: clap::Error, root: &mut clap::Command, args: &[OsString]) -> clap::Error {
    if err.kind() != ErrorKind::ValueValidation || err.get(ContextKind::Usage).is_some() {
        return err;
    }
    let name = args.get(1).map(|a| a.to_string_lossy().into_owned());
    if let Some(sub) = name.and_then(|n| root.find_subcommand_mut(n)) {
        let usage = sub.render_usage();
        err.insert(ContextKind::Usage, ContextValue::StyledStr(usage));
    }
    err
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::List { json } => commands::list::run(json),
        Command::WakeUp { brain, json } => commands::wake_up::run(brain.as_deref(), json),
        Command::Search {
            query,
            brain,
            lobe,
            tag,
            limit,
            json,
        } => commands::search::run(&commands::search::Options {
            query: &query,
            brain: brain.as_deref(),
            lobe: lobe.as_deref(),
            tag: tag.as_deref(),
            limit,
            json,
        }),
        Command::Status { brain, json } => commands::status::run(brain.as_deref(), json),
        Command::Help { command, json } => commands::help::run(command.as_deref(), json),
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let json = error::json_requested(args.get(1..).unwrap_or_default());
    match parse(&args) {
        Ok(Parsed::Run(cli)) => match run(cli) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => error::report(&err, json),
        },
        Ok(Parsed::Help(text) | Parsed::Version(text)) => {
            human::print(&text);
            ExitCode::SUCCESS
        }
        Err(err) => error::report_parse(&err, json),
    }
}
