//! The hooks `git commit` and `git merge` run, for Kluris's commits and
//! merge commits alike: a gate that can stop the commit ([`gate`]), then
//! `prepare-commit-msg` and `commit-msg` (unless `no_verify`) on a message
//! file ([`message_hooks`]); the message is what they leave there, cleaned
//! up as `git commit` does.
//!
//! A merge commit ([`merge_commit`]) runs its gate (unless `no_verify`)
//! and `prepare-commit-msg` with `merge`, against the merged index: as
//! `git merge` runs them, `pre-merge-commit`; as `git commit` concluding a
//! merge runs them, `pre-commit` ([`MergeHooks`]), then `post-commit` once
//! it landed. A hook that changes that index stops the merge: the hooks see
//! the old work tree, so an ordinary `git add -A` would stage it over the
//! incoming files. A merge hook may change the message, never the files.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::commit::cleanup_message;
use crate::git::{GitError, Plumbing, Repo, StagingIndex};

/// Why the hooks stopped a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookError {
    /// A hook that can stop it failed with this exit code.
    Failed {
        name: &'static str,
        code: i32,
    },
    /// The message hooks left the message empty.
    EmptyMessage,
    /// The message file in the git directory could not be written or read.
    MessageFile(String),
    Git(GitError),
}

impl From<GitError> for HookError {
    fn from(e: GitError) -> Self {
        HookError::Git(e)
    }
}

/// Why the hooks stopped a merge commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeHookError {
    Hook(HookError),
    /// The hook changed the merged index.
    Changed {
        name: &'static str,
    },
}

impl From<HookError> for MergeHookError {
    fn from(e: HookError) -> Self {
        MergeHookError::Hook(e)
    }
}

impl From<GitError> for MergeHookError {
    fn from(e: GitError) -> Self {
        MergeHookError::Hook(HookError::Git(e))
    }
}

impl fmt::Display for MergeHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeHookError::Hook(HookError::Failed { name, code }) => write!(
                f,
                "the {name} hook failed (exit {code}); nothing was merged"
            ),
            MergeHookError::Hook(HookError::EmptyMessage) => {
                f.write_str("the message hooks left the merge message empty; nothing was merged")
            }
            MergeHookError::Hook(HookError::MessageFile(e)) => write!(
                f,
                "cannot write the merge message file: {e}; nothing was merged"
            ),
            MergeHookError::Hook(HookError::Git(e)) => e.fmt(f),
            MergeHookError::Changed { name } => write!(
                f,
                "the {name} hook changed the merge; nothing was merged (a merge hook may change the message, not the files)"
            ),
        }
    }
}

/// Run a hook that can stop the commit, against `index`.
pub fn gate<G: Plumbing>(
    git: &G,
    repo: &Repo,
    index: &StagingIndex,
    name: &'static str,
    args: &[&str],
) -> Result<(), HookError> {
    match git.run_hook(repo, Some(index), name, args)? {
        0 => Ok(()),
        code => Err(HookError::Failed { name, code }),
    }
}

/// The message file in the git directory, removed when dropped, and the
/// message the hooks left in it.
pub struct Message {
    file: PathBuf,
    pub text: String,
}

impl Message {
    pub fn file(&self) -> &Path {
        &self.file
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
    }
}

/// Where a message comes from, as `prepare-commit-msg` is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A commit's own message.
    Message,
    /// A merge commit's.
    Merge,
}

impl Source {
    fn arg(self) -> &'static str {
        match self {
            Source::Message => "message",
            Source::Merge => "merge",
        }
    }

    /// The message file, in the git directory, of this process.
    fn file_name(self) -> String {
        let stem = match self {
            Source::Message => "KLURIS_EDITMSG",
            Source::Merge => "KLURIS_MERGE_MSG",
        };
        format!("{stem}-{}", std::process::id())
    }
}

/// `message` through the message hooks, on a file in the git directory:
/// `prepare-commit-msg <file> <source>`, then `commit-msg <file>` unless
/// `no_verify`, against `index`; `after` runs after each hook. The file
/// holds the cleaned-up message for `git commit-tree -F`.
pub fn message_hooks<G: Plumbing, E: From<HookError>>(
    git: &G,
    repo: &Repo,
    index: &StagingIndex,
    message: &str,
    source: Source,
    no_verify: bool,
    after: impl Fn(&'static str) -> Result<(), E>,
) -> Result<Message, E> {
    let file = git
        .git_dir_path(repo, &source.file_name())
        .map_err(HookError::Git)?;
    let mut done = Message {
        file: file.clone(),
        text: String::new(),
    };
    let written =
        |text: &str| fs::write(&file, text).map_err(|e| HookError::MessageFile(e.to_string()));
    written(message)?;
    // Hooks are shell scripts (Git for Windows' sh too): `/` separators.
    let arg = file.to_string_lossy().replace('\\', "/");
    gate(
        git,
        repo,
        index,
        "prepare-commit-msg",
        &[&arg, source.arg()],
    )?;
    after("prepare-commit-msg")?;
    if !no_verify {
        gate(git, repo, index, "commit-msg", &[&arg])?;
        after("commit-msg")?;
    }
    let text = fs::read(&file).map_err(|e| HookError::MessageFile(e.to_string()))?;
    done.text = cleanup_message(&String::from_utf8_lossy(&text));
    if done.text.is_empty() {
        return Err(HookError::EmptyMessage.into());
    }
    written(&done.text)?;
    Ok(done)
}

/// Which command's hooks a merge commit runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeHooks {
    /// `git merge`'s: `pre-merge-commit` is the gate.
    Merge,
    /// `git commit`'s, concluding a merge: `pre-commit` is the gate, and
    /// `post-commit` runs once the merge landed.
    Commit,
}

impl MergeHooks {
    fn gate(self) -> &'static str {
        match self {
            MergeHooks::Merge => "pre-merge-commit",
            MergeHooks::Commit => "pre-commit",
        }
    }
}

/// The merge commit of the tree in `index` (`merged_tree`) on `parents`,
/// with the hooks of `hooks` (see the module docs); its commit.
#[allow(clippy::too_many_arguments)]
pub fn merge_commit<G: Plumbing>(
    git: &G,
    repo: &Repo,
    index: &StagingIndex,
    merged_tree: &str,
    parents: [&str; 2],
    message: &str,
    no_verify: bool,
    hooks: MergeHooks,
) -> Result<String, MergeHookError> {
    let unchanged = |name: &'static str| -> Result<(), MergeHookError> {
        if git.write_tree(repo, index)? == merged_tree {
            Ok(())
        } else {
            Err(MergeHookError::Changed { name })
        }
    };
    if !no_verify {
        let name = hooks.gate();
        gate(git, repo, index, name, &[])?;
        unchanged(name)?;
    }
    let message = message_hooks(
        git,
        repo,
        index,
        message,
        Source::Merge,
        no_verify,
        unchanged,
    )?;
    Ok(git.commit_tree(repo, merged_tree, &parents, message.file())?)
}
