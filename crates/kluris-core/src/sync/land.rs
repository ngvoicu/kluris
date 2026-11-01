//! Moving the branch, the index and the work tree to a commit, never over
//! a file that is not committed ([`land`]), and the checks it makes:
//! [`blocked_paths`] (the uncommitted, untracked or ignored files the move
//! would write over) and the locks git holds while it writes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::git::SyncGit;
use super::{LeftAsIs, LeftReason, SyncError};
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
        stopped: false,
    }
}

/// Left because a step failed: what failed.
pub fn failed(error: impl std::fmt::Display) -> LeftAsIs {
    LeftAsIs::because(LeftReason::Failed, error.to_string())
}

/// A git that failed, or was stopped at its deadline ([`LeftAsIs::stopped`]).
impl From<GitError> for LeftAsIs {
    fn from(e: GitError) -> LeftAsIs {
        LeftAsIs {
            stopped: e.stopped,
            ..failed(e)
        }
    }
}

/// A brain that could not be opened: its git's failure or stop as
/// [`From<GitError>`] has it, anything else failed.
impl From<SyncError> for LeftAsIs {
    fn from(e: SyncError) -> LeftAsIs {
        match e {
            SyncError::Git(e) => e.into(),
            e => failed(e),
        }
    }
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

/// Why the work tree is not the one the engine decided for: a git
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

/// After git's write: `branch` holds `to`, or HEAD named another branch
/// when git ran and git moved that one, which is reported (busy) with the
/// branch that moved and to what; nothing is undone. HEAD naming another
/// branch once `branch` holds `to` is a switch made after the land, and
/// harmless.
fn moved_elsewhere<G: SyncGit>(
    git: &G,
    repo: &Repo,
    branch: &str,
    to: &str,
) -> Result<(), LeftAsIs> {
    let now = git.ref_value(repo, branch)?;
    if now.as_deref() == Some(to) {
        return Ok(());
    }
    let stays = now
        .as_deref()
        .map_or("no commit".to_string(), |c| short(c).to_string());
    let moved = match git.branch(repo)? {
        Some(head) if git.ref_value(repo, &head)?.as_deref() == Some(to) => {
            format!(
                "{head} moved to {} instead of {branch}: HEAD named it when git landed the pull",
                short(to)
            )
        }
        Some(head) => format!("HEAD named {head} when git landed the pull, not {branch}"),
        None => "HEAD was detached when git landed the pull".to_string(),
    };
    Err(LeftAsIs::because(
        LeftReason::Busy,
        format!(
            "{moved}; {branch} stays at {stays}: check the branch HEAD names (git reflog) and run again"
        ),
    ))
}

/// [`changed`] as a reason to leave the brain as it is.
pub fn unchanged<G: SyncGit>(
    git: &G,
    repo: &Repo,
    branch: &str,
    ours: &str,
) -> Result<(), LeftAsIs> {
    match changed(git, repo, branch, ours)? {
        Some(why) => Err(LeftAsIs::because(LeftReason::Busy, why)),
        None => Ok(()),
    }
}

/// Move `branch`, which HEAD names, from `ours` to `to`, a descendant,
/// unless a file that is not committed sits where it writes, a git lock is
/// held, or the work tree is not the one decided for ([`changed`]). Those
/// checks run on `git`, stopped at its deadline when it has one; the move,
/// the check after it and the reads after a refusal on `landing`, never
/// stopped: a move stopped partway would leave the work tree half moved.
/// The caller holds the tree claim. Checked right before the move (a hook
/// or an editor may have written a file, or switched or moved the branch,
/// since the pull decided) and again when git refuses (a file written in
/// between, told apart from what git itself wrote before it stopped): the
/// move itself never overwrites an ignored file nor stashes. git stopping
/// partway is [`LeftReason::Failed`] with its message; nothing is rolled
/// back. The moment between the last check and git's own write stays open
/// (git's fast-forward moves whichever branch HEAD names when it runs): an
/// edit made then is found by git's refusal or the second check, and a
/// switch made then by the check after the write ([`moved_elsewhere`]),
/// which reports it; nothing is undone.
pub fn land<G: SyncGit>(
    git: &G,
    landing: &G,
    repo: &Repo,
    root: &Path,
    branch: &str,
    ours: &str,
    to: &str,
) -> Result<(), LeftAsIs> {
    let in_the_way = |git: &G| -> Result<Vec<String>, LeftAsIs> {
        let dirty = git.dirty(repo)?;
        Ok(blocked_paths(git, repo, root, &dirty, ours, to)?)
    };
    let blocked = in_the_way(git)?;
    if !blocked.is_empty() {
        return Err(uncommitted(blocked));
    }
    if let Some(lock) = held_lock(git, repo, branch)? {
        return Err(LeftAsIs::because(
            LeftReason::Busy,
            format!(
                "{} exists, so a git is writing to this brain; if none is running, remove that file and run again",
                lock.display()
            ),
        ));
    }
    unchanged(git, repo, branch, ours)?;
    let Err(e) = landing.fast_forward_to(repo, to) else {
        return moved_elsewhere(landing, repo, branch, to);
    };
    let blocked = not_written_by_git(landing, repo, root, to, in_the_way(landing)?)?;
    if blocked.is_empty() {
        Err(failed(&e.detail))
    } else {
        Err(uncommitted(blocked))
    }
}
