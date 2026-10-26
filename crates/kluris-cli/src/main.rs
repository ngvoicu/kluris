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
    override_usage = ROOT_USAGE,
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
        long_about = SEARCH_HELP,
        override_usage = "kluris search [OPTIONS] QUERY"
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
        /// Max number of results (default 10). Kept as given: only the
        /// winning occurrence is converted ([`converted`]).
        #[arg(
            long,
            value_name = "INTEGER",
            default_value = "10",
            allow_hyphen_values = true
        )]
        limit: String,
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

/// v2's root usage line (clap's would be `kluris [COMMAND]`).
const ROOT_USAGE: &str = "kluris [OPTIONS] COMMAND [ARGS]...";

/// The `--help` flag Click gives every command. It is a plain flag, not
/// clap's help action (which prints at once), so the whole line is parsed
/// first, as Click does ([`parse`]).
fn help_flag() -> Arg {
    Arg::new("help")
        .long("help")
        .help("Show this message and exit.")
        .action(ArgAction::SetTrue)
}

/// The root's own options, Click's: `--version` and `--help`. Every option
/// may repeat, the last value winning, as in Click.
fn root_options(cmd: clap::Command) -> clap::Command {
    // Usage says `kluris` whatever argv[0] is (`kluris.exe` on Windows).
    cmd.bin_name("kluris")
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
}

/// The root command with Click's surface (see [`Cli`]).
fn command() -> clap::Command {
    root_options(Cli::command()).mut_subcommands(|sub| {
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

/// Parse in Click's order:
/// 1. the root's own tokens, up to the command name ([`root_only`]): an
///    unknown root option fails here, then the root's eager `--help` /
///    `--version` act, the first given first, before anything after the
///    command name is looked at;
/// 2. the command's tokens ([`lenient`]): an unknown option, an option
///    without its value or a value given to a flag fail here, wherever
///    `--help` is, then the command's `--help` acts;
/// 3. the strict parse: required arguments, extra arguments, and the
///    conversion of each option's winning value only ([`converted`]).
///
/// Help is always rendered from the strict schema.
fn parse(args: &[OsString]) -> Result<Parsed, clap::Error> {
    let matches = root_only().try_get_matches_from(args)?;
    let help = first_root_token(args, "--help").filter(|_| flag(&matches, "help"));
    let version = first_root_token(args, "--version").filter(|_| flag(&matches, "version"));
    match (help, version) {
        (Some(h), Some(v)) if v < h => return Ok(Parsed::Version(command().render_version())),
        (Some(_), _) => return Ok(Parsed::Help(command().render_long_help().to_string())),
        (None, Some(_)) => return Ok(Parsed::Version(command().render_version())),
        (None, None) => {}
    }

    let matches = lenient(command()).try_get_matches_from(args)?;
    if let Some((name, sub_matches)) = matches.subcommand()
        && flag(sub_matches, "help")
    {
        let mut strict = command();
        strict.build();
        if let Some(sub) = strict.find_subcommand_mut(name) {
            return Ok(Parsed::Help(sub.render_long_help().to_string()));
        }
    }

    let mut root = command();
    let matches = root
        .try_get_matches_from_mut(args)
        .map_err(|e| with_usage(e, &mut root, args))?;
    let cli = Cli::from_arg_matches(&matches)?;
    converted(&cli).map_err(|e| with_usage(e, &mut root, args))?;
    Ok(Parsed::Run(cli))
}

/// The root alone, as Click's group parses it: its own options, and the
/// command name with everything after it taken as it is.
fn root_only() -> clap::Command {
    root_options(clap::Command::new("kluris"))
        .override_usage(ROOT_USAGE)
        .allow_external_subcommands(true)
}

/// Where `token` first appears among the root's own tokens (before the
/// command name); the first occurrence decides the eager order, as in Click.
fn first_root_token(args: &[OsString], token: &str) -> Option<usize> {
    args.iter()
        .skip(1)
        .take_while(|a| a.to_string_lossy().starts_with('-') && *a != "--")
        .position(|a| a == token)
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

fn flag(matches: &ArgMatches, id: &str) -> bool {
    matches!(matches.try_get_one::<bool>(id), Ok(Some(true)))
}

/// Convert the options that are not strings, from their winning (last)
/// value only: Click converts once, after parsing, where a clap value
/// parser would check every repeated occurrence. The error is the one the
/// value parser gives (`Invalid value for '--limit': 'x' is not a valid
/// integer.`).
fn converted(cli: &Cli) -> Result<(), clap::Error> {
    if let Command::Search { limit, .. } = &cli.command {
        search_limit(limit)?;
    }
    Ok(())
}

/// `search --limit` as an integer.
fn search_limit(raw: &str) -> Result<i64, clap::Error> {
    use clap::builder::TypedValueParser;
    let mut root = command();
    root.build();
    let search = root.find_subcommand("search");
    let arg = search.and_then(|s| s.get_arguments().find(|a| a.get_id() == "limit"));
    match search {
        Some(search) => {
            commands::search::parse_limit.parse_ref(search, arg, std::ffi::OsStr::new(raw))
        }
        None => commands::search::parse_limit(raw)
            .map_err(|reason| clap::Error::raw(ErrorKind::ValueValidation, reason)),
    }
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
            // Checked by `parse` (`converted`).
            limit: search_limit(&limit).map_err(|e| CliError::new(e.to_string()))?,
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
