//! The `kluris` binary: parses arguments, calls `kluris-core`, prints.

mod commands;
mod error;
mod output;
mod platform;
mod prompt;
mod resolve;
mod skills;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::error::CliError;
use crate::output::human;

// The root command. Its surface is Click's, as in v2 (`command`): `--help`
// on every command and `--version` on the root, no `-h`, `-V` or automatic
// `help` subcommand; `kluris help` is Kluris's own command.
#[derive(Debug, Parser)]
#[command(
    name = "kluris",
    version,
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
    #[command(
        args_override_self = true,
        about = "Create a new brain.",
        long_about = CREATE_HELP,
        override_usage = "kluris create [OPTIONS] [NAME]"
    )]
    Create {
        #[arg(value_name = "NAME")]
        name: Option<String>,
        /// What this brain covers (one sentence)
        #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
        description: Option<String>,
        /// Directory to create the brain in (default: current dir)
        #[arg(long, value_name = "PATH", allow_hyphen_values = true)]
        path: Option<String>,
        // Kept as given: only the winning occurrence is converted
        // (`converted`).
        #[arg(
            long = "type",
            value_name = "[product-group|personal|product|research|blank]",
            allow_hyphen_values = true,
            help = "Brain type"
        )]
        brain_type: Option<String>,
        /// Optional git remote URL (default: local git only)
        #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
        remote: Option<String>,
        /// Default git branch
        #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
        branch: Option<String>,
        /// Skip git entirely (default: local git only)
        #[arg(long = "no-git")]
        no_git: bool,
        // Checked like the type (`converted`).
        #[arg(
            long = "from-config",
            value_name = "PATH",
            allow_hyphen_values = true,
            help = "Custom YAML config file for structure"
        )]
        from_config: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Register an existing brain directory on disk.",
        long_about = REGISTER_HELP,
        override_usage = "kluris register [OPTIONS] [SOURCE]"
    )]
    Register {
        #[arg(value_name = "SOURCE")]
        source: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
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
        // Kept as given: only the winning occurrence is converted
        // (`converted`).
        /// Max number of results (default 10)
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
        about = "Unregister a brain (does not delete files).",
        long_about = REMOVE_HELP,
        override_usage = "kluris remove [OPTIONS] BRAIN_NAME"
    )]
    Remove {
        #[arg(value_name = "BRAIN_NAME")]
        brain_name: String,
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
    #[command(
        args_override_self = true,
        about = "Commit approved brain files and the maps they change.",
        long_about = COMMIT_HELP
    )]
    Commit {
        /// Specific brain
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// Brain-relative files written or deleted, comma-separated
        #[arg(
            long = "paths",
            value_name = "PATHS",
            allow_hyphen_values = true,
            required = true
        )]
        paths: String,
        /// Commit message (default: kluris: update <paths>)
        #[arg(
            short = 'm',
            long = "message",
            value_name = "MSG",
            allow_hyphen_values = true
        )]
        message: Option<String>,
        /// Skip the pre-commit and commit-msg hooks
        #[arg(long = "no-verify")]
        no_verify: bool,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        args_override_self = true,
        about = "Fetch, merge and push brains with their remotes.",
        long_about = SYNC_HELP
    )]
    Sync {
        /// Brain name or 'all'
        #[arg(long = "brain", value_name = "BRAIN", allow_hyphen_values = true)]
        brain: Option<String>,
        /// Skip the pre-merge-commit hook
        #[arg(long = "no-verify")]
        no_verify: bool,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        name = "self-update",
        args_override_self = true,
        about = "Update kluris to the newest release.",
        long_about = SELF_UPDATE_HELP
    )]
    SelfUpdate {
        /// Only show the installed and the newest version
        #[arg(long)]
        check: bool,
        /// Install this release instead of the newest (3.1.0 or v3.1.0)
        #[arg(long = "version", value_name = "VERSION", allow_hyphen_values = true)]
        version: Option<String>,
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    #[command(
        name = "self-uninstall",
        args_override_self = true,
        about = "Remove the installed kluris.",
        long_about = SELF_UNINSTALL_HELP
    )]
    SelfUninstall {
        /// Also remove ~/.kluris and every Kluris-written agent skill
        #[arg(long)]
        purge: bool,
        /// Do not ask before --purge
        #[arg(long)]
        yes: bool,
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

/// v2's `create` docstring.
const CREATE_HELP: &str = "Create a new brain.

Prompts for anything not provided via flags. Use --json to skip all prompts.

  kluris create                    # full wizard
  kluris create my-brain           # prompts for description, location, type, git
  kluris create my-brain --type personal --path ~/brains --no-git  # no prompts
  kluris create team-brain --remote git@github.com:team/brain.git";

/// v2's `register` docstring.
const REGISTER_HELP: &str = "Register an existing brain directory on disk.

The brain stays where it is -- registration is in-place, no copy. For brains \
hosted on git, use ``git clone <url> <path>`` first, then ``kluris register \
<path>``.

  kluris register ~/brains/acme-sme";

/// v2's `remove` docstring.
const REMOVE_HELP: &str = "Unregister a brain (does not delete files).

Always preserves the files on disk. Git state is the user's responsibility \
after unregistering.";

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

/// `sync` (new in v3.1).
const SYNC_HELP: &str = "Fetch, merge and push brains with their remotes.

For each brain: fetch its branch's upstream (never prompting), then \
fast-forward when behind, push when ahead, and when both sides have new \
commits merge them: different files, the generated maps, dates, related and \
tags lists, and edits to different parts of a neuron merge on their own; \
overlapping edits, a delete against an edit, different parents and other \
frontmatter changed on both sides are left for you, with nothing changed \
until `kluris resolve`. A file with uncommitted changes is never \
overwritten: if incoming changes touch one, that brain is not synced. The \
repository's pre-merge-commit, post-merge and pre-push hooks run \
(--no-verify skips pre-merge-commit). Needs git 2.38 or newer.

  kluris sync --brain acme
  kluris sync --brain all --json";

/// `self-update` (new in v3).
const SELF_UPDATE_HELP: &str = "Update kluris to the newest release.

Downloads this platform's archive from the newest GitHub release of \
ngvoicu/kluris, checks its SHA-256 against the release's sha256.sum (and \
GitHub's digest), replaces this binary in place, then runs the new \
`kluris doctor`. Works only on the copy the Kluris installer put in \
~/.kluris/bin (its install receipt says where).

Set KLURIS_GITHUB_TOKEN to a GitHub token when GitHub's rate limit for \
unauthenticated requests (60 an hour per address) is in the way. Set \
KLURIS_GITHUB_API_URL to use a GitHub API mirror or GitHub Enterprise \
instead of https://api.github.com (https only; the checksums are still \
checked).";

/// `self-uninstall` (new in v3).
const SELF_UNINSTALL_HELP: &str = "Remove the installed kluris.

Removes what the Kluris installer wrote: the binary in ~/.kluris/bin, the \
PATH setup (the env scripts, the line each shell profile got, the Windows \
user PATH entry) and the install receipt. Keeps ~/.kluris/config.yml, the \
companions, the agent skills and every brain.

--purge also removes ~/.kluris and every Kluris-written agent skill, after \
asking (--yes skips the question; it is required when not interactive). \
Brains are never removed.";

/// `commit` (new in v3.1).
const COMMIT_HELP: &str = "Commit approved brain files and the maps they change.

Commits only the given files (written or deleted, relative to the brain) and \
the map.md/brain.md files the map step regenerates because of them. Every \
other changed file is left as it is, staged or not, and listed as left \
uncommitted. The author is your git identity; the message ends with a \
Kluris-Approved-By trailer (git user.name) and, when KLURIS_AGENT is set, a \
Co-Authored-By trailer naming the agent. The maps are built from what is \
committed, so an uncommitted file never shows up in them. The repository's \
hooks run as git commit runs them (--no-verify skips pre-commit and \
commit-msg). Needs git 2.38 or newer.

  kluris commit --brain acme --paths knowledge/auth.md,knowledge/oauth.md
  kluris commit --brain acme --paths old.md -m \"Remove the old note\" --json";

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
    converted(&cli, args).map_err(|e| with_usage(e, &mut root, args))?;
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
fn converted(cli: &Cli, args: &[OsString]) -> Result<(), clap::Error> {
    match &cli.command {
        Command::Search { limit, .. } => {
            search_limit(limit)?;
        }
        Command::Create {
            brain_type,
            from_config,
            ..
        } => {
            // Click converts the options in the order they first appear.
            let mut checks = [
                (
                    first_option(args, "--type"),
                    brain_type.as_deref().map(check_type),
                ),
                (
                    first_option(args, "--from-config"),
                    from_config.as_deref().map(check_from_config),
                ),
            ];
            checks.sort_by_key(|(at, _)| *at);
            for (_, check) in checks {
                if let Some(Err(message)) = check {
                    return Err(clap::Error::raw(ErrorKind::ValueValidation, message));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Where `--name` (or `--name=...`) first appears in the command line.
fn first_option(args: &[OsString], name: &str) -> usize {
    args.iter()
        .position(|a| {
            let a = a.to_string_lossy();
            a == name
                || a.strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with('='))
        })
        .unwrap_or(usize::MAX)
}

/// `create --type` as Click's `Choice` converts it.
fn check_type(value: &str) -> Result<(), String> {
    if kluris_core::scaffold::is_brain_type(value) {
        return Ok(());
    }
    Err(format!(
        "Invalid value for '--type': {}\n",
        kluris_core::scaffold::not_a_brain_type(value)
    ))
}

/// `create --from-config` as Click's `Path(exists=True)` converts it.
fn check_from_config(value: &str) -> Result<(), String> {
    if std::path::Path::new(value).exists() {
        return Ok(());
    }
    Err(format!(
        "Invalid value for '--from-config': Path {} does not exist.\n",
        kluris_core::pyrepr::py_repr_str(value)
    ))
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
    if err.get(ContextKind::InvalidArg).is_none() {
        // A message of our own (`create --type`): clap renders the usage
        // of a raw error once it knows the command.
        return err.format(sub);
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
        Command::Create {
            name,
            description,
            path,
            brain_type,
            remote,
            branch,
            no_git,
            from_config,
            json,
        } => commands::create::run(commands::create::Options {
            input: kluris_core::create::CreateInput {
                name,
                description,
                path,
                brain_type,
                remote,
                branch,
                no_git,
            },
            from_config,
            json,
        }),
        Command::Register { source, json } => commands::register::run(source.as_deref(), json),
        Command::Remove { brain_name, json } => commands::remove::run(&brain_name, json),
        Command::Doctor { no_refresh, json } => commands::doctor::run(no_refresh, json),
        Command::SelfUpdate {
            check,
            version,
            json,
        } => commands::self_update::run(check, version.as_deref(), json),
        Command::SelfUninstall { purge, yes, json } => {
            commands::self_uninstall::run(purge, yes, json)
        }
        Command::Commit {
            brain,
            paths,
            message,
            no_verify,
            json,
        } => commands::commit::run(&commands::commit::Options {
            brain: brain.as_deref(),
            paths: &paths,
            message: message.as_deref(),
            no_verify,
            json,
        }),
        Command::Sync {
            brain,
            no_verify,
            json,
        } => commands::sync::run(brain.as_deref(), no_verify, json),
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
