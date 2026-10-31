//! Moving the branch, the index and the work tree to a commit, never over
//! a file that is not committed ([`land`]), and the checks it makes:
//! [`blocked_paths`] (the uncommitted, untracked or ignored files the move
//! would write over) and the locks git holds while it writes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::git::SyncGit;
use super::{LeftAsIs, LeftReason};
use crate::git::{GitError, Repo, TreeEntry};

/// The file at `rel` (`/`-separated) under `root`.
pub fn join(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn exists_on_disk(root: &Path, rel: &str) -> bool {
    fs::symlink_metadata(join(root, rel)).is_ok()
}

/// Whether every part of `rel` is on disk under exactly that name (Unicode
/// normalization aside): on a case-insensitive file system (macOS,
/// Windows) `notes/Keep.md` also "exists" when only `notes/keep.md` does.
fn exactly_on_disk(root: &Path, rel: &str) -> bool {
    use unicode_normalization::UnicodeNormalization;
    let mut dir = root.to_path_buf();
    for part in rel.split('/') {
        let want: String = part.nfc().collect();
        let Ok(entries) = fs::read_dir(&dir) else {
            return false;
        };
        let found = entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().nfc().collect::<String>() == want);
        if !found {
            return false;
        }
        dir = dir.join(part);
    }
    true
}

/// The uncommitted files (in `dirty`) that moving from `ours` to `target`
/// would overwrite, and the untracked or ignored files a new file would
/// land on.
pub fn blocked_paths<G: SyncGit>(
    git: &G,
    repo: &Repo,
    root: &Path,
    dirty: &[String],
    ours: &str,
    target: &str,
) -> Result<Vec<String>, GitError> {
    let dirty: BTreeSet<&String> = dirty.iter().collect();
    let incoming = git.diff_names(repo, ours, target)?;
    let tracked = git.tracked(repo, ours, &incoming)?;
    // A new path whose only match on disk is a case variant of a tracked
    // file the same change moves or removes (a case-only rename, of a file
    // or a folder) is that file, not something in the way.
    let mut ours_files: Option<Vec<String>> = None;
    let mut blocked = Vec::new();
    for path in &incoming {
        let in_the_way = if dirty.contains(path) {
            true
        } else if tracked.contains(path) || !exists_on_disk(root, path) {
            false
        } else if exactly_on_disk(root, path) {
            true
        } else {
            if ours_files.is_none() {
                ours_files = Some(
                    git.tree_entries(repo, ours)?
                        .into_iter()
                        .map(|e| e.rel)
                        .collect(),
                );
            }
            let lower = path.to_lowercase();
            !ours_files
                .iter()
                .flatten()
                .any(|f| f != path && f.to_lowercase() == lower && incoming.contains(f))
        };
        if in_the_way {
            blocked.push(path.clone());
        }
    }
    Ok(blocked)
}

/// Of the `paths` in the way after git stopped partway, those that are not
/// git's own writes, decided by what checkout writes: a file holding the
/// incoming blob as checkout converts it for its path
/// ([`SyncGit::checked_out`]: the smudge filter, the line endings), a
/// symlink pointing where the incoming one does, or a path gone where the
/// commit removes it, is what git wrote before it stopped; anything else is
/// the user's. A symlink git wrote as a plain file (`core.symlinks` false)
/// holds its target as it is.
fn not_written_by_git<G: SyncGit>(
    git: &G,
    repo: &Repo,
    root: &Path,
    to: &str,
    paths: Vec<String>,
) -> Result<Vec<String>, GitError> {
    let wanted: BTreeSet<&String> = paths.iter().collect();
    let incoming: BTreeMap<String, TreeEntry> = git
        .tree_entries(repo, to)?
        .into_iter()
        .filter(|e| wanted.contains(&e.rel))
        .map(|e| (e.rel.clone(), e))
        .collect();
    let links: Vec<String> = incoming
        .values()
        .filter(|e| e.is_symlink())
        .map(|e| e.oid.clone())
        .collect();
    let targets = git.read_blobs(repo, &links)?;
    let mut mine = Vec::new();
    for rel in paths {
        let path = join(root, &rel);
        let on_disk = fs::symlink_metadata(&path).ok();
        let written = match (incoming.get(&rel), on_disk) {
            (None, on_disk) => on_disk.is_none(),
            (Some(_), None) => false,
            (Some(e), Some(meta)) if e.is_symlink() => {
                let target = targets.get(&e.oid).map(Vec::as_slice);
                if meta.file_type().is_symlink() {
                    fs::read_link(&path).is_ok_and(|link| {
                        let link = link.to_string_lossy().replace('\\', "/");
                        target == Some(link.as_bytes())
                    })
                } else {
                    meta.is_file() && target.is_some_and(|t| fs::read(&path).is_ok_and(|b| b == t))
                }
            }
            (Some(e), Some(meta)) => {
                meta.is_file()
                    && fs::read(&path).is_ok_and(|bytes| {
                        git.checked_out(repo, &rel, &e.oid)
                            .is_ok_and(|want| want == bytes)
                    })
            }
        };
        if !written {
            mine.push(rel);
        }
    }
    Ok(mine)
}

/// Left for the uncommitted `paths` in the way.
pub fn uncommitted(paths: Vec<String>) -> LeftAsIs {
    LeftAsIs {
        reason: LeftReason::Uncommitted,
        paths,
        detail: None,
    }
}

/// Left because a step failed: what failed.
pub fn failed(error: impl std::fmt::Display) -> LeftAsIs {
    LeftAsIs::because(LeftReason::Failed, error.to_string())
}

/// The lock of the current branch's ref or of the index, when one is
/// there: another git is writing to the brain. A move started now would
/// write the work tree and the index, then fail on the ref, leaving the
/// incoming files staged under the old HEAD.
fn held_lock<G: SyncGit>(git: &G, repo: &Repo, branch: &str) -> Result<Option<PathBuf>, GitError> {
    for lock in [format!("{branch}.lock"), "index.lock".to_string()] {
        let path = git.git_dir_path(repo, &lock)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn short(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

/// Why the work tree is no longer the one the engine decided for: a git
/// operation started, HEAD names another branch (or none), or `branch`
/// moved from `ours`. Read under the tree claim, just before git writes.
pub fn changed<G: SyncGit>(
    git: &G,
    repo: &Repo,
    branch: &str,
    ours: &str,
) -> Result<Option<String>, GitError> {
    let busy = |what: String| {
        Some(format!(
            "{what} while kluris synced {branch}; nothing was landed: run again"
        ))
    };
    if let Some(op) = git.in_progress(repo)? {
        return Ok(Some(format!(
            "a {op} started while kluris synced {branch}; nothing was landed: finish or abort it, then run again"
        )));
    }
    match git.branch(repo)? {
        Some(head) if head == branch => {}
        Some(head) => return Ok(busy(format!("HEAD moved to {head}"))),
        None => return Ok(busy("HEAD was detached".into())),
    }
    match git.ref_value(repo, branch)? {
        Some(now) if now == ours => Ok(None),
        Some(now) => Ok(busy(format!(
            "{branch} moved from {} to {}",
            short(ours),
            short(&now)
        ))),
        None => Ok(busy(format!("{branch} was deleted"))),
    }
}

/// [`changed`] as a reason to leave the brain as it is.
pub fn unchanged<G: SyncGit>(
    git: &G,
    repo: &Repo,
    branch: &str,
    ours: &str,
) -> Result<(), LeftAsIs> {
    match changed(git, repo, branch, ours).map_err(failed)? {
        Some(why) => Err(LeftAsIs::because(LeftReason::Busy, why)),
        None => Ok(()),
    }
}

/// Move `branch`, which HEAD names, from `ours` to `to`, a descendant,
/// unless a file that is not committed sits where it writes, a git lock is
/// held, or the work tree is no longer the one decided for ([`changed`]).
/// The caller holds the tree claim. Checked right before the move (a hook
/// or an editor may have written a file, or switched or moved the branch,
/// since the pull decided) and again when git refuses (a file written in
/// between, told apart from what git itself wrote before it stopped): the
/// move itself never overwrites an ignored file nor stashes. git stopping
/// partway is [`LeftReason::Failed`] with its message; nothing is rolled
/// back. What is left open is the moment between the last check and git's
/// own write: a switch or a commit made then is the same race as an edit
/// made then, reported by git's refusal or the second check, not closed.
pub fn land<G: SyncGit>(
    git: &G,
    repo: &Repo,
    root: &Path,
    branch: &str,
    ours: &str,
    to: &str,
) -> Result<(), LeftAsIs> {
    let in_the_way = || -> Result<Vec<String>, LeftAsIs> {
        let dirty = git.dirty(repo).map_err(failed)?;
        blocked_paths(git, repo, root, &dirty, ours, to).map_err(failed)
    };
    let blocked = in_the_way()?;
    if !blocked.is_empty() {
        return Err(uncommitted(blocked));
    }
    if let Some(lock) = held_lock(git, repo, branch).map_err(failed)? {
        return Err(LeftAsIs::because(
            LeftReason::Busy,
            format!(
                "{} exists, so a git is writing to this brain; if none is running, remove that file and run again",
                lock.display()
            ),
        ));
    }
    unchanged(git, repo, branch, ours)?;
    let Err(e) = git.fast_forward_to(repo, to) else {
        return Ok(());
    };
    let blocked = not_written_by_git(git, repo, root, to, in_the_way()?).map_err(failed)?;
    if blocked.is_empty() {
        Err(failed(&e.detail))
    } else {
        Err(uncommitted(blocked))
    }
}
