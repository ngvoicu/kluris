//! The worktree as it is now, as one tarball: `git archive` of HEAD, or of the
//! commit `git stash create` makes of the uncommitted tracked changes (which
//! touches neither the stash nor the worktree), with the run script and the
//! commit beside the files. Untracked files do not go, nor the build and parity
//! folders.

use std::fs;
use std::path::Path;
use std::process::Command;

use super::script::{RUN_FILE, TREE_FILE};

/// Never sent, tracked or not.
pub const EXCLUDED: [&str; 3] = ["target", "parity/.oracle", "parity/.venv"];

/// Writes the tarball of the worktree at `root` to `tarball`, with `run_script`
/// in it, and answers the commit it was made from.
pub fn pack(root: &Path, tarball: &Path, run_script: &str) -> Result<String, String> {
    // An identity of its own, so the stash commit can be made in any repository.
    let stash = git(
        root,
        "stash create",
        &[
            "-c",
            "user.name=kluris-ci",
            "-c",
            "user.email=kluris-ci@localhost",
            "stash",
            "create",
        ],
    )?;
    let commit = if stash.is_empty() {
        git(root, "rev-parse", &["rev-parse", "--verify", "HEAD"])?
    } else {
        stash
    };
    // The committed bytes, whatever line ends the checkout's git is told to
    // write: Git for Windows sets core.autocrlf=true system-wide.
    let mut args = vec![
        "-c".to_owned(),
        "core.autocrlf=false".to_owned(),
        "-c".to_owned(),
        "core.eol=lf".to_owned(),
        "archive".to_owned(),
        "--format=tar.gz".to_owned(),
        "-o".to_owned(),
        tarball.to_string_lossy().into_owned(),
        format!("--add-virtual-file={RUN_FILE}:{run_script}"),
        format!("--add-virtual-file={TREE_FILE}:{commit}\n"),
        commit.clone(),
        "--".to_owned(),
        ".".to_owned(),
    ];
    args.extend(EXCLUDED.iter().map(|path| format!(":(exclude){path}")));
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    git(root, "archive", &words)?;
    Ok(commit)
}

/// The toolchain channel the tree's `rust-toolchain.toml` pins.
pub fn channel(root: &Path) -> Result<String, String> {
    let path = root.join("rust-toolchain.toml");
    let text =
        fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    text.lines()
        .find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("channel")?
                .trim()
                .strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
        .filter(|channel| !channel.is_empty())
        .ok_or_else(|| format!("{} pins no channel", path.display()))
}

fn git(root: &Path, what: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|e| format!("could not start git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
