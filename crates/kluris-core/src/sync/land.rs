//! Moving the branch, the index and the work tree to a commit, never over
//! a file that is not committed ([`land`]), and the checks it makes:
//! [`blocked_paths`] (the uncommitted, untracked or ignored files the move
//! would write over) and the locks git holds while it writes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::git::SyncGit;
use super::{LeftAsIs, LeftReason};
use crate::git::{GitError, Repo};

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
fn held_lock<G: SyncGit>(git: &G, repo: &Repo) -> Result<Option<PathBuf>, GitError> {
    let mut locks = vec!["index.lock".to_string()];
    if let Some(branch) = git.symbolic_target(repo, "HEAD")? {
        locks.insert(0, format!("{branch}.lock"));
    }
    for lock in locks {
        let path = git.git_dir_path(repo, &lock)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Move the branch to `to`, a descendant of `ours`, unless a file that is
/// not committed sits where it writes or a git lock is held. Checked right
/// before the move (a hook or an editor may have written one since the
/// pull decided) and again when git refuses (one written in between): the
/// move itself never overwrites an ignored file nor stashes. git stopping
/// partway is [`LeftReason::Failed`] with its message; nothing is rolled
/// back.
pub fn land<G: SyncGit>(
    git: &G,
    repo: &Repo,
    root: &Path,
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
    if let Some(lock) = held_lock(git, repo).map_err(failed)? {
        return Err(LeftAsIs::because(
            LeftReason::Busy,
            format!(
                "{} exists, so a git is writing to this brain; if none is running, remove that file and run again",
                lock.display()
            ),
        ));
    }
    match git.fast_forward_to(repo, to) {
        Ok(()) => Ok(()),
        Err(e) => match in_the_way()? {
            blocked if blocked.is_empty() => Err(failed(&e.detail)),
            blocked => Err(uncommitted(blocked)),
        },
    }
}
