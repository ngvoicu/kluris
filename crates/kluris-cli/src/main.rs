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
    #[command(
        about = "Manage embedded companion playbooks per brain.",
        override_usage = "kluris companion [OPTIONS] COMMAND [ARGS]...",
        arg_required_else_help = true
    )]
    Companion {
        #[command(subcommand)]
        action: CompanionCommand,
    },
    #[command(
        args_override_self = true,
        about = "Brain maintenance — regenerate maps, update dates, auto-fix safe issues, validate remaining links."
    )]
    Dream {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
        #[arg(
            long = "broken-only",
            help = "In --json mode, return only broken-synapse entries for the agent to repair."
        )]
        broken_only: bool,
    },
    #[command(
        args_override_self = true,
        about = "Generate interactive brain visualization."
    )]
    Mri {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// Output HTML file path
        #[arg(long = "output", value_name = "OUTPUT", allow_hyphen_values = true)]
        output: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Check prerequisites and refresh installed agent skills.",
        long_about = DOCTOR_HELP
    )]
    Doctor {
        /// Skip refreshing installed agent skills
        #[arg(long = "no-refresh")]
        no_refresh: bool,
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

#[derive(Debug, Subcommand)]
enum CompanionCommand {
    #[command(
        args_override_self = true,
        about = "List known companions, installed runtime copies, and brain opt-ins."
    )]
    List {
        /// Brain name or 'all'
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Opt one or more brains into an embedded companion playbook.",
        long_about = COMPANION_ADD_HELP
    )]
    Add {
        #[arg(
            value_name = commands::companion::NAME_CHOICES,
            value_parser = commands::companion::parse_name
        )]
        name: Option<String>,
        /// Brain name or 'all'
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Remove a companion opt-in from one or more brains.",
        long_about = COMPANION_REMOVE_HELP
    )]
    Remove {
        #[arg(
            value_name = commands::companion::NAME_CHOICES,
            value_parser = commands::companion::parse_name
        )]
        name: Option<String>,
        /// Brain name or 'all'
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
}

/// v2's `companion add` docstring.
const COMPANION_ADD_HELP: &str = "Opt one or more brains into an embedded companion playbook.

Runs an interactive picker when no NAME is given. Pass NAME (or use --json) \
to skip the wizard.";

/// v2's `companion remove` docstring.
const COMPANION_REMOVE_HELP: &str = "Remove a companion opt-in from one or more brains.

Runs an interactive picker listing currently-enabled companions when no \
NAME is given. The global companion copy under ~/.kluris/companions is \
kept for cheap reuse.";

/// v2's `doctor` docstring, for what v3's doctor does.
const DOCTOR_HELP: &str = "Check prerequisites and refresh installed agent skills.

Checks that git runs, that the first kluris on PATH is this one, and that \
the config and every registered brain's kluris.yml read. Then refreshes the \
companion playbooks and re-installs every brain's agent skill \
(~/.agents/skills, plus the folders of the selected agents), removing the \
copies Kluris 2 left in folders it no longer uses. This is the step after \
an upgrade: run `kluris doctor` and everything is back in sync.

Pass --no-refresh to run only the read-only checks.";

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
    root_options(Cli::command()).mut_subcommands(click_surface)
}

/// A command (and its own commands) with Click's surface: a plain `--help`
/// flag, repeatable options, no automatic `help` command.
fn click_surface(cmd: clap::Command) -> clap::Command {
    cmd.disable_help_flag(true)
        .disable_help_subcommand(true)
        .args_override_self(true)
        .arg(help_flag())
        .mut_subcommands(click_surface)
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
    if let Some(path) = help_path(&matches) {
        let mut strict = command();
        strict.build();
        let mut sub = Some(&mut strict);
        for name in &path {
            sub = sub.and_then(|cmd| cmd.find_subcommand_mut(name));
        }
        if let Some(sub) = sub {
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

/// The command path (`["companion", "add"]`) of the outermost command
/// given `--help`: a group's own `--help` before its command's, as Click
/// parses a group's options before it looks at the command.
fn help_path(matches: &ArgMatches) -> Option<Vec<String>> {
    let mut path = Vec::new();
    let mut matches = matches;
    while let Some((name, sub_matches)) = matches.subcommand() {
        path.push(name.to_string());
        if flag(sub_matches, "help") {
            return Some(path);
        }
        matches = sub_matches;
    }
    None
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
        if cmd.has_subcommands() {
            // A group (`companion`): any command name parses here.
            return cmd
                .subcommand_required(false)
                .allow_external_subcommands(true)
                .mut_subcommands(relax);
        }
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
    // The command path: each command name in turn (options skipped) while
    // it names a command of the one before (`companion add`).
    let mut path = Vec::new();
    let mut cmd: &clap::Command = root;
    for arg in args.iter().skip(1) {
        let arg = arg.to_string_lossy();
        if arg.starts_with('-') {
            continue;
        }
        match cmd.find_subcommand(arg.as_ref()) {
            Some(sub) => {
                path.push(sub.get_name().to_string());
                cmd = sub;
            }
            None => break,
        }
    }
    if path.is_empty() {
        return err;
    }
    let mut sub = root;
    for name in &path {
        let Some(next) = sub.find_subcommand_mut(name) else {
            return err;
        };
        sub = next;
    }
    let usage = sub.render_usage();
    err.insert(ContextKind::Usage, ContextValue::StyledStr(usage));
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
        Command::Companion { action } => match action {
            CompanionCommand::List { brain, json } => {
                commands::companion::list(brain.as_deref(), json)
            }
            CompanionCommand::Add { name, brain, json } => {
                commands::companion::add(name.as_deref(), brain.as_deref(), json)
            }
            CompanionCommand::Remove { name, brain, json } => {
                commands::companion::remove(name.as_deref(), brain.as_deref(), json)
            }
        },
        Command::Dream {
            brain,
            json,
            broken_only,
        } => commands::dream::run(brain.as_deref(), json, broken_only),
        Command::Mri {
            brain,
            output,
            json,
        } => commands::mri::run(brain.as_deref(), output.as_deref(), json),
        Command::Doctor { no_refresh, json } => commands::doctor::run(no_refresh, json),
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
