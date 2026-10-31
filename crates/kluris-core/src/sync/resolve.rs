//! `kluris resolve` (SPEC Phase 7, item 48): settle an open sync conflict
//! ([`PENDING`]) path by path, then finish the merge.
//!
//! Each call is one transaction under the worktree's tree claim, from
//! reading the waiting merge to writing or removing it, so two resolvers,
//! an abort or a pull never change it in between. It records one choice in
//! the waiting merge: our side,
//! their side (a deletion included), or a file the person wrote, validated
//! first (markdown frontmatter must parse; a YAML neuron needs its `#---`
//! block; no conflict markers) and stored as a blob. Nothing else changes
//! until every conflicted path has a choice. Then, in the same call, the
//! engine finishes the merge ([`engine::finish_merge`]) with the message
//! `kluris: sync (resolved)`, `Kluris-Sync: resolved` and
//! `Kluris-Resolved-By`: anything that leaves the brain as it is keeps the
//! choices. Once it landed, the waiting merge goes, and a sync pushes it
//! (pulling again when the remote moved; a new conflict there is a new
//! waiting merge).
//!
//! Each call names the waiting merge it answers (its id, shown with every
//! conflict): an id that is not the current record's is refused, with the
//! current conflict, and nothing is recorded ([`pending::check_answer`]).
//! A waiting merge whose `ours` is no longer the branch's commit (the user
//! committed meanwhile) is stale: refused, with [`abort`] as the way out.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::claim::{ClaimError, TreeClaim};
use super::commit::{PathProblem, normalize_path};
use super::engine;
use super::git::{GitDirs, SyncGit};
use super::pending::{self, Found, Pending};
use super::{
    ConflictKind, ConflictText, LeftAsIs, LeftReason, ResolvedChoice, SyncError, SyncRequest,
};
use crate::frontmatter;
use crate::git::{GitError, Repo};

/// What to keep for a conflicted path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    /// The person's merged content.
    File(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct ResolveRequest<'a> {
    /// Brain-relative, as given.
    pub path: &'a str,
    /// The id of the waiting merge the choice answers.
    pub merge: &'a str,
    pub choice: Choice,
    /// For the merge commit (`no_verify`, the maps) and the push.
    pub sync: SyncRequest<'a>,
}

/// What one call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveReport {
    /// The path, normalized.
    pub path: String,
    /// `ours`, `theirs` or `file`.
    pub choice: &'static str,
    /// The conflicted paths still without a choice; after the commit, those
    /// of a new conflict the push met.
    pub remaining: Vec<String>,
    pub committed: bool,
    pub commit: Option<String>,
    pub pushed: bool,
    /// Why the sync after the commit did not push, when it failed.
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NoOpenConflict,
    /// The pending state does not read.
    Unreadable(PathBuf),
    InvalidPath {
        path: String,
        problem: PathProblem,
    },
    NotConflicted(String),
    /// `merge` is not the current waiting merge's id; nothing was recorded.
    WrongMerge {
        path: String,
        wrong: pending::WrongMerge,
        /// The current conflict at `path`, when the record has one.
        conflict: Option<Box<ConflictText>>,
    },
    InvalidFile {
        path: String,
        reason: String,
    },
    /// `--file` for a file/directory conflict: which side's structure to
    /// keep is a person's choice of `--ours` or `--theirs`.
    Structure {
        path: String,
    },
    /// A structural conflict whose sides' files git does not name.
    Unmappable {
        path: String,
    },
    /// `HEAD` is not the commit the conflict was found on.
    Stale {
        head: Option<String>,
        ours: String,
    },
    /// Uncommitted files the merge would overwrite; the choices are kept.
    Blocked(Vec<String>),
    /// The merge was left as it is (a hook, a lock, a git that failed);
    /// the choices are kept.
    Left(LeftAsIs),
    /// The waiting merge could not be written or removed.
    Record(String),
    Sync(SyncError),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::NoOpenConflict => f.write_str("there is no open sync conflict"),
            ResolveError::WrongMerge { path, wrong, .. } => write!(
                f,
                "{path}: merge {} is no longer the open conflict; this brain now waits on merge {}, so nothing was recorded: decide again from its sides, then give --merge {}",
                wrong.given, wrong.current, wrong.current
            ),
            ResolveError::Unreadable(path) => write!(
                f,
                "the open sync conflict cannot be read ({}); drop it with `kluris resolve --abort`",
                path.display()
            ),
            ResolveError::InvalidPath { path, problem } => write!(f, "{path} {problem}"),
            ResolveError::NotConflicted(path) => write!(
                f,
                "{path} is not conflicted; `kluris status` lists the open conflicts"
            ),
            ResolveError::InvalidFile { path, reason } => write!(
                f,
                "the file for {path} cannot be used: {reason}; nothing was recorded"
            ),
            ResolveError::Structure { path } => write!(
                f,
                "{path} is a file on one side and a directory on the other; keep one side's whole structure with --ours or --theirs (a --file says nothing about the directory); nothing was recorded"
            ),
            ResolveError::Unmappable { path } => write!(
                f,
                "{path}: git names no file on either side, so no choice can be mapped to files; nothing was recorded (drop the conflict with `kluris resolve --abort` and settle it with git)"
            ),
            ResolveError::Stale { head, ours } => write!(
                f,
                "the conflict was found on commit {}, but HEAD is now {}; drop it with `kluris resolve --abort`, then run `kluris sync` again",
                short(ours),
                head.as_deref().map_or("unborn".to_string(), short)
            ),
            ResolveError::Blocked(paths) => write!(
                f,
                "uncommitted changes are in the way: {}; commit or stash them, then run `kluris resolve` again for any conflicted path (the choices are kept)",
                paths.join(", ")
            ),
            ResolveError::Left(left) => left.fmt(f),
            ResolveError::Record(e) => write!(f, "cannot record the open conflict: {e}"),
            ResolveError::Sync(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ResolveError {}

impl From<SyncError> for ResolveError {
    fn from(e: SyncError) -> Self {
        ResolveError::Sync(e)
    }
}

impl From<GitError> for ResolveError {
    fn from(e: GitError) -> Self {
        ResolveError::Sync(SyncError::Git(e))
    }
}

impl From<LeftAsIs> for ResolveError {
    fn from(left: LeftAsIs) -> Self {
        match left.reason {
            LeftReason::Uncommitted => ResolveError::Blocked(left.paths),
            _ => ResolveError::Left(left),
        }
    }
}

/// Another command holds the work tree.
fn busy() -> LeftAsIs {
    LeftAsIs::because(
        LeftReason::Busy,
        "another kluris command is changing this brain; nothing was recorded: run again once it is done",
    )
}

fn record(e: std::io::Error) -> ResolveError {
    ResolveError::Record(e.to_string())
}

fn short(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// The brain's repo, refused as the engine refuses it, and its git
/// directories.
fn locate(root: &Path, git: &impl SyncGit) -> Result<(Repo, GitDirs), ResolveError> {
    let repo = git.repo(root).ok_or(SyncError::NotARepo)?;
    if repo.nested() {
        return Err(SyncError::Nested {
            prefix: repo.prefix.clone(),
        }
        .into());
    }
    let dirs = git.git_dirs(&repo)?;
    Ok((repo, dirs))
}

fn read_pending(path: &Path) -> Result<Pending, ResolveError> {
    match Pending::read(path) {
        Found::Waiting(pending) => Ok(pending),
        Found::None => Err(ResolveError::NoOpenConflict),
        Found::Unreadable => Err(ResolveError::Unreadable(path.into())),
    }
}

/// The waiting merge of the brain at `root`, for `kluris status`; `None`
/// when none is open (or it cannot be read).
pub fn waiting_merge(root: &Path, git: &impl SyncGit) -> Option<Pending> {
    let (_, dirs) = locate(root, git).ok()?;
    read_pending(&pending::path(&dirs)).ok()
}

/// Drop the open conflict, changing nothing else; how many paths it had.
/// Under the tree claim, waited for up to `bound`.
pub fn abort(
    root: &Path,
    git: &impl SyncGit,
    bound: std::time::Duration,
) -> Result<usize, ResolveError> {
    let (_, dirs) = locate(root, git)?;
    let _claim = TreeClaim::exclusive(&dirs.own, bound).map_err(|e| match e {
        ClaimError::Busy => ResolveError::Left(busy()),
        ClaimError::Io(e) => ResolveError::Record(format!("cannot claim the work tree: {e}")),
    })?;
    let path = pending::path(&dirs);
    let count = match read_pending(&path) {
        Ok(pending) => pending.conflicts.len(),
        Err(ResolveError::Unreadable(_)) => 0,
        Err(e) => return Err(e),
    };
    fs::remove_file(&path).map_err(record)?;
    Ok(count)
}

/// Why a person's file cannot be kept for `rel`.
fn validate(rel: &str, bytes: &[u8]) -> Result<(), String> {
    let lower = rel.to_ascii_lowercase();
    let markdown = lower.ends_with(".md");
    let yaml = lower.ends_with(".yml") || lower.ends_with(".yaml");
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) if markdown || yaml => return Err("the file is not valid UTF-8".into()),
        Err(_) => return Ok(()),
    };
    if text
        .lines()
        .any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> "))
    {
        return Err("it still has conflict markers".into());
    }
    if markdown {
        frontmatter::parse(text).map_err(|e| format!("invalid frontmatter: {e}"))?;
    } else if yaml {
        if !frontmatter::has_yaml_opt_in_head(bytes) {
            return Err("a YAML neuron must open with its #--- block".into());
        }
        frontmatter::parse_yaml_neuron(text).map_err(|e| format!("invalid frontmatter: {e}"))?;
    }
    Ok(())
}

/// Record a choice for one conflicted path; with the last one, finish the
/// merge (see the module docs).
pub fn resolve<G: SyncGit>(
    root: &Path,
    git: &G,
    request: &ResolveRequest,
) -> Result<ResolveReport, ResolveError> {
    // Every git from here has the remote environment: none prompts.
    let ctx = engine::open(root, git, &request.sync)?;
    let (git, repo, path) = (ctx.git(), ctx.repo(), ctx.pending());
    // Read only once the claim is held: what another resolver recorded is
    // in it.
    let claim = ctx.claim_tree().map_err(|_| ResolveError::Left(busy()))?;
    let mut pending = read_pending(&path)?;
    let rel = normalize_path(request.path).map_err(|problem| ResolveError::InvalidPath {
        path: request.path.to_string(),
        problem,
    })?;
    if let Err(wrong) = pending::check_answer(&pending, request.merge) {
        let conflict = engine::texts(git, repo, &pending)?
            .into_iter()
            .find(|c| c.path == rel)
            .map(Box::new);
        return Err(ResolveError::WrongMerge {
            path: rel,
            wrong,
            conflict,
        });
    }
    let Some(conflict) = pending.conflicts.iter().find(|c| c.path == rel) else {
        return Err(ResolveError::NotConflicted(rel));
    };
    // A structural conflict where git names no file on any side cannot be
    // mapped to a choice of files: refused, nothing recorded.
    if conflict.kind == ConflictKind::Structure
        && conflict.base.is_none()
        && conflict.ours.is_none()
        && conflict.theirs.is_none()
    {
        return Err(ResolveError::Unmappable { path: rel });
    }
    // A file for a file/directory conflict would say nothing about the
    // directory: only a whole side settles it.
    if conflict.kind == ConflictKind::Structure && matches!(request.choice, Choice::File(_)) {
        let below = format!("{rel}/");
        for side in [&pending.ours, &pending.theirs] {
            if git
                .tree_entries(repo, side)?
                .iter()
                .any(|e| e.rel.starts_with(&below))
            {
                return Err(ResolveError::Structure { path: rel });
            }
        }
    }
    // The pinned branch's commit, never HEAD's.
    let head = match ctx.branch() {
        Some(branch) => git.ref_value(repo, branch)?,
        None => None,
    };
    if head.as_deref() != Some(pending.ours.as_str()) {
        return Err(ResolveError::Stale {
            head,
            ours: pending.ours.clone(),
        });
    }
    if let Some(op) = git.in_progress(repo)? {
        return Err(ResolveError::Left(LeftAsIs::because(
            LeftReason::InProgress,
            format!("a {op} is in progress; finish or abort it first"),
        )));
    }
    let choice = match &request.choice {
        Choice::Ours => ResolvedChoice::Ours,
        Choice::Theirs => ResolvedChoice::Theirs,
        Choice::File(bytes) => {
            validate(&rel, bytes).map_err(|reason| ResolveError::InvalidFile {
                path: rel.clone(),
                reason,
            })?;
            ResolvedChoice::Blob(git.hash_blob(repo, bytes)?)
        }
    };
    let label = choice.as_str();
    pending.resolved.retain(|(p, _)| *p != rel);
    pending.resolved.push((rel.clone(), choice));
    let remaining: Vec<String> = pending
        .conflicts
        .iter()
        .filter(|c| !pending.resolved.iter().any(|(p, _)| *p == c.path))
        .map(|c| c.path.clone())
        .collect();
    let mut report = ResolveReport {
        path: rel,
        choice: label,
        remaining,
        committed: false,
        commit: None,
        pushed: false,
        sync_error: None,
    };
    if !report.remaining.is_empty() {
        pending.write(&path).map_err(record)?;
        return Ok(report);
    }

    // Every path has a choice: finish the merge. Until the merge commit is
    // in place the waiting merge (with the choices) stays.
    let commit = finish(&ctx, &claim, &pending);
    let commit = match commit {
        Ok(commit) => commit,
        Err(e) => {
            pending.write(&path).map_err(record)?;
            return Err(e);
        }
    };
    pending::remove(&path).map_err(record)?;
    drop(claim);
    report.committed = true;
    report.commit = Some(commit);
    let synced = engine::sync(&ctx);
    report.pushed = synced.pushed;
    report.remaining = synced.conflicts.into_iter().map(|c| c.path).collect();
    // Why it was not pushed, unless the network was missing.
    report.sync_error = synced
        .left_as_is
        .filter(|left| left.reason != LeftReason::Offline)
        .map(|left| left.to_string());
    Ok(report)
}

/// The merge with the choices, committed and landed; its commit.
fn finish<G: SyncGit>(
    ctx: &engine::Context<G>,
    claim: &TreeClaim,
    pending: &Pending,
) -> Result<String, ResolveError> {
    let choices: BTreeMap<String, ResolvedChoice> = pending.resolved.iter().cloned().collect();
    let (git, repo) = (ctx.git(), ctx.repo());
    let author = git.author(repo)?;
    let resolver = git
        .config(repo, "user.name")
        .unwrap_or_else(|| author.name.clone());
    let message = format!(
        "kluris: sync (resolved)\n\nKluris-Sync: resolved\nKluris-Resolved-By: {}\n",
        resolver.replace(['\r', '\n'], " ")
    );
    Ok(engine::finish_merge(
        ctx, claim, pending, &choices, &message,
    )?)
}
