//! The housekeeping commit `dream` and `mri` make once their own work is
//! done: dream's steps run again over a [`TreeStore`] of HEAD's files,
//! never the work tree (dates from git, a missing `parent: ./map.md`,
//! reverse `related` links, every `map.md`, `brain.md`), and whatever they
//! change is committed on HEAD as `kluris: dream` with a `Kluris-Sync:
//! dream` trailer, as the person, through the path `kluris commit` takes:
//! a temporary index, the `pre-commit`, `prepare-commit-msg` and
//! `commit-msg` hooks, `git commit-tree`, the compare-and-swap of HEAD,
//! and `post-commit`.
//!
//! Built from HEAD, the commit never holds an uncommitted file's text (not
//! even a title in a map), and two clones that tidy the same history make
//! the same blobs, so their commits merge with no conflict; only a map's
//! `updated` date can differ, and a merge rebuilds every map. The fixes an
//! uncommitted file causes (the reverse link a new neuron adds, its line
//! in a map) stay in the work tree, where dream wrote them.
//!
//! A neuron is taken only when nothing of the person's is in it: its
//! index entry is HEAD's blob or the commit's, and its work-tree copy is
//! HEAD's or exactly the commit's (dream already wrote the same fix
//! there). Any other neuron is left out, its fixes staying in the work
//! tree, and a version the person staged stays staged. A generated file
//! (`map.md`, `brain.md`) is always taken, so no map in history lags its
//! neurons; a version of it the person staged keeps its index entry.
//!
//! Afterwards each committed file's index entry and work-tree copy get the
//! committed bytes where they had nothing of the person's, as `kluris
//! commit` does for its maps.
//! A hook that stops the commit, or changes what it would commit, leaves
//! everything as it was. Nothing to commit makes no commit.

use std::collections::{BTreeMap, BTreeSet};

use super::LeftAsIs;
use super::LeftReason;
use super::commit::{Seen, Untouched, base_nodes, changed_paths, untouched, update_worktree_copy};
use super::hooks::{HookError, Source, gate, message_hooks};
use super::land::{failed, join};
use super::merge::is_generated;
use crate::brain::tree_store::TreeStore;
use crate::dream::{DreamOptions, housekeeping_changes};
use crate::git::{Plumbing, Repo, StagedBlob};

/// The housekeeping commit's message.
pub const MESSAGE: &str = "kluris: dream\n\nKluris-Sync: dream\n";

/// What is said of fixes left uncommitted.
const LEFT: &str = "the housekeeping was not committed and its fixes are in the work tree";

/// Commit the brain's own housekeeping on HEAD (see the module docs), in
/// the brain at `repo` (the repository's root), whose work tree the caller
/// holds exclusively. The files committed, sorted; none when there was
/// nothing to commit, while a merge, rebase, cherry-pick or revert is in
/// progress, or on a branch with no commit yet. Every git runs to its end:
/// a commit stopped partway would leave HEAD and the index apart.
pub fn commit<G: Plumbing>(
    git: &G,
    repo: &Repo,
    maps: &DreamOptions,
) -> Result<Vec<String>, LeftAsIs> {
    if git.in_progress(repo)?.is_some() {
        return Ok(Vec::new());
    }
    let Some(base) = git.head(repo)? else {
        return Ok(Vec::new());
    };
    let (nodes, modes) = base_nodes(git, repo, &base)?;
    let view = TreeStore::new(&repo.dir, nodes);
    let (changes, _skipped) = housekeeping_changes(&view, &git.file_dates(repo), maps);
    let blobs: Vec<StagedBlob> = changes
        .writes
        .into_iter()
        .map(|write| StagedBlob {
            mode: modes
                .get(&write.rel)
                .cloned()
                .unwrap_or_else(|| "100644".into()),
            rel: write.rel,
            bytes: Some(write.contents),
        })
        .collect();
    if blobs.is_empty() {
        return Ok(Vec::new());
    }
    // Each file as it is before its cleanliness is checked, so one changed
    // after the check is never replaced.
    let seen: BTreeMap<String, Seen> = blobs
        .iter()
        .map(|blob| (blob.rel.clone(), Seen::of(&join(&repo.dir, &blob.rel))))
        .collect();
    let dirty: BTreeSet<String> = git.dirty(repo)?.into_iter().collect();
    let theirs = untouched(git, repo, &repo.dir, Some(&base), &blobs, &dirty)?;
    let is_untouched = |rel: &str, part: fn(&Untouched) -> bool| theirs.get(rel).is_some_and(part);
    // A neuron with anything of the person's in it is left out.
    let blobs: Vec<StagedBlob> = blobs
        .into_iter()
        .filter(|blob| is_generated(&blob.rel) || is_untouched(&blob.rel, |u| u.both()))
        .collect();
    let paths: Vec<String> = blobs.iter().map(|blob| blob.rel.clone()).collect();

    let index = git.staging_index(repo, Some(&base))?;
    git.stage_blobs(repo, &index, &blobs)?;
    let tree = git.write_tree(repo, &index)?;
    let files = changed_paths(git, repo, Some(&base), &tree, &paths)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    gate(git, repo, &index, "pre-commit", &[]).map_err(stopped)?;
    if git.write_tree(repo, &index)? != tree {
        return Err(LeftAsIs::because(
            LeftReason::Hook,
            format!("the pre-commit hook changed the housekeeping commit, so {LEFT}"),
        ));
    }
    let message = message_hooks(git, repo, &index, MESSAGE, Source::Message, false, |_| {
        Ok::<(), HookError>(())
    })
    .map_err(stopped)?;
    let new = git.commit_tree(repo, &tree, &[&base], message.file())?;
    git.update_head(repo, &new, Some(&base), "kluris dream: housekeeping")
        .map_err(|e| {
            LeftAsIs::because(
                LeftReason::Busy,
                format!(
                    "HEAD moved while the housekeeping commit was made, so {LEFT}: run again ({e})"
                ),
            )
        })?;
    drop(index);
    let files: Vec<String> = files.into_iter().collect();
    // A version of a generated file the person staged stays staged.
    let index_files: Vec<String> = files
        .iter()
        .filter(|rel| is_untouched(rel, |u| u.index))
        .cloned()
        .collect();
    git.reset_paths(repo, &new, &index_files).map_err(|e| {
        failed(format!(
            "committed the housekeeping as {new}, but the index was not updated for its files ({e})"
        ))
    })?;
    for blob in &blobs {
        if let Some(bytes) = &blob.bytes
            && files.contains(&blob.rel)
            && let Some(seen) = seen.get(&blob.rel)
        {
            let clean = is_untouched(&blob.rel, |u| u.work_tree);
            update_worktree_copy(&repo.dir, &blob.rel, bytes, clean, seen);
        }
    }
    // As with `git commit`, post-commit cannot undo or fail the commit.
    let _ = git.run_hook(repo, None, "post-commit", &[]);
    Ok(files)
}

/// Why the hooks stopped the housekeeping commit.
fn stopped(e: HookError) -> LeftAsIs {
    match e {
        HookError::Failed { name, code } => LeftAsIs::because(
            LeftReason::Hook,
            format!("the {name} hook failed (exit {code}), so {LEFT}"),
        ),
        HookError::EmptyMessage => LeftAsIs::because(
            LeftReason::Hook,
            format!("the message hooks left the housekeeping commit's message empty, so {LEFT}"),
        ),
        HookError::MessageFile(e) => failed(format!(
            "cannot write the housekeeping commit's message file ({e}), so {LEFT}"
        )),
        HookError::Git(e) => e.into(),
    }
}
