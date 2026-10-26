//! `kluris help`: v2's command table (`cli.py:2010-2024`) without `pack`,
//! with `mri`'s description as fixed in v2.28.11. Commands Kluris 3 has not
//! ported yet are listed as v2 lists them.

/// One row of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandHelp {
    pub name: &'static str,
    pub description: &'static str,
}

const fn row(name: &'static str, description: &'static str) -> CommandHelp {
    CommandHelp { name, description }
}

/// Every command, in v2's order.
pub const COMMANDS: [CommandHelp; 12] = [
    row("create", "Create a new brain (--type, --remote, --no-git)"),
    row("register", "Register an existing brain directory on disk"),
    row("list", "List registered brains"),
    row(
        "status",
        "Show brain tree, recent changes, and neuron counts",
    ),
    row(
        "search",
        "Search a brain for a query string (neurons, glossary, brain.md)",
    ),
    row(
        "wake-up",
        "Compact brain snapshot for agent session bootstrap",
    ),
    row("companion", "Opt brains into embedded companion playbooks"),
    row(
        "dream",
        "Regenerate maps, auto-fix safe issues, and validate links",
    ),
    row(
        "mri",
        "Generate interactive brain visualization; prints a file:// link",
    ),
    row("remove", "Unregister a brain (keeps files on disk)"),
    row(
        "doctor",
        "Check prerequisites and refresh installed agent skills (--no-refresh to skip)",
    ),
    row("help", "Show this help"),
];

/// The row of a command, by its exact name.
pub fn find(name: &str) -> Option<&'static CommandHelp> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// `help <name>` for a name that is not in the table.
pub fn unknown_command(name: &str) -> String {
    format!("Unknown command: {name}")
}
