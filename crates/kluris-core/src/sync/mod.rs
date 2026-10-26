//! Git auto-sync (SPEC Phase 7). `commit` commits approved paths with the
//! maps they change; `fresh` brings a brain up to date before `wake-up` and
//! `search` read it; [`sync`] fetches, merges and pushes a brain.
//!
//! [`sync`], for one brain at the root of its own git repo:
//! 1. refuses a brain nested in a larger repo, one that is not a work tree,
//!    one with a merge, rebase, cherry-pick or revert in progress, and one
//!    with an open sync conflict ([`PENDING`], for `kluris resolve`);
//! 2. fetches the current branch's upstream, never prompting, within the
//!    network time limit; offline (or without an upstream) nothing changes;
//! 3. behind only: fast-forwards; ahead only: pushes; diverged: merges in
//!    memory ([`merge`]) and, without conflicts, commits the merge
//!    (`commit-tree`, two parents, `kluris: sync` with `Kluris-Sync: auto`,
//!    after the `pre-merge-commit` hook), fast-forwards to it and pushes;
//!    with conflicts nothing changes and they are recorded in [`PENDING`];
//! 4. never overwrites a file with uncommitted changes: when the incoming
//!    changes touch one (or a new file would land on an untracked or
//!    ignored file), nothing changes and those paths are `blocked`;
//! 5. a push rejected because the remote moved fetches, merges and pushes
//!    again, [`PUSH_ATTEMPTS`] times at most.

pub mod commit;
pub mod fresh;
pub mod frontmatter_merge;
pub mod merge;
pub mod plumbing;

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::atomic;
use crate::dream::DreamOptions;
use crate::git::{Git, GitError, InProgress, MIN_SYNC_VERSION, parse_version};
use crate::json::{Json, to_string};
use merge::{MergeOutcome, merge_commits};
use plumbing::{PushOutcome, SyncGit};

/// An open sync conflict, relative to the git directory: what `kluris
/// resolve` finishes.
pub const PENDING: &str = "kluris-sync/pending.json";

/// How many times a push rejected because the remote moved is merged and
/// tried again.
pub const PUSH_ATTEMPTS: usize = 5;

/// The time a fetch or a push may take.
pub const NETWORK_TIME_LIMIT: Duration = Duration::from_secs(60);

const MESSAGE: &str = "kluris: sync\n\nKluris-Sync: auto\n";

/// What a person must decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Overlapping edits (or a file that is not markdown, changed on both
    /// sides in a way git cannot merge).
    Content,
    /// Deleted on one side, changed on the other.
    DeleteEdit,
    /// Different `parent` values.
    Parent,
    /// Another frontmatter key changed differently on both sides.
    Frontmatter,
}

impl ConflictKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::Content => "content",
            ConflictKind::DeleteEdit => "delete_edit",
            ConflictKind::Parent => "parent",
            ConflictKind::Frontmatter => "frontmatter",
        }
    }
}

/// One path a person must decide, with its blob on each side (`None`: not
/// there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub kind: ConflictKind,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

/// How to sync.
#[derive(Debug, Clone, Copy)]
pub struct SyncRequest<'a> {
    /// Skip the `pre-merge-commit` hook (`pre-push` always runs).
    pub no_verify: bool,
    /// For the maps rebuilt after a merge.
    pub maps: DreamOptions<'a>,
    /// The time each fetch and push may take.
    pub network_time_limit: Duration,
}

/// What a sync did to one brain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub fetched: bool,
    /// The branch took the remote's commits (a fast-forward or a merge).
    pub merged: bool,
    pub pushed: bool,
    /// Markdown files both sides changed, merged by the rules.
    pub auto_merged: Vec<String>,
    /// Open; nothing changed.
    pub conflicts: Vec<Conflict>,
    /// Every file with uncommitted changes (untracked included), left as
    /// it is.
    pub skipped_dirty: Vec<String>,
    /// No upstream, or the remote could not be reached.
    pub offline: bool,
    /// The uncommitted files the incoming changes would overwrite; when
    /// any, nothing changed.
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// git is older than [`MIN_SYNC_VERSION`].
    GitTooOld {
        found: String,
    },
    NotARepo,
    /// The brain is below the root of a larger repo (`prefix` from it).
    Nested {
        prefix: String,
    },
    InProgress(InProgress),
    /// An open sync conflict, recorded at this path.
    OpenConflict(PathBuf),
    /// A hook stopped the merge.
    Hook {
        name: &'static str,
        code: i32,
    },
    /// The merge landed locally but the push was refused.
    Push(String),
    /// Every push was rejected because the remote kept moving.
    RemoteKeptMoving,
    /// The open conflict could not be recorded.
    Pending(String),
    Git(GitError),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::GitTooOld { found } => {
                let (major, minor, _) = MIN_SYNC_VERSION;
                write!(
                    f,
                    "kluris sync needs git {major}.{minor} or newer; this is {found}"
                )
            }
            SyncError::NotARepo => f.write_str("it is not a git repository"),
            SyncError::Nested { prefix } => write!(
                f,
                "it is inside a larger git repository (at {}); kluris sync only moves a repository that is the brain itself",
                prefix.trim_end_matches('/')
            ),
            SyncError::InProgress(op) => {
                write!(f, "a {op} is in progress; finish or abort it first")
            }
            SyncError::OpenConflict(path) => write!(
                f,
                "it has an open sync conflict; settle it with `kluris resolve` (recorded in {})",
                path.display()
            ),
            SyncError::Hook { name, code } => write!(
                f,
                "the {name} hook failed (exit {code}); nothing was merged"
            ),
            SyncError::Push(detail) => {
                write!(f, "merged locally, but the push was refused: {detail}")
            }
            SyncError::RemoteKeptMoving => write!(
                f,
                "the remote kept moving: {PUSH_ATTEMPTS} pushes were rejected; run kluris sync again"
            ),
            SyncError::Pending(e) => write!(f, "cannot record the open conflict: {e}"),
            SyncError::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<GitError> for SyncError {
    fn from(e: GitError) -> Self {
        SyncError::Git(e)
    }
}

impl SyncError {
    /// Whether it is a refusal (the brain was left alone), as opposed to a
    /// step that failed.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            SyncError::NotARepo
                | SyncError::Nested { .. }
                | SyncError::InProgress(_)
                | SyncError::OpenConflict(_)
        )
    }
}

/// The precondition for every brain: git >= 2.38 (`merge-tree
/// --write-tree`).
pub fn check_git(git: &impl Git) -> Result<(), SyncError> {
    let found = git.version()?;
    if parse_version(&found).is_some_and(|v| v >= MIN_SYNC_VERSION) {
        Ok(())
    } else {
        Err(SyncError::GitTooOld { found })
    }
}

fn exists_on_disk(root: &Path, rel: &str) -> bool {
    let path = rel
        .split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part));
    fs::symlink_metadata(path).is_ok()
}

fn opt(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

/// Record an open conflict for `kluris resolve`, atomically.
fn write_pending(
    path: &Path,
    base: &Option<String>,
    ours: &str,
    theirs: &str,
    conflicts: &[Conflict],
) -> Result<(), SyncError> {
    let conflicts = conflicts
        .iter()
        .map(|c| {
            Json::Object(vec![
                ("path".into(), Json::Str(c.path.clone())),
                ("kind".into(), Json::Str(c.kind.as_str().into())),
                ("base".into(), opt(&c.base)),
                ("ours".into(), opt(&c.ours)),
                ("theirs".into(), opt(&c.theirs)),
            ])
        })
        .collect();
    let doc = Json::Object(vec![
        ("base".into(), opt(base)),
        ("ours".into(), Json::Str(ours.into())),
        ("theirs".into(), Json::Str(theirs.into())),
        ("conflicts".into(), Json::Array(conflicts)),
        (
            "created".into(),
            Json::Str(jiff::Timestamp::now().to_string()),
        ),
    ]);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| SyncError::Pending(e.to_string()))?;
    }
    let mut text = to_string(&doc);
    text.push('\n');
    atomic::write_atomic(path, text.as_bytes()).map_err(|e| SyncError::Pending(e.to_string()))
}

/// Sync the brain at `root` (see the module docs).
pub fn sync<G: SyncGit>(
    root: &Path,
    git: &G,
    request: &SyncRequest,
) -> Result<SyncReport, SyncError> {
    check_git(git)?;
    let repo = git.repo(root).ok_or(SyncError::NotARepo)?;
    if repo.nested() {
        return Err(SyncError::Nested {
            prefix: repo.prefix.clone(),
        });
    }
    let pending = git.git_dir_path(&repo, PENDING)?;
    if fs::symlink_metadata(&pending).is_ok() {
        return Err(SyncError::OpenConflict(pending));
    }
    if let Some(op) = git.in_progress(&repo)? {
        return Err(SyncError::InProgress(op));
    }
    let mut report = SyncReport {
        skipped_dirty: git.dirty(&repo)?,
        ..SyncReport::default()
    };
    let Some(tracking) = git.tracking(&repo)? else {
        report.offline = true;
        return Ok(report);
    };
    let dirty: BTreeSet<&String> = report.skipped_dirty.iter().collect();
    let mut blocked = Vec::new();

    for _ in 0..PUSH_ATTEMPTS {
        let deadline = Instant::now() + request.network_time_limit;
        if git.fetch(&repo.dir, &tracking.remote, deadline).is_err() {
            report.offline = true;
            return Ok(report);
        }
        report.fetched = true;
        let ours = git
            .head(&repo)?
            .ok_or_else(|| SyncError::Git(no_commit_yet()))?;
        let theirs = git.rev_parse(&repo, &tracking.refname)?;
        let has = |a: &str, b: &str| git.is_ancestor(&repo.dir, a, b, far_future());
        let behind = !has(&theirs, &ours).map_err(fresh_error)?;
        let ahead = !has(&ours, &theirs).map_err(fresh_error)?;

        if behind {
            // The tree the branch will hold: theirs, or the merge.
            let (target, merge) = if ahead {
                match merge_commits(git, &repo, &ours, &theirs, &request.maps)? {
                    MergeOutcome::Conflicts { base, conflicts } => {
                        write_pending(&pending, &base, &ours, &theirs, &conflicts)?;
                        report.conflicts = conflicts;
                        return Ok(report);
                    }
                    MergeOutcome::Clean {
                        tree,
                        index,
                        auto_merged,
                    } => (tree, Some((index, auto_merged))),
                }
            } else {
                (theirs.clone(), None)
            };
            let incoming = git.diff_names(&repo, &ours, &target)?;
            let tracked = git.tracked(&repo, &ours, &incoming)?;
            for path in &incoming {
                if dirty.contains(path) || (!tracked.contains(path) && exists_on_disk(root, path)) {
                    blocked.push(path.clone());
                }
            }
            if !blocked.is_empty() {
                report.blocked = blocked;
                return Ok(report);
            }
            let to = match merge {
                None => theirs.clone(),
                Some((index, auto_merged)) => {
                    if !request.no_verify {
                        match git.run_hook(&repo, Some(&index), "pre-merge-commit", &[])? {
                            0 => {}
                            code => {
                                return Err(SyncError::Hook {
                                    name: "pre-merge-commit",
                                    code,
                                });
                            }
                        }
                    }
                    // The hook may have changed the index, as with git merge.
                    let tree = git.write_tree(&repo, &index)?;
                    let file = git
                        .git_dir_path(&repo, &format!("KLURIS_MERGE_MSG-{}", std::process::id()))?;
                    fs::write(&file, MESSAGE).map_err(|e| SyncError::Pending(e.to_string()))?;
                    let commit = git.commit_tree(&repo, &tree, &[&ours, &theirs], &file);
                    let _ = fs::remove_file(&file);
                    for path in auto_merged {
                        if !report.auto_merged.contains(&path) {
                            report.auto_merged.push(path);
                        }
                    }
                    commit?
                }
            };
            git.fast_forward_to(&repo, &to)?;
            report.merged = true;
            if !ahead {
                return Ok(report);
            }
        } else if !ahead {
            return Ok(report);
        }

        let deadline = Instant::now() + request.network_time_limit;
        match git.push(&repo, &tracking, deadline) {
            PushOutcome::Pushed => {
                report.pushed = true;
                return Ok(report);
            }
            PushOutcome::RemoteMoved => continue,
            PushOutcome::Unreachable => {
                report.offline = true;
                return Ok(report);
            }
            PushOutcome::Failed(detail) => return Err(SyncError::Push(detail)),
        }
    }
    Err(SyncError::RemoteKeptMoving)
}

/// Local steps have no deadline; [`fresh::FreshGit::is_ancestor`] takes one.
fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

fn fresh_error(e: fresh::FreshError) -> SyncError {
    match e {
        fresh::FreshError::Failed(e) => SyncError::Git(e),
        fresh::FreshError::TimedOut => SyncError::Git(GitError {
            args: vec!["merge-base".into(), "--is-ancestor".into()],
            code: None,
            detail: "timed out".into(),
        }),
    }
}

fn no_commit_yet() -> GitError {
    GitError {
        args: vec!["rev-parse".into(), "HEAD".into()],
        code: None,
        detail: "the branch has no commit yet: commit first".into(),
    }
}
