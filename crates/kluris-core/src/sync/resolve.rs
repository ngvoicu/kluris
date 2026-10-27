//! `kluris resolve` (SPEC Phase 7, item 48): settle an open sync conflict
//! ([`PENDING`]) path by path, then finish the merge.
//!
//! Each call records one choice in the pending state, atomically: our side,
//! their side (a deletion included), or a file the person wrote, validated
//! first (markdown frontmatter must parse; a YAML neuron needs its `#---`
//! block; no conflict markers) and stored as a blob. Nothing else changes
//! until every conflicted path has a choice. Then, in the same call:
//! 1. the merge is built again from the recorded `ours` and `theirs`, with
//!    the choices in place of the rules and the generated maps rebuilt;
//! 2. uncommitted files in the way stop it, as in sync (the choices stay);
//! 3. the merge commit (`pre-merge-commit` unless `no_verify`, two parents,
//!    `kluris: sync (resolved)` with `Kluris-Sync: resolved` and
//!    `Kluris-Resolved-By`) is fast-forwarded to;
//! 4. the pending state goes, and a sync pushes it (retrying when the
//!    remote moved; a new conflict there is a new pending state).
//!
//! A pending state whose `ours` is no longer `HEAD` (the user committed
//! meanwhile) is stale: refused, with [`abort`] as the way out.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::commit::{PathProblem, normalize_path};
use super::merge::{MergeOutcome, merge_commits};
use super::plumbing::SyncGit;
use super::{
    Conflict, ConflictKind, PENDING, Pending, ResolvedChoice, SyncError, SyncRequest,
    blocked_paths, check_git, commit_merge, sync,
};
use crate::frontmatter;
use crate::git::{GitError, Plumbing, Repo};

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
    InvalidFile {
        path: String,
        reason: String,
    },
    /// `--file` for a file/directory conflict: which side's structure to
    /// keep is a person's choice of `--ours` or `--theirs`.
    Structure {
        path: String,
    },
    /// `HEAD` is not the commit the conflict was found on.
    Stale {
        head: Option<String>,
        ours: String,
    },
    /// Uncommitted files the merge would overwrite; the choices are kept.
    Blocked(Vec<String>),
    Sync(SyncError),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::NoOpenConflict => f.write_str("there is no open sync conflict"),
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

fn short(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// The brain's repo, refused as sync refuses it, and where its pending
/// state lives.
fn locate(root: &Path, git: &impl Plumbing) -> Result<(Repo, PathBuf), ResolveError> {
    let repo = git.repo(root).ok_or(SyncError::NotARepo)?;
    if repo.nested() {
        return Err(SyncError::Nested {
            prefix: repo.prefix.clone(),
        }
        .into());
    }
    let pending = git.git_dir_path(&repo, PENDING)?;
    Ok((repo, pending))
}

fn read_pending(path: &Path) -> Result<Pending, ResolveError> {
    match fs::read_to_string(path) {
        Ok(text) => Pending::parse(&text).ok_or_else(|| ResolveError::Unreadable(path.into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ResolveError::NoOpenConflict),
        Err(_) => Err(ResolveError::Unreadable(path.into())),
    }
}

/// The open conflicts of the brain at `root`, for `kluris status`; `None`
/// when none is open (or it cannot be read).
pub fn open_conflicts(root: &Path, git: &impl Plumbing) -> Option<Vec<Conflict>> {
    let (_, path) = locate(root, git).ok()?;
    read_pending(&path).ok().map(|p| p.conflicts)
}

/// Drop the open conflict, changing nothing else; how many paths it had.
pub fn abort(root: &Path, git: &impl Plumbing) -> Result<usize, ResolveError> {
    let (_, path) = locate(root, git)?;
    let count = match read_pending(&path) {
        Ok(pending) => pending.conflicts.len(),
        Err(ResolveError::Unreadable(_)) => 0,
        Err(e) => return Err(e),
    };
    fs::remove_file(&path).map_err(|e| SyncError::Pending(e.to_string()))?;
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
    let (repo, path) = locate(root, git)?;
    let mut pending = read_pending(&path)?;
    let rel = normalize_path(request.path).map_err(|problem| ResolveError::InvalidPath {
        path: request.path.to_string(),
        problem,
    })?;
    let Some(conflict) = pending.conflicts.iter().find(|c| c.path == rel) else {
        return Err(ResolveError::NotConflicted(rel));
    };
    // A file for a file/directory conflict would say nothing about the
    // directory: only a whole side settles it.
    if conflict.kind == ConflictKind::Structure && matches!(request.choice, Choice::File(_)) {
        let below = format!("{rel}/");
        for side in [&pending.ours, &pending.theirs] {
            if git
                .tree_entries(&repo, side)?
                .iter()
                .any(|e| e.rel.starts_with(&below))
            {
                return Err(ResolveError::Structure { path: rel });
            }
        }
    }
    let head = git.head(&repo)?;
    if head.as_deref() != Some(pending.ours.as_str()) {
        return Err(ResolveError::Stale {
            head,
            ours: pending.ours.clone(),
        });
    }
    if let Some(op) = git.in_progress(&repo)? {
        return Err(SyncError::InProgress(op).into());
    }
    let choice = match &request.choice {
        Choice::Ours => ResolvedChoice::Ours,
        Choice::Theirs => ResolvedChoice::Theirs,
        Choice::File(bytes) => {
            validate(&rel, bytes).map_err(|reason| ResolveError::InvalidFile {
                path: rel.clone(),
                reason,
            })?;
            ResolvedChoice::Blob(git.hash_blob(&repo, bytes)?)
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
        pending.write(&path)?;
        return Ok(report);
    }

    // Every path has a choice: finish the merge. Until the merge commit is
    // in place the pending state (with the choices) stays.
    check_git(git)?;
    let commit = finish(root, git, &repo, &pending, request);
    if let Err(e) = &commit {
        pending.write(&path)?;
        return Err(e.clone());
    }
    let commit = commit?;
    fs::remove_file(&path).map_err(|e| SyncError::Pending(e.to_string()))?;
    report.committed = true;
    report.commit = Some(commit);
    match sync(root, git, &request.sync) {
        Ok(synced) => {
            report.pushed = synced.pushed;
            report.remaining = synced.conflicts.into_iter().map(|c| c.path).collect();
        }
        Err(e) => report.sync_error = Some(e.to_string()),
    }
    Ok(report)
}

/// The merge with the choices, committed and moved to; its commit.
fn finish<G: SyncGit>(
    root: &Path,
    git: &G,
    repo: &Repo,
    pending: &Pending,
    request: &ResolveRequest,
) -> Result<String, ResolveError> {
    let choices: BTreeMap<String, ResolvedChoice> = pending.resolved.iter().cloned().collect();
    let outcome = merge_commits(
        git,
        repo,
        &pending.ours,
        &pending.theirs,
        &request.sync.maps,
        &choices,
    )?;
    let MergeOutcome::Clean { tree, index, .. } = outcome else {
        return Err(SyncError::Git(GitError {
            args: vec!["merge-tree".into()],
            code: None,
            detail: "the merge has conflicts the recorded choices do not cover".into(),
        })
        .into());
    };
    let dirty = git.dirty(repo)?;
    let blocked = blocked_paths(git, repo, root, &dirty, &pending.ours, &tree)?;
    if !blocked.is_empty() {
        return Err(ResolveError::Blocked(blocked));
    }
    let author = git.author(repo)?;
    let resolver = git
        .config(repo, "user.name")
        .unwrap_or_else(|| author.name.clone());
    let message = format!(
        "kluris: sync (resolved)\n\nKluris-Sync: resolved\nKluris-Resolved-By: {}\n",
        resolver.replace(['\r', '\n'], " ")
    );
    let commit = commit_merge(
        git,
        repo,
        &index,
        [&pending.ours, &pending.theirs],
        &message,
        request.sync.no_verify,
    )?;
    let blocked = super::land(git, repo, root, &pending.ours, &commit)?;
    if !blocked.is_empty() {
        return Err(ResolveError::Blocked(blocked));
    }
    Ok(commit)
}
