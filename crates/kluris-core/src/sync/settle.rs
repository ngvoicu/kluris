//! `kluris commit` inside its session: a plain commit ([`commit`]), or,
//! while a merge waits for a person, finishing that merge from the files
//! they settled in the work tree ([`commit_in`]).
//!
//! A waiting merge is the record whose `ours` is the pinned branch's commit
//! now ([`engine::waiting_merge`]). While one waits, `kluris commit` only
//! finishes it, and only as asked:
//! - `--merge` must name it ([`pending::check_answer`]); a commit without
//!   one, or with another id, is refused;
//! - a choice `kluris resolve` recorded in the waiting merge stands, for a
//!   `structure` path or a text one, and that path need not be given;
//! - a `structure` path is never settled from a file: one file says
//!   nothing about the directory or the rename, which `kluris resolve
//!   --ours|--theirs` settles. Without a recorded choice it is refused, and
//!   so is one given in the paths;
//! - every other conflicted path without a recorded choice must be given,
//!   and no path that is not conflicted: an unrelated path is committed
//!   after the merge, on the merged files, so a teammate's clean change to
//!   it is never reverted and no edit hides in a merge. A name `--paths`
//!   cannot carry ([`super::advice::fits_paths`]) is settled with `kluris
//!   resolve` alone. With a choice recorded for every conflicted path, the
//!   paths may name none, and `--paths` may be left out.
//!
//! Each given path's file is its choice, in place of one recorded for it
//! as a later `kluris resolve` replaces an earlier one, checked as `kluris
//! resolve --file` checks one (no conflict markers, frontmatter that
//! parses); a path gone from disk is a deletion, unless a folder on its
//! way is a file in the work tree: no file could be written there, so the
//! path is refused and settled with `kluris resolve`. The merge commit has the
//! two commits the person was shown as its parents, every map rebuilt from
//! the merged files, the message `kluris: sync (resolved)` (or the
//! person's) with the `Kluris-Sync: resolved`, `Kluris-Resolved-By`,
//! `Kluris-Approved-By` and `Co-Authored-By` trailers, and the hooks `git
//! commit` runs to conclude a merge. It lands over the settled files
//! ([`super::land`]); the record goes once it landed. Every refusal changes
//! nothing, and each one while a merge waits says what finishing it still
//! needs ([`MergeNeeds`], in the lines of [`super::advice`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::advice::{MergeNeeds, Settle, file_above};
use super::claim::TreeClaim;
use super::commit::{
    CommitError, CommitReport, CommitRequest, check_on_disk, commit, normalize_path, one_line,
};
use super::engine::{self, Context, Finish};
use super::git::SyncGit;
use super::hooks::MergeHooks;
use super::land::{Settled, join};
use super::merge::is_generated;
use super::pending::{self, Pending};
use super::resolve::validate;
use super::{ConflictKind, ResolvedChoice};

/// The merge commit's subject unless the person gave a message.
const RESOLVED: &str = "kluris: sync (resolved)";

/// Commit `request.paths` in the brain at `root`: with the session's brain
/// and its exclusive tree claim (`claimed`), the waiting merge `merge`
/// names is finished from them while one waits (see the module docs);
/// otherwise a plain [`commit`], and `merge` is refused.
pub fn commit_in<G: SyncGit>(
    root: &Path,
    git: &G,
    claimed: Option<(&Context<G>, &TreeClaim)>,
    request: &CommitRequest,
    merge: Option<&str>,
) -> Result<CommitReport, CommitError> {
    if let Some((ctx, claim)) = claimed
        && let Some(waiting) = engine::waiting_merge(ctx).map_err(CommitError::Left)?
    {
        return finish(root, ctx, claim, &waiting, request, merge);
    }
    if let Some(given) = merge {
        return Err(CommitError::NoMergeWaits {
            given: given.to_string(),
        });
    }
    commit(root, git, request)
}

/// The given paths, normalized, in order, without repeats.
fn normalized(paths: &[String]) -> Result<Vec<String>, CommitError> {
    let mut out: Vec<String> = Vec::new();
    for raw in paths {
        let rel = normalize_path(raw).map_err(|problem| CommitError::Path {
            path: raw.clone(),
            problem,
        })?;
        if !out.contains(&rel) {
            out.push(rel);
        }
    }
    Ok(out)
}

/// What `waiting` still needs of `kluris resolve` and `--paths`, given
/// the choices `recorded` in it.
fn needs(
    root: &Path,
    brain: &str,
    waiting: &Pending,
    recorded: &BTreeMap<String, ResolvedChoice>,
) -> MergeNeeds {
    MergeNeeds {
        brain: brain.to_string(),
        merge: waiting.id(),
        open: waiting
            .conflicts
            .iter()
            .filter(|c| !recorded.contains_key(&c.path))
            .map(|c| {
                let structure = c.kind == ConflictKind::Structure;
                (c.path.clone(), Settle::of(root, &c.path, structure, false))
            })
            .collect(),
    }
}

/// Finish `waiting` from the settled files (see the module docs).
fn finish<G: SyncGit>(
    root: &Path,
    ctx: &Context<G>,
    claim: &TreeClaim,
    waiting: &Pending,
    request: &CommitRequest,
    merge: Option<&str>,
) -> Result<CommitReport, CommitError> {
    let (git, repo) = (ctx.git(), ctx.repo());
    let conflicted: Vec<String> = waiting.conflicts.iter().map(|c| c.path.clone()).collect();
    let recorded: BTreeMap<String, ResolvedChoice> = waiting.resolved.iter().cloned().collect();
    let needs = needs(root, request.brain, waiting, &recorded);
    let Some(merge) = merge else {
        return Err(CommitError::MergeWaits(needs));
    };
    pending::check_answer(waiting, merge).map_err(CommitError::WrongMerge)?;
    if let Some(op) = git.in_progress(repo)? {
        return Err(CommitError::InProgress(op));
    }
    let paths = normalized(request.paths)?;
    // A structure path is settled by a recorded side alone.
    if let Some(c) = waiting.conflicts.iter().find(|c| {
        c.kind == ConflictKind::Structure
            && (paths.contains(&c.path) || !recorded.contains_key(&c.path))
    }) {
        return Err(CommitError::Structure {
            path: c.path.clone(),
            recorded: recorded.contains_key(&c.path),
            needs,
        });
    }
    // A path `--paths` cannot name is settled with `kluris resolve` alone.
    let missing: Vec<String> = needs
        .open
        .iter()
        .filter(|(rel, how)| *how != Settle::Side && !paths.contains(rel))
        .map(|(rel, _)| rel.clone())
        .collect();
    if !missing.is_empty() {
        return Err(CommitError::Unsettled { missing, needs });
    }
    let rest: Vec<String> = paths
        .iter()
        .filter(|rel| !conflicted.contains(rel))
        .cloned()
        .collect();
    if !rest.is_empty() {
        return Err(CommitError::Unrelated { needs, rest });
    }

    // The recorded choices, then each given file in place of its path's:
    // a file gone is a deletion.
    let mut choices = recorded;
    let mut settled = Settled::new();
    for rel in &paths {
        check_on_disk(root, rel, true).map_err(|problem| CommitError::Path {
            path: rel.clone(),
            problem,
        })?;
        let path = join(root, rel);
        if fs::symlink_metadata(&path).is_err() {
            // No file can be there: its absence says nothing.
            if let Some(file) = file_above(root, rel) {
                return Err(CommitError::Unwritable {
                    path: rel.clone(),
                    file,
                    needs,
                });
            }
            choices.insert(rel.clone(), ResolvedChoice::Removed);
            settled.insert(rel.clone(), None);
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| CommitError::InvalidFile {
            path: rel.clone(),
            reason: e.to_string(),
        })?;
        validate(rel, &bytes).map_err(|reason| CommitError::InvalidFile {
            path: rel.clone(),
            reason,
        })?;
        choices.insert(
            rel.clone(),
            ResolvedChoice::Blob(git.hash_blob(repo, &bytes)?),
        );
        settled.insert(rel.clone(), Some(bytes));
    }

    let message = resolved_message(git, repo, request)?;
    let finished = engine::finish_merge(
        ctx,
        claim,
        waiting,
        &Finish {
            choices: &choices,
            message: &message,
            hooks: MergeHooks::Commit,
            settled: &settled,
            keep_generated: true,
        },
    )
    .map_err(CommitError::Left)?;
    // The merge landed: what fails from here is a warning.
    let engine::Finished {
        commit: finished,
        warning,
    } = finished;
    let mut warnings: Vec<String> = warning.into_iter().collect();
    let record = ctx.pending();
    if let Err(e) = pending::remove(&record) {
        warnings.push(format!(
            "the waiting merge's record could not be removed ({e}); remove {}",
            record.display()
        ));
    }
    let regenerated: Vec<String> = match git.diff_names(repo, &waiting.ours, &finished) {
        Ok(files) => files.into_iter().filter(|rel| is_generated(rel)).collect(),
        Err(e) => {
            warnings.push(format!(
                "the maps the merge rebuilt could not be listed ({e})"
            ));
            Vec::new()
        }
    };
    let message = commit_message(git, repo, &finished).unwrap_or_else(|e| {
        warnings.push(format!("the merge's message could not be read ({e})"));
        message
    });
    let left_dirty = git.dirty(repo).unwrap_or_else(|e| {
        warnings.push(format!("the files left changed could not be listed ({e})"));
        Vec::new()
    });
    Ok(CommitReport {
        message: Some(message),
        commit: Some(finished),
        paths,
        regenerated,
        linked: Vec::new(),
        left_dirty,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
        set: None,
    })
}

/// The merge commit's message: the person's, else [`RESOLVED`], with its
/// trailers.
fn resolved_message<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    request: &CommitRequest,
) -> Result<String, CommitError> {
    let author = git.author(repo)?;
    let person = git
        .config(repo, "user.name")
        .unwrap_or_else(|| author.name.clone());
    let person = one_line(&person);
    let subject = match request.message {
        Some(message) if !message.trim().is_empty() => message.trim_end(),
        _ => RESOLVED,
    };
    let mut message = format!(
        "{subject}\n\nKluris-Sync: resolved\nKluris-Resolved-By: {person}\nKluris-Approved-By: {person}\n"
    );
    if let Some(agent) = request
        .agent
        .map(str::trim)
        .filter(|agent| !agent.is_empty())
    {
        message.push_str(&format!("Co-Authored-By: {}\n", one_line(agent)));
    }
    Ok(message)
}

/// The message of `commit` as the hooks left it.
fn commit_message<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    commit: &str,
) -> Result<String, CommitError> {
    let objects = git.read_blobs(repo, &[commit.to_string()])?;
    let raw = objects
        .get(commit)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    Ok(raw
        .split_once("\n\n")
        .map_or(String::new(), |(_, message)| message.to_string()))
}
