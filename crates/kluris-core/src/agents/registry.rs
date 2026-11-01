//! Where each agent loads skills from, as each agent reads them.
//!
//! Every skill goes to `~/.agents/skills`, which most agents read. The
//! registry lists only the agents that need a folder of their own; the other
//! names a `commands_for` may carry are served by `~/.agents/skills` alone.

use std::fs;
use std::path::{Path, PathBuf};

/// The folder every skill is written to, relative to HOME.
pub const UNIVERSAL: &str = ".agents/skills";

/// Agents that need their own folder, sorted by name (the order installs
/// go in). `copilot` needs one only while `COPILOT_HOME` is set: Copilot CLI
/// then stops reading `~/.agents/skills`.
pub const REGISTRY: [&str; 4] = ["antigravity", "claude", "copilot", "hermes"];

/// Agents served by `~/.agents/skills` alone: accepted in `commands_for`,
/// they write nothing extra. `windsurf` (Windsurf became Devin Desktop) is
/// read as `devin` by the config reader and listed for old configs.
pub const UNIVERSAL_ONLY: [&str; 6] = ["codex", "cursor", "devin", "junie", "kilocode", "windsurf"];

/// The skill folders of one registry agent, for `home` and an optional
/// `COPILOT_HOME`: `~/.claude/skills`; `~/.hermes/skills` and the `skills`
/// folder of every directory in `~/.hermes/profiles`, sorted by name;
/// `~/.gemini/config/skills` for Antigravity (one folder only: its CLI also
/// reads `~/.gemini/skills` and `~/.gemini/antigravity-cli/skills`, so a
/// second copy would show twice); `$COPILOT_HOME/skills` (none when unset).
pub fn skill_folders(agent: &str, home: &Path, copilot_home: Option<&Path>) -> Vec<PathBuf> {
    match agent {
        "claude" => vec![home.join(".claude").join("skills")],
        "antigravity" => vec![home.join(".gemini").join("config").join("skills")],
        "copilot" => copilot_home.map(|c| c.join("skills")).into_iter().collect(),
        "hermes" => {
            let mut folders = vec![home.join(".hermes").join("skills")];
            folders.extend(
                sorted_dirs(&home.join(".hermes").join("profiles"))
                    .into_iter()
                    .map(|p| p.join("skills")),
            );
            folders
        }
        _ => Vec::new(),
    }
}

/// The directories directly inside `dir` (a symlink to one counts, as
/// Python's `Path.is_dir()` counts it), sorted by name; none when `dir` cannot be read.
fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}
