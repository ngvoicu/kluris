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

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    pub(crate) fn env_var(&self, key: &str) -> Option<OsString> {
        match self.env.iter().rev().find(|(k, _)| k == key) {
            Some((_, value)) => value.clone(),
            None => std::env::var_os(key),
        }
    }

    /// git with `args`, in `dir` when given, with this runner's environment.
    pub(crate) fn command(&self, dir: Option<&Path>, args: &[&str]) -> Command {
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

    pub(crate) fn output(&self, mut cmd: Command, args: &[&str]) -> Result<Output, GitError> {
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
    pub(crate) fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
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

    /// v2's identity fallback on `cmd`: when git config (as seen from `dir`)
    /// lacks `user.name` or `user.email`, the author is `kluris
    /// <kluris@local>` and the committer the author, each variable only when
    /// the environment does not set it already.
    fn identity_fallback(&self, dir: &Path, cmd: &mut Command) {
        let configured = self.config_value(dir, "user.name").is_some()
            && self.config_value(dir, "user.email").is_some();
        if configured {
            return;
        }
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

    /// Run git in `dir` with `GIT_INDEX_FILE` set; its stdout on success.
    fn run_with_index(&self, dir: &Path, index: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let mut cmd = self.command(Some(dir), args);
        cmd.env("GIT_INDEX_FILE", index);
        self.output(cmd, args).map(|out| out.stdout)
    }

    /// Run git in `dir` (with `GIT_INDEX_FILE` when `index` is given),
    /// feeding `input` on stdin; its stdout on success.
    pub(crate) fn run_input(
        &self,
        dir: &Path,
        index: Option<&Path>,
        args: &[&str],
        input: Vec<u8>,
    ) -> Result<Vec<u8>, GitError> {
        let error = |code, detail| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code,
            detail,
        };
        let mut cmd = self.command(Some(dir), args);
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| error(None, e.to_string()))?;
        // Written from a thread: git may fill stdout before it has read all
        // of stdin.
        let writer = child.stdin.take().map(|mut stdin| {
            std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            })
        });
        let out = child
            .wait_with_output()
            .map_err(|e| error(None, e.to_string()))?;
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        if out.status.success() {
            Ok(out.stdout)
        } else {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(error(out.status.code(), detail))
        }
    }

    /// A path `git rev-parse --git-path` names, made absolute.
    fn git_path(&self, dir: &Path, name: &str) -> Result<PathBuf, GitError> {
        let out = self.run(dir, &["rev-parse", "--git-path", name])?;
        let path = PathBuf::from(String::from_utf8_lossy(&out).trim_end_matches(['\n', '\r']));
        Ok(if path.is_absolute() {
            path
        } else {
            dir.join(path)
        })
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
        self.identity_fallback(dir, &mut cmd);
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

/// A commit author: `git var GIT_AUTHOR_IDENT` without its date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

/// A git operation left half done in the work tree, which a commit made now
/// would finish or corrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl fmt::Display for InProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InProgress::Merge => "merge",
            InProgress::Rebase => "rebase",
            InProgress::CherryPick => "cherry-pick",
            InProgress::Revert => "revert",
        })
    }
}

/// The marker each [`InProgress`] leaves in the git directory.
const IN_PROGRESS_MARKERS: [(&str, InProgress); 5] = [
    ("MERGE_HEAD", InProgress::Merge),
    ("rebase-merge", InProgress::Rebase),
    ("rebase-apply", InProgress::Rebase),
    ("CHERRY_PICK_HEAD", InProgress::CherryPick),
    ("REVERT_HEAD", InProgress::Revert),
];

/// The git plumbing `commit` (and next `sync`) builds commits with, without
/// touching the user's index until a commit is made. Every path is relative
/// to the brain directory and `/`-separated, and is taken literally (no
/// pathspec magic: `[draft].md` is that file, not a glob).
pub trait Plumbing: Git {
    /// The commit `HEAD` names; `None` on a branch with no commit yet.
    fn head(&self, repo: &Repo) -> Result<Option<String>, GitError>;

    /// The merge, rebase, cherry-pick or revert the work tree is in the
    /// middle of, found by the markers in its git directory (a worktree's
    /// or a nested repo's own, wherever `.git` points).
    fn in_progress(&self, repo: &Repo) -> Result<Option<InProgress>, GitError>;

    /// Which of `paths` are files in `rev` (a commit or a tree).
    fn tracked(
        &self,
        repo: &Repo,
        rev: &str,
        paths: &[String],
    ) -> Result<BTreeSet<String>, GitError>;

    /// Every file of `rev` (a commit or a tree) under the brain directory,
    /// brain-relative; submodules are left out.
    fn tree_entries(&self, repo: &Repo, rev: &str) -> Result<Vec<TreeEntry>, GitError>;

    /// The contents of blobs, by their ids (one `git cat-file --batch`).
    fn read_blobs(
        &self,
        repo: &Repo,
        oids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, GitError>;

    /// A path in the git directory (`git rev-parse --git-path`), absolute.
    fn git_dir_path(&self, repo: &Repo, name: &str) -> Result<PathBuf, GitError>;

    /// A temporary index holding the tree of `base` (none: empty). The
    /// user's index is never touched through it.
    fn staging_index(&self, repo: &Repo, base: Option<&str>) -> Result<StagingIndex, GitError>;

    /// Take `paths` from the work tree into `index` as `git add -A` takes
    /// them (a missing file is removed).
    fn stage_worktree(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        paths: &[String],
    ) -> Result<(), GitError>;

    /// Put contents that are not in the work tree into `index` (a `None`
    /// removes the path).
    fn stage_blobs(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        blobs: &[StagedBlob],
    ) -> Result<(), GitError>;

    /// Put blobs already in the object database into `index`, by
    /// `(path, mode, blob id)`.
    fn stage_entries(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        entries: &[(String, String, String)],
    ) -> Result<(), GitError>;

    /// `git write-tree` of `index`.
    fn write_tree(&self, repo: &Repo, index: &StagingIndex) -> Result<String, GitError>;

    /// Run a hook (`git hook run --ignore-missing`, so `core.hooksPath` is
    /// honoured), against `index` when given; its stdout and stderr go to
    /// this process's stderr. Returns the hook's exit code (0 when there is
    /// no such hook).
    fn run_hook(
        &self,
        repo: &Repo,
        index: Option<&StagingIndex>,
        name: &str,
        args: &[&str],
    ) -> Result<i32, GitError>;

    /// The tree of a commit.
    fn tree_of(&self, repo: &Repo, rev: &str) -> Result<String, GitError>;

    /// Which of `paths` differ between two trees.
    fn changed(
        &self,
        repo: &Repo,
        from: &str,
        to: &str,
        paths: &[String],
    ) -> Result<BTreeSet<String>, GitError>;

    /// The identity a commit made now is authored by, with v2's fallback
    /// (`kluris <kluris@local>`) when git config has none.
    fn author(&self, repo: &Repo) -> Result<Identity, GitError>;

    /// A git config value as seen from the brain; `None` when unset or
    /// empty.
    fn config(&self, repo: &Repo, key: &str) -> Option<String>;

    /// A commit of `tree` on `parents` (`git commit-tree -F`, the message
    /// read from `message_file` as it is), authored as [`Plumbing::author`]
    /// says.
    fn commit_tree(
        &self,
        repo: &Repo,
        tree: &str,
        parents: &[&str],
        message_file: &Path,
    ) -> Result<String, GitError>;

    /// Move `HEAD` (the branch it names) to `new` only if it is still at
    /// `old` (`None`: only if it has no commit yet), recording `reason` in
    /// the reflog.
    fn update_head(
        &self,
        repo: &Repo,
        new: &str,
        old: Option<&str>,
        reason: &str,
    ) -> Result<(), GitError>;

    /// Set the user's index entries of `paths` to `rev`'s (`git reset -q
    /// <rev> -- <paths>`); every other entry stays as it is.
    fn reset_paths(&self, repo: &Repo, rev: &str, paths: &[String]) -> Result<(), GitError>;

    /// Every path in the brain with a change git would commit or an
    /// untracked file it would add (`git status`, ignored files left out),
    /// sorted.
    fn dirty(&self, repo: &Repo) -> Result<Vec<String>, GitError>;
}

/// The oldest git Kluris's sync commands run with: `merge-tree
/// --write-tree` (2.38) and `git hook run` (2.36).
pub const MIN_SYNC_VERSION: (u32, u32, u32) = (2, 38, 0);

/// `git --version` as `(major, minor, patch)`: `git version 2.47.1.windows.1`
/// is `(2, 47, 1)`, `git version 2.38` is `(2, 38, 0)`.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let number = text.trim().strip_prefix("git version ")?;
    let number = number.split_whitespace().next()?;
    let mut parts = number.split('.').map_while(|p| p.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    Some((major, minor, parts.next().unwrap_or(0)))
}

/// A file of a git tree ([`Plumbing::tree_entries`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Brain-relative, `/`-separated.
    pub rel: String,
    /// `100644`, `100755` or `120000` (a symlink).
    pub mode: String,
    pub oid: String,
}

impl TreeEntry {
    pub fn is_symlink(&self) -> bool {
        self.mode == "120000"
    }
}

/// Contents for [`Plumbing::stage_blobs`]: `None` removes the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedBlob {
    /// Brain-relative, `/`-separated.
    pub rel: String,
    pub mode: String,
    pub bytes: Option<Vec<u8>>,
}

/// A temporary index file in the git directory ([`Plumbing::staging_index`]),
/// removed (with a lock git may have left) when dropped.
#[derive(Debug)]
pub struct StagingIndex(PathBuf);

impl StagingIndex {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// A file name unique to this process and moment.
fn unique_name(stem: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{stem}-{}-{nanos}", std::process::id())
}

impl Drop for StagingIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

/// `git <args> -- <paths>` with every path taken literally.
fn with_paths<'a>(args: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut all = vec!["--literal-pathspecs"];
    all.extend_from_slice(args);
    all.push("--");
    all.extend(paths.iter().map(String::as_str));
    all
}

/// NUL-separated names, the empty ones dropped.
fn nul_names(out: &[u8]) -> impl Iterator<Item = String> + '_ {
    out.split(|b| *b == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
}

/// One hash on a line of its own.
fn hash_line(out: &[u8]) -> String {
    String::from_utf8_lossy(out).trim().to_string()
}

/// `git ls-tree -r -z` records: `<mode> <type> <oid>\t<path>`; blobs only.
fn parse_ls_tree(out: &[u8]) -> Vec<TreeEntry> {
    out.split(|b| *b == 0)
        .filter_map(|record| {
            let tab = record.iter().position(|b| *b == b'\t')?;
            let head = String::from_utf8_lossy(&record[..tab]);
            let mut fields = head.split(' ');
            let (mode, kind, oid) = (fields.next()?, fields.next()?, fields.next()?);
            (kind == "blob").then(|| TreeEntry {
                rel: String::from_utf8_lossy(&record[tab + 1..]).into_owned(),
                mode: mode.to_string(),
                oid: oid.to_string(),
            })
        })
        .collect()
}

/// `git cat-file --batch` output: per object `<oid> <type> <size>\n`, the
/// contents, `\n`; a `<oid> missing` line for one that is not there.
fn parse_batch(out: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut blobs = BTreeMap::new();
    let mut rest = out;
    while let Some(end) = rest.iter().position(|b| *b == b'\n') {
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        let fields: Vec<&str> = header.split(' ').collect();
        let [oid, _kind, size] = fields[..] else {
            continue;
        };
        let Ok(size) = size.parse::<usize>() else {
            break;
        };
        let Some(contents) = rest.get(..size) else {
            break;
        };
        blobs.insert(oid.to_string(), contents.to_vec());
        rest = rest.get(size + 1..).unwrap_or_default();
    }
    blobs
}

/// `Name <email> <time> <zone>` from `git var GIT_AUTHOR_IDENT`.
fn parse_ident(text: &str) -> Option<Identity> {
    let open = text.rfind('<')?;
    let close = open + text[open..].find('>')?;
    Some(Identity {
        name: text[..open].trim().to_string(),
        email: text[open + 1..close].to_string(),
    })
}

/// `git status --porcelain -z` records: `XY path`, and for a rename or copy
/// `XY to` then `from`. Paths are relative to the work tree root.
fn parse_porcelain(out: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut fields = out.split(|b| *b == 0);
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let (x, y) = (field[0], field[1]);
        paths.push(String::from_utf8_lossy(&field[3..]).into_owned());
        if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            fields.next();
        }
    }
    paths
}

impl Plumbing for SystemGit {
    fn head(&self, repo: &Repo) -> Result<Option<String>, GitError> {
        match self.run(&repo.dir, &["rev-parse", "-q", "--verify", "HEAD^{commit}"]) {
            Ok(out) => Ok(Some(hash_line(&out))),
            Err(err) => match self.run(&repo.dir, &["symbolic-ref", "-q", "HEAD"]) {
                // A branch with no commit yet: `HEAD` names a ref that is
                // not there.
                Ok(_)
                    if self
                        .run(&repo.dir, &["rev-parse", "-q", "--verify", "HEAD"])
                        .is_err() =>
                {
                    Ok(None)
                }
                _ => Err(err),
            },
        }
    }

    fn in_progress(&self, repo: &Repo) -> Result<Option<InProgress>, GitError> {
        for (marker, what) in IN_PROGRESS_MARKERS {
            if self.git_path(&repo.dir, marker)?.exists() {
                return Ok(Some(what));
            }
        }
        Ok(None)
    }

    fn tracked(
        &self,
        repo: &Repo,
        rev: &str,
        paths: &[String],
    ) -> Result<BTreeSet<String>, GitError> {
        if paths.is_empty() {
            return Ok(BTreeSet::new());
        }
        let args = with_paths(&["ls-tree", "-r", "-z", "--name-only", rev], paths);
        let out = self.run(&repo.dir, &args)?;
        Ok(nul_names(&out).collect())
    }

    fn tree_entries(&self, repo: &Repo, rev: &str) -> Result<Vec<TreeEntry>, GitError> {
        let out = self.run(&repo.dir, &["ls-tree", "-r", "-z", rev])?;
        Ok(parse_ls_tree(&out))
    }

    fn read_blobs(
        &self,
        repo: &Repo,
        oids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, GitError> {
        if oids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut input = oids.join("\n").into_bytes();
        input.push(b'\n');
        let out = self.run_input(&repo.dir, None, &["cat-file", "--batch"], input)?;
        Ok(parse_batch(&out))
    }

    fn git_dir_path(&self, repo: &Repo, name: &str) -> Result<PathBuf, GitError> {
        self.git_path(&repo.dir, name)
    }

    fn staging_index(&self, repo: &Repo, base: Option<&str>) -> Result<StagingIndex, GitError> {
        let name = format!("{}.index", unique_name("kluris-commit"));
        let index = StagingIndex(self.git_path(&repo.dir, &name)?);
        if let Some(base) = base {
            self.run_with_index(&repo.dir, &index.0, &["read-tree", base])?;
        }
        Ok(index)
    }

    fn stage_worktree(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        paths: &[String],
    ) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let args = with_paths(&["add", "-A"], paths);
        self.run_with_index(&repo.dir, &index.0, &args).map(drop)
    }

    fn stage_blobs(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        blobs: &[StagedBlob],
    ) -> Result<(), GitError> {
        let mut info = Vec::new();
        let mut removed = Vec::new();
        for blob in blobs {
            match &blob.bytes {
                Some(bytes) => {
                    let out = self.run_input(
                        &repo.dir,
                        None,
                        &["hash-object", "-w", "--stdin"],
                        bytes.clone(),
                    )?;
                    // `--index-info` paths are relative to the work tree root.
                    let record = format!(
                        "{} {}\t{}{}\0",
                        blob.mode,
                        hash_line(&out),
                        repo.prefix,
                        blob.rel
                    );
                    info.extend_from_slice(record.as_bytes());
                }
                None => removed.push(blob.rel.clone()),
            }
        }
        if !info.is_empty() {
            self.run_input(
                &repo.dir,
                Some(&index.0),
                &["update-index", "-z", "--index-info"],
                info,
            )?;
        }
        if !removed.is_empty() {
            let mut args = vec!["update-index", "--force-remove", "--"];
            args.extend(removed.iter().map(String::as_str));
            self.run_with_index(&repo.dir, &index.0, &args)?;
        }
        Ok(())
    }

    fn stage_entries(
        &self,
        repo: &Repo,
        index: &StagingIndex,
        entries: &[(String, String, String)],
    ) -> Result<(), GitError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut info = Vec::new();
        for (rel, mode, oid) in entries {
            // `--index-info` paths are relative to the work tree root.
            info.extend_from_slice(format!("{mode} {oid}\t{}{rel}\0", repo.prefix).as_bytes());
        }
        self.run_input(
            &repo.dir,
            Some(&index.0),
            &["update-index", "-z", "--index-info"],
            info,
        )
        .map(drop)
    }

    fn write_tree(&self, repo: &Repo, index: &StagingIndex) -> Result<String, GitError> {
        let out = self.run_with_index(&repo.dir, &index.0, &["write-tree"])?;
        Ok(hash_line(&out))
    }

    fn run_hook(
        &self,
        repo: &Repo,
        index: Option<&StagingIndex>,
        name: &str,
        args: &[&str],
    ) -> Result<i32, GitError> {
        let mut all = vec!["hook", "run", "--ignore-missing", name];
        if !args.is_empty() {
            all.push("--");
            all.extend_from_slice(args);
        }
        let error = |code, detail: String| GitError {
            args: all.iter().map(|a| a.to_string()).collect(),
            code,
            detail,
        };
        let mut cmd = self.command(Some(&repo.dir), &all);
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", &index.0);
        }
        // As `git commit` runs them: no editor, the hook's output on stderr.
        cmd.env("GIT_EDITOR", ":")
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::io::stderr()))
            .stderr(Stdio::inherit());
        let status = cmd.status().map_err(|e| error(None, e.to_string()))?;
        status
            .code()
            .ok_or_else(|| error(None, "the hook was killed by a signal".into()))
    }

    fn tree_of(&self, repo: &Repo, rev: &str) -> Result<String, GitError> {
        let spec = format!("{rev}^{{tree}}");
        let out = self.run(&repo.dir, &["rev-parse", "--verify", &spec])?;
        Ok(hash_line(&out))
    }

    fn changed(
        &self,
        repo: &Repo,
        from: &str,
        to: &str,
        paths: &[String],
    ) -> Result<BTreeSet<String>, GitError> {
        if paths.is_empty() {
            return Ok(BTreeSet::new());
        }
        let args = with_paths(
            &[
                "diff-tree",
                "-r",
                "-z",
                "--name-only",
                "--relative",
                from,
                to,
            ],
            paths,
        );
        let out = self.run(&repo.dir, &args)?;
        Ok(nul_names(&out).collect())
    }

    fn author(&self, repo: &Repo) -> Result<Identity, GitError> {
        let args = ["var", "GIT_AUTHOR_IDENT"];
        let mut cmd = self.command(Some(&repo.dir), &args);
        self.identity_fallback(&repo.dir, &mut cmd);
        let out = self.output(cmd, &args)?;
        let text = String::from_utf8_lossy(&out.stdout);
        parse_ident(text.trim()).ok_or_else(|| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code: Some(0),
            detail: format!("unexpected identity {:?}", text.trim()),
        })
    }

    fn config(&self, repo: &Repo, key: &str) -> Option<String> {
        self.config_value(&repo.dir, key)
    }

    fn commit_tree(
        &self,
        repo: &Repo,
        tree: &str,
        parents: &[&str],
        message_file: &Path,
    ) -> Result<String, GitError> {
        let file = message_file.to_string_lossy();
        let mut args = vec!["commit-tree", tree];
        for parent in parents {
            args.extend(["-p", parent]);
        }
        args.extend(["-F", &file]);
        let mut cmd = self.command(Some(&repo.dir), &args);
        self.identity_fallback(&repo.dir, &mut cmd);
        let out = self.output(cmd, &args)?;
        Ok(hash_line(&out.stdout))
    }

    fn update_head(
        &self,
        repo: &Repo,
        new: &str,
        old: Option<&str>,
        reason: &str,
    ) -> Result<(), GitError> {
        let args = ["update-ref", "-m", reason, "HEAD", new, old.unwrap_or("")];
        self.run(&repo.dir, &args).map(drop)
    }

    fn reset_paths(&self, repo: &Repo, rev: &str, paths: &[String]) -> Result<(), GitError> {
        if paths.is_empty() {
            return Ok(());
        }
        let args = with_paths(&["reset", "-q", rev], paths);
        self.run(&repo.dir, &args).map(drop)
    }

    fn dirty(&self, repo: &Repo) -> Result<Vec<String>, GitError> {
        let args = [
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ];
        let out = self.run(&repo.dir, &args)?;
        let mut paths: Vec<String> = parse_porcelain(&out)
            .into_iter()
            .filter_map(|path| path.strip_prefix(&repo.prefix).map(str::to_string))
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
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
    fn parse_git_versions() {
        assert_eq!(parse_version("git version 2.55.0"), Some((2, 55, 0)));
        assert_eq!(
            parse_version("git version 2.47.1.windows.1"),
            Some((2, 47, 1))
        );
        assert_eq!(parse_version("git version 2.38"), Some((2, 38, 0)));
        assert_eq!(
            parse_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39, 5))
        );
        assert_eq!(parse_version("not git"), None);
        assert!(MIN_SYNC_VERSION <= (2, 38, 0) && (2, 37, 9) < MIN_SYNC_VERSION);
    }

    #[test]
    fn parse_ident_and_porcelain() {
        assert_eq!(
            parse_ident("Ana Tester <ana@example.com> 1793037949 +0200"),
            Some(Identity {
                name: "Ana Tester".into(),
                email: "ana@example.com".into()
            })
        );
        assert_eq!(parse_ident("no email"), None);
        let out = b" M a.md\0?? sub/new \xc3\xa9.md\0R  to.md\0from.md\0D  gone.md\0";
        assert_eq!(
            parse_porcelain(out),
            ["a.md", "sub/new é.md", "to.md", "gone.md"]
        );
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
