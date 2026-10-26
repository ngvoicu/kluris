//! The git layer: every git operation Kluris runs, through the `git` CLI
//! (never a library), behind the [`Git`] trait so tests and later phases can
//! swap it.
//!
//! The calls are v2's (`core/git.py`, research-03 §1.15), with four fixes:
//! - `log` reads NUL-separated fields: v2 split `%H|%s|%aI` on `|`, so a
//!   subject containing `|` was cut short and its date corrupted.
//! - `-c core.quotePath=false` on the calls that print paths: v2 got
//!   non-ASCII names C-quoted (`"\303\251.md"`).
//! - `file_dates` gives paths relative to the brain (`--relative`, NUL
//!   separated): for a brain in a subdirectory of a larger repo v2 got
//!   repo-relative paths that never matched a neuron.
//! - for such a nested brain, `log` and `status_short` are scoped to the
//!   brain directory (`-- .`); v2 showed the whole outer repo. A brain at the
//!   repo root runs v2's exact commands (`git log -- .` there would drop
//!   empty commits).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::yaml::is_py_space;

/// A git work tree that contains a brain directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// The brain directory the repo was found from (every call runs there).
    pub dir: PathBuf,
    /// The brain directory relative to the work tree root, `/`-separated with
    /// a trailing `/` (`git rev-parse --show-prefix`); empty at the root.
    pub prefix: String,
}

impl Repo {
    /// Whether the brain is a subdirectory of a larger repo.
    pub fn nested(&self) -> bool {
        !self.prefix.is_empty()
    }
}

/// One commit of [`Git::log`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The full hash (`%H`).
    pub hash: String,
    /// The subject (`%s`).
    pub subject: String,
    /// The author date, strict ISO 8601 (`%aI`).
    pub date: String,
}

/// The author dates of every path the history touched ([`Git::file_dates`]),
/// keyed by the `/`-separated path relative to the brain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDates {
    /// The newest commit that touched the path.
    pub latest: BTreeMap<String, String>,
    /// The oldest commit that touched the path.
    pub created: BTreeMap<String, String>,
}

/// A git command that could not run or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    /// The arguments after `git`.
    pub args: Vec<String>,
    /// The exit code; `None` when git could not be started or was killed.
    pub code: Option<i32>,
    /// git's stderr (trimmed), or why it could not be started.
    pub detail: String,
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let command = format!("git {}", self.args.join(" "));
        match self.code {
            Some(code) => write!(f, "`{command}` failed (exit {code}): {}", self.detail),
            None => write!(f, "`{command}` could not run: {}", self.detail),
        }
    }
}

impl std::error::Error for GitError {}

/// Every git operation Kluris runs.
pub trait Git {
    /// The work tree `dir` is in (`git rev-parse --is-inside-work-tree
    /// --show-prefix`); `None` outside one, inside a `.git` directory, or
    /// when git cannot run (v2's `is_git_repo`).
    fn repo(&self, dir: &Path) -> Option<Repo>;

    /// Whether `dir` is inside a git work tree.
    fn is_git_repo(&self, dir: &Path) -> bool {
        self.repo(dir).is_some()
    }

    /// `git init`, then `git checkout -b main` (v2's `git_init`; no
    /// identity is set).
    fn init(&self, dir: &Path) -> Result<(), GitError>;

    /// `git checkout -b <branch>`.
    fn checkout_new_branch(&self, dir: &Path, branch: &str) -> Result<(), GitError>;

    /// `git remote add <name> <url>`.
    fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<(), GitError>;

    /// `git add <pathspec>` (v2 always passes `-A`).
    fn add(&self, dir: &Path, pathspec: &str) -> Result<(), GitError>;

    /// `git commit -m <message>`, with v2's identity fallback: when git
    /// config (as seen from `dir`) lacks `user.name` or `user.email`, the
    /// author is `kluris <kluris@local>` and the committer the author, each
    /// variable only when the environment does not set it already.
    fn commit(&self, dir: &Path, message: &str) -> Result<(), GitError>;

    /// The newest `limit` commits, newest first; none in a fresh repo (no
    /// commit yet). Any other failure is an error. Scoped to the brain when it
    /// is nested.
    fn log(&self, repo: &Repo, limit: usize) -> Result<Vec<Commit>, GitError>;

    /// `git status --short`, stripped as Python's `str.strip()` strips it
    /// (so the first line's leading space goes, as in v2); empty when clean.
    /// Scoped to the brain, with brain-relative paths, when it is nested.
    fn status_short(&self, repo: &Repo) -> Result<String, GitError>;

    /// The latest and created author date of every path in the history of
    /// `HEAD`, from one `git log` (v2's `git_log_file_dates`); empty when
    /// the log fails (no commits, no git).
    fn file_dates(&self, repo: &Repo) -> FileDates;

    /// `git --version`, trimmed.
    fn version(&self) -> Result<String, GitError>;
}

/// [`Git`] through the `git` program found on `PATH`.
#[derive(Debug, Clone)]
pub struct SystemGit {
    program: OsString,
    /// Environment changes for every run, in order (`None` removes).
    env: Vec<(OsString, Option<OsString>)>,
}

impl Default for SystemGit {
    fn default() -> Self {
        SystemGit::new()
    }
}

/// v2's commit identity fallback, in `setdefault` order.
const IDENTITY_FALLBACK: [(&str, &str); 2] = [
    ("GIT_AUTHOR_NAME", "kluris"),
    ("GIT_AUTHOR_EMAIL", "kluris@local"),
];

impl SystemGit {
    pub fn new() -> SystemGit {
        SystemGit {
            program: OsString::from("git"),
            env: Vec::new(),
        }
    }

    /// Run `program` instead of `git`.
    pub fn program(mut self, program: impl Into<OsString>) -> SystemGit {
        self.program = program.into();
        self
    }

    /// Set an environment variable for every run.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> SystemGit {
        self.env.push((key.into(), Some(value.into())));
        self
    }

    /// Remove an environment variable for every run.
    pub fn env_remove(mut self, key: impl Into<OsString>) -> SystemGit {
        self.env.push((key.into(), None));
        self
    }

    /// A variable as the runs see it: the last change, else the process's.
    fn env_var(&self, key: &str) -> Option<OsString> {
        match self.env.iter().rev().find(|(k, _)| k == key) {
            Some((_, value)) => value.clone(),
            None => std::env::var_os(key),
        }
    }

    fn command(&self, dir: Option<&Path>, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(args);
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        for (key, value) in &self.env {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
        cmd
    }

    fn output(&self, mut cmd: Command, args: &[&str]) -> Result<Output, GitError> {
        let error = |code, detail| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code,
            detail,
        };
        let out = cmd.output().map_err(|e| error(None, e.to_string()))?;
        if out.status.success() {
            Ok(out)
        } else {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(error(out.status.code(), detail))
        }
    }

    /// Run git in `dir`; its stdout on success.
    fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
        self.output(self.command(Some(dir), args), args)
            .map(|out| out.stdout)
    }

    /// A config value as seen from `dir`; `None` when unset or empty
    /// (v2's `_read_git_config`).
    fn config_value(&self, dir: &Path, key: &str) -> Option<String> {
        let out = self.run(dir, &["config", "--get", key]).ok()?;
        let value = String::from_utf8_lossy(&out).trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    /// Whether the repo is a fresh one: `HEAD` names a branch (`git
    /// symbolic-ref`, which fails on a broken ref), no ref exists at all and
    /// the object database holds no commit. That is where git's own log says
    /// "your current branch '<b>' does not have any commits yet" with no
    /// history anywhere; an orphan branch beside other history, a broken ref
    /// or a missing object is not.
    fn is_fresh_repo(&self, dir: &Path) -> bool {
        if self.run(dir, &["symbolic-ref", "--quiet", "HEAD"]).is_err() {
            return false;
        }
        let refs = self.run(dir, &["for-each-ref", "--count=1", "--format=%(refname)"]);
        if !refs.is_ok_and(|out| out.is_empty()) {
            return false;
        }
        let objects = self.run(
            dir,
            &[
                "cat-file",
                "--batch-all-objects",
                "--batch-check=%(objecttype)",
            ],
        );
        objects.is_ok_and(|out| !out.split(|b| *b == b'\n').any(|t| t == b"commit"))
    }
}

impl Git for SystemGit {
    fn repo(&self, dir: &Path) -> Option<Repo> {
        let out = self
            .run(
                dir,
                &["rev-parse", "--is-inside-work-tree", "--show-prefix"],
            )
            .ok()?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        if lines.next()? != "true" {
            return None;
        }
        Some(Repo {
            dir: dir.to_path_buf(),
            prefix: lines.next().unwrap_or_default().to_string(),
        })
    }

    fn init(&self, dir: &Path) -> Result<(), GitError> {
        self.run(dir, &["init"])?;
        self.run(dir, &["checkout", "-b", "main"])?;
        Ok(())
    }

    fn checkout_new_branch(&self, dir: &Path, branch: &str) -> Result<(), GitError> {
        self.run(dir, &["checkout", "-b", branch]).map(drop)
    }

    fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<(), GitError> {
        self.run(dir, &["remote", "add", name, url]).map(drop)
    }

    fn add(&self, dir: &Path, pathspec: &str) -> Result<(), GitError> {
        self.run(dir, &["add", pathspec]).map(drop)
    }

    fn commit(&self, dir: &Path, message: &str) -> Result<(), GitError> {
        let args = ["commit", "-m", message];
        let mut cmd = self.command(Some(dir), &args);
        let configured = self.config_value(dir, "user.name").is_some()
            && self.config_value(dir, "user.email").is_some();
        if !configured {
            let mut set = |key: &str, default: OsString| {
                let value = self.env_var(key).unwrap_or(default);
                cmd.env(key, &value);
                value
            };
            let name = set(IDENTITY_FALLBACK[0].0, IDENTITY_FALLBACK[0].1.into());
            let email = set(IDENTITY_FALLBACK[1].0, IDENTITY_FALLBACK[1].1.into());
            set("GIT_COMMITTER_NAME", name);
            set("GIT_COMMITTER_EMAIL", email);
        }
        self.output(cmd, &args).map(drop)
    }

    fn log(&self, repo: &Repo, limit: usize) -> Result<Vec<Commit>, GitError> {
        let count = format!("-{limit}");
        let mut args = vec!["log", &count, "--format=%H%x00%s%x00%aI%x00"];
        if repo.nested() {
            args.extend(["--", "."]);
        }
        let out = match self.run(&repo.dir, &args) {
            Ok(out) => out,
            Err(_) if self.is_fresh_repo(&repo.dir) => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        Ok(parse_log(&String::from_utf8_lossy(&out)))
    }

    fn status_short(&self, repo: &Repo) -> Result<String, GitError> {
        let mut args = vec!["-c", "core.quotePath=false", "status", "--short"];
        if repo.nested() {
            args.splice(2..2, ["-c", "status.relativePaths=true"]);
            args.extend(["--", "."]);
        }
        let out = self.run(&repo.dir, &args)?;
        Ok(String::from_utf8_lossy(&out)
            .trim_matches(is_py_space)
            .to_string())
    }

    fn file_dates(&self, repo: &Repo) -> FileDates {
        let args = [
            "-c",
            "core.quotePath=false",
            "log",
            "-z",
            "--relative",
            "--format=%x00%aI",
            "--name-only",
            "HEAD",
        ];
        match self.run(&repo.dir, &args) {
            Ok(out) => parse_file_dates(&out),
            Err(_) => FileDates::default(),
        }
    }

    fn version(&self) -> Result<String, GitError> {
        let args = ["--version"];
        let out = self.output(self.command(None, &args), &args)?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

/// `%H\0%s\0%aI\0` records, each but the first after a `\n`.
fn parse_log(text: &str) -> Vec<Commit> {
    let mut fields = text.split('\0');
    let mut commits = Vec::new();
    while let (Some(hash), Some(subject), Some(date)) =
        (fields.next(), fields.next(), fields.next())
    {
        commits.push(Commit {
            hash: hash.trim_start_matches('\n').to_string(),
            subject: subject.to_string(),
            date: date.to_string(),
        });
    }
    commits
}

/// `git log -z --format=%x00%aI --name-only` output: per commit `\0<date>\0`,
/// then, when it touched a path in scope, `\n` and each path ending in `\0`.
/// A path is never empty, so an empty field always comes before a date.
fn parse_file_dates(out: &[u8]) -> FileDates {
    let mut dates = FileDates::default();
    let mut fields = out.split(|b| *b == 0);
    let mut date: Option<String> = None;
    let mut first_path = false;
    while let Some(field) = fields.next() {
        if field.is_empty() {
            date = fields
                .next()
                .map(|d| String::from_utf8_lossy(d).into_owned());
            first_path = true;
            continue;
        }
        let Some(date) = &date else { continue };
        let field = match field {
            [b'\n', rest @ ..] if first_path => rest,
            _ => field,
        };
        first_path = false;
        let path = String::from_utf8_lossy(field).into_owned();
        dates
            .latest
            .entry(path.clone())
            .or_insert_with(|| date.clone());
        dates.created.insert(path, date.clone());
    }
    dates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_records() {
        let text = "a\0one | two\x002026-01-01T12:00:00Z\0\nb\0\x002026-01-02T00:00:00+02:00\0\n";
        let commits = parse_log(text);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "one | two");
        assert_eq!(commits[1].hash, "b");
        assert_eq!(commits[1].subject, "");
        assert_eq!(commits[1].date, "2026-01-02T00:00:00+02:00");
        assert!(parse_log("").is_empty());
    }

    #[test]
    fn parse_file_dates_records() {
        let out = b"\0D3\0\0D2\0\n\nodd.md\0b.md\0\0D1\0\nb.md\0";
        let dates = parse_file_dates(out);
        assert_eq!(dates.latest["\nodd.md"], "D2", "only one \\n is git's");
        assert_eq!(dates.latest["b.md"], "D2");
        assert_eq!(dates.created["b.md"], "D1");
        assert_eq!(dates.latest.len(), 2);
    }
}
