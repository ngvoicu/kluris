//! `kluris resolve`: settle a waiting merge ([`PENDING`]) path by path, then
//! finish it, in the explicit session the command opened.
//!
//! The session's open pulls first, so the waiting merge resolved is the one
//! for the branch's commit now: one the branch moved on from was replaced
//! by that pull (or, offline, is no open conflict at all). Each call holds
//! the session's exclusive tree claim from reading the waiting merge to
//! writing or removing it, so two resolvers or a pull never change it in
//! between. It records one choice: our side, their side (a deletion
//! included), or a file the person wrote, validated first (markdown
//! frontmatter must parse; a YAML neuron needs its `#---` block; no
//! conflict markers) and stored as a blob. Nothing else changes until every
//! conflicted path has a choice. Then, in the same call, the engine
//! finishes the merge ([`engine::finish_merge`]) with the message `kluris:
//! sync (resolved)`, `Kluris-Sync: resolved` and `Kluris-Resolved-By`:
//! anything that leaves the brain as it is keeps the choices. Once it
//! landed, the waiting merge goes and the session is told the command
//! committed ([`Session::committed`]), so its close pulls anything newer
//! and pushes.
//!
//! Each call names the waiting merge it answers (its id, shown with every
//! conflict): an id that is not the current record's is refused, with the
//! current conflict, and nothing is recorded ([`pending::check_answer`]).

use std::collections::BTreeMap;
use std::fmt;

use super::claim::TreeClaim;
use super::commit::{PathProblem, normalize_path};
use super::engine::{self, Context};
use super::git::SyncGit;
use super::hooks::MergeHooks;
use super::land::Settled;
use super::pending::{self, Pending};
use super::session::Session;
use super::{ConflictKind, ConflictText, LeftAsIs, LeftReason, ResolvedChoice, SyncError};
use crate::frontmatter;
use crate::git::GitError;

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
}

/// What one call did; the session's close says what the sync did after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveReport {
    /// The path, normalized.
    pub path: String,
    /// `ours`, `theirs` or `file`.
    pub choice: &'static str,
    /// The conflicted paths still without a choice.
    pub remaining: Vec<String>,
    pub committed: bool,
    pub commit: Option<String>,
    /// What failed once the merge landed, and how to repair it.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    NoOpenConflict,
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
    /// Uncommitted files the merge would overwrite; the choices are kept.
    Blocked(Vec<String>),
    /// The merge was left as it is (a hook, a lock, a git that failed, the
    /// tree claim another command held); the choices are kept.
    Left(LeftAsIs),
    /// The waiting merge could not be written or removed.
    Record(String),
    /// The brain cannot be synced: why, as the session refuses it.
    Refused(String),
    Sync(SyncError),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::NoOpenConflict => f.write_str("there is no open sync conflict"),
            ResolveError::WrongMerge { path, wrong, .. } => write!(
                f,
                "{path}: merge {} is not the open conflict: this brain waits on merge {} now, so nothing was recorded; decide again from its sides, then give --merge {}",
                wrong.given, wrong.current, wrong.current
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
                "{path}: git names no file on either side, so no choice can be mapped to files; nothing was recorded (merge the upstream with git and commit: the next sync replaces this waiting merge)"
            ),
            ResolveError::Blocked(paths) => write!(
                f,
                "uncommitted changes are in the way: {}; commit or stash them, then finish the merge (the choices are kept)",
                paths.join(", ")
            ),
            ResolveError::Left(left) => left.fmt(f),
            ResolveError::Record(e) => write!(f, "cannot record the open conflict: {e}"),
            ResolveError::Refused(why) => f.write_str(why),
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

/// Another command held the work tree past the session's bound: its own
/// words when it said so.
fn busy(session: &Session) -> LeftAsIs {
    session
        .report()
        .and_then(|r| r.left_as_is.clone())
        .filter(|left| left.reason == LeftReason::Busy)
        .unwrap_or_else(|| {
            LeftAsIs::because(
                LeftReason::Busy,
                "another kluris command is changing this brain; nothing was recorded: run again once it is done",
            )
        })
}

fn record(e: std::io::Error) -> ResolveError {
    ResolveError::Record(e.to_string())
}

/// Why a person's file cannot be kept for `rel`.
pub(crate) fn validate(rel: &str, bytes: &[u8]) -> Result<(), String> {
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

/// Record a choice for one conflicted path in the brain `session` opened;
/// with the last one, finish the merge (see the module docs).
pub fn resolve(
    session: &mut Session,
    request: &ResolveRequest,
) -> Result<ResolveReport, ResolveError> {
    let report = match (session.context(), session.claim()) {
        (Some(ctx), Some(claim)) => resolve_in(ctx, claim, request)?,
        (Some(_), None) => return Err(ResolveError::Left(busy(session))),
        (None, _) => {
            // Why the session could not open the brain, when it says; a
            // brain with no upstream has no merge waiting.
            let failed = session
                .report()
                .and_then(|r| r.left_as_is.clone())
                .filter(|left| left.reason != LeftReason::NoUpstream);
            return Err(
                match (session.alone().and_then(|why| why.refusal()), failed) {
                    (Some(why), _) => ResolveError::Refused(why),
                    (None, Some(left)) => ResolveError::Left(left),
                    (None, None) => ResolveError::NoOpenConflict,
                },
            );
        }
    };
    if report.committed {
        session.committed();
    }
    Ok(report)
}

/// [`resolve`] in the brain `ctx` opened, under the exclusive tree claim
/// the caller holds (`claim`): the session's, for the command.
pub fn resolve_in<G: SyncGit>(
    ctx: &Context<G>,
    claim: &TreeClaim,
    request: &ResolveRequest,
) -> Result<ResolveReport, ResolveError> {
    let (git, repo, path) = (ctx.git(), ctx.repo(), ctx.pending());
    let mut pending = engine::waiting_merge(ctx)?.ok_or(ResolveError::NoOpenConflict)?;
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
        warning: None,
    };
    if !report.remaining.is_empty() {
        pending.write(&path).map_err(record)?;
        return Ok(report);
    }

    // Every path has a choice: finish the merge. Until the merge commit is
    // in place the waiting merge (with the choices) stays.
    let finished = match finish(ctx, claim, &pending) {
        Ok(finished) => finished,
        Err(e) => {
            pending.write(&path).map_err(record)?;
            return Err(e);
        }
    };
    // The merge landed: what fails from here is a warning.
    let mut warnings: Vec<String> = finished.warning.into_iter().collect();
    if let Err(e) = pending::remove(&path) {
        warnings.push(format!(
            "the waiting merge's record could not be removed ({e}); remove {}",
            path.display()
        ));
    }
    report.committed = true;
    report.commit = Some(finished.commit);
    report.warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    Ok(report)
}

/// The merge with the choices, committed and landed.
fn finish<G: SyncGit>(
    ctx: &Context<G>,
    claim: &TreeClaim,
    pending: &Pending,
) -> Result<engine::Finished, ResolveError> {
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
    let finish = engine::Finish {
        choices: &choices,
        message: &message,
        hooks: MergeHooks::Merge,
        settled: &Settled::new(),
        keep_generated: false,
    };
    Ok(engine::finish_merge(ctx, claim, pending, &finish)?)
}
