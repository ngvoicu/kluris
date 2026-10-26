//! Where each agent loads skills from (research-04, confirmed live on this
//! Mac by research-07; SPEC decisions 2026-10-24 and 2026-10-25).
//!
//! Every skill goes to `~/.agents/skills`, which most agents read. The
//! registry lists only the agents that need a folder of their own; the other
//! names a `commands_for` may carry are served by `~/.agents/skills` alone.

use std::fs;
use std::path::{Path, PathBuf};

/// The folder every skill is written to, relative to HOME.
pub const UNIVERSAL: &str = ".agents/skills";

/// Agents that need their own folder, sorted by name (the order v2 installs
/// in). `copilot` needs one only while `COPILOT_HOME` is set: Copilot CLI
/// then stops reading `~/.agents/skills`.
pub const REGISTRY: [&str; 4] = ["antigravity", "claude", "copilot", "hermes"];

/// Agents served by `~/.agents/skills` alone: accepted in `commands_for`,
/// they write nothing extra. `windsurf` (Windsurf became Devin Desktop) is
/// read as `devin` by the config reader and listed for old configs.
pub const UNIVERSAL_ONLY: [&str; 7] = [
    "codex", "cursor", "devin", "gemini", "junie", "kilocode", "windsurf",
];

/// Skill folders v2 wrote that v3 no longer uses, relative to HOME; their
/// copies duplicate `~/.agents/skills` (research-07: Codex lists two enabled
/// entries, Devin an `agents:` and a `claude:` copy). `~/.copilot/skills` is
/// only one while `COPILOT_HOME` is unset. The Windsurf folders are found
/// with [`windsurf_folders`].
pub const V2_ONLY_SKILL_FOLDERS: [&str; 7] = [
    ".cursor/skills",
    ".codex/skills",
    ".copilot/skills",
    ".gemini/skills",
    ".kilo/skills",
    ".kilocode/skills",
    ".junie/skills",
];

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

/// The v2 Windsurf folders under `~/.codeium/windsurf*` (Windsurf,
/// Windsurf Next, Windsurf Insiders): `(skills folders, workflow folders)`.
pub fn windsurf_folders(home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let roots: Vec<PathBuf> = sorted_dirs(&home.join(".codeium"))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("windsurf"))
        })
        .collect();
    (
        roots.iter().map(|r| r.join("skills")).collect(),
        roots.iter().map(|r| r.join("global_workflows")).collect(),
    )
}

/// The directories directly inside `dir` (a symlink to one counts, as in
/// v2's `Path.is_dir()`), sorted by name; none when `dir` cannot be read.
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
