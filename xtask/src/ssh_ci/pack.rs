//! The worktree as it is now, as one tarball: `git archive` of HEAD, or of the
//! commit `git stash create` makes of the uncommitted tracked changes (which
//! touches neither the stash nor the worktree; the index's stat information
//! is refreshed first), with the run script and the commit beside the files. Untracked files do not go, nor the build folder.
//! The same for every machine: only the run script's name differs.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus, Output};

use super::plan::TREE_FILE;
use crate::exec_files;

/// Never sent, tracked or not.
pub const EXCLUDED: [&str; 1] = ["target"];

/// Writes the tarball of the worktree at `root` to `tarball`, with `run_script`
/// in it as `run_file`, and answers the commit it was made from.
#[cfg(test)]
pub fn pack(
    root: &Path,
    tarball: &Path,
    run_file: &str,
    run_script: &str,
) -> Result<String, String> {
    pack_with_files(root, tarball, run_file, run_script, &[])
}

/// [`pack`], with `files` (path in the tarball, text) beside the tree too.
pub fn pack_with_files(
    root: &Path,
    tarball: &Path,
    run_file: &str,
    run_script: &str,
    files: &[(String, String)],
) -> Result<String, String> {
    pack_by(
        OsStr::new("git"),
        root,
        tarball,
        run_file,
        run_script,
        files,
    )
}

/// [`pack_with_files`], with `git` as the git it runs.
pub fn pack_by(
    git: &OsStr,
    root: &Path,
    tarball: &Path,
    run_file: &str,
    run_script: &str,
    files: &[(String, String)],
) -> Result<String, String> {
    let git = |what: &str, args: &[&str]| run_git(git, root, what, args);
    // The stat information made current first: `stash create` takes a file
    // whose stat changed and whose bytes did not for a change, then finds
    // none, and ends with 1 saying nothing.
    git("update-index", &["update-index", "-q", "--refresh"])?;
    // An identity of its own, so the stash commit can be made in any repository.
    let stash = git(
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
        git("rev-parse", &["rev-parse", "--verify", "HEAD"])?
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
        format!("--add-virtual-file={run_file}:{run_script}"),
        format!("--add-virtual-file={TREE_FILE}:{commit}\n"),
    ];
    args.extend(
        files
            .iter()
            .map(|(path, text)| format!("--add-virtual-file={path}:{text}")),
    );
    args.extend([commit.clone(), "--".to_owned(), ".".to_owned()]);
    args.extend(EXCLUDED.iter().map(|path| format!(":(exclude){path}")));
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    git("archive", &words)?;
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

fn run_git(git: &OsStr, root: &Path, what: &str, args: &[&str]) -> Result<String, String> {
    let out = exec_files::output(
        Command::new(git)
            .args(args)
            .current_dir(root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE"),
    )
    .map_err(|e| format!("could not start git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {what} failed: {}", failure(&out)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// How a git that failed ended, then what it said on stderr and on stdout:
/// `status 1; stderr: ...; stdout: ...`, each stream left out when it said
/// nothing, and `, and nothing on stderr or stdout` when neither said a thing.
fn failure(out: &Output) -> String {
    let said: Vec<String> = [("stderr", &out.stderr), ("stdout", &out.stdout)]
        .into_iter()
        .filter_map(|(stream, bytes)| {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim();
            (!text.is_empty()).then(|| format!("{stream}: {text}"))
        })
        .collect();
    let ended = ended(out.status);
    if said.is_empty() {
        format!("{ended}, and nothing on stderr or stdout")
    } else {
        format!("{ended}; {}", said.join("; "))
    }
}

/// `status N`, or the signal that ended the process.
fn ended(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("ended by signal {signal}");
        }
    }
    status
        .code()
        .map_or_else(|| "no status".to_owned(), |code| format!("status {code}"))
}
