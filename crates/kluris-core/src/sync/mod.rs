//! Git sync: the engine that brings teammates' commits into a brain and
//! shares what is committed ([`engine`]), and what it is built on.
//!
//! - [`git`]: the git steps beyond plumbing, never prompting;
//! - [`fetch`]: the upstream's commit, fetched and its tracking ref moved;
//! - [`claim`]: the remote claim and the tree claim commands share a
//!   checkout by;
//! - [`merge`] and [`frontmatter_merge`]: the in-memory merge and its rules;
//! - [`hooks`]: the gate and message hooks of commits and merge commits;
//! - [`land`]: moving the work tree, never over an uncommitted file;
//! - [`pending`]: the waiting merge a person settles;
//! - [`session`]: the one step through which every command that opens a
//!   brain syncs; [`background`]: the detached push of the commits a read
//!   leaves waiting;
//! - [`commit`]: `kluris commit`, and [`settle`]: `kluris commit` in its
//!   session, a waiting merge finished from the work tree; [`resolve`]:
//!   `kluris resolve`.
//!
//! This module holds the types they share.

pub mod background;
pub mod claim;
pub mod commit;
pub mod engine;
pub mod fetch;
pub mod frontmatter_merge;
pub mod git;
pub mod hooks;
pub mod land;
pub mod merge;
pub mod pending;
pub mod resolve;
pub mod session;
pub mod settle;

use std::fmt;
use std::time::Duration;

use crate::dream::DreamOptions;
use crate::git::{GitError, MIN_SYNC_VERSION};

/// The waiting merge, relative to the worktree's own git directory.
pub const PENDING: &str = "kluris-sync/pending.json";

/// How many times a push the remote turned back because it moved is pulled
/// again and tried again.
pub const PUSH_ATTEMPTS: usize = 5;

/// The time a fetch or a push may take.
pub const NETWORK_TIME_LIMIT: Duration = Duration::from_secs(60);

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
    /// A file on one side where the other has a directory, or a rename git
    /// could not settle: a `null` side has no file at the path (a directory
    /// or a rename there).
    Structure,
    /// Changed on both sides of a history with several merge bases, where no
    /// single base can say who changed what.
    AmbiguousBase,
}

impl ConflictKind {
    const ALL: [ConflictKind; 6] = [
        ConflictKind::Content,
        ConflictKind::DeleteEdit,
        ConflictKind::Parent,
        ConflictKind::Frontmatter,
        ConflictKind::Structure,
        ConflictKind::AmbiguousBase,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ConflictKind::Content => "content",
            ConflictKind::DeleteEdit => "delete_edit",
            ConflictKind::Parent => "parent",
            ConflictKind::Frontmatter => "frontmatter",
            ConflictKind::Structure => "structure",
            ConflictKind::AmbiguousBase => "ambiguous_base",
        }
    }

    /// The kind [`ConflictKind::as_str`] spells `name`.
    pub fn from_name(name: &str) -> Option<ConflictKind> {
        ConflictKind::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

/// One path a person must decide, with its blob on each side (`None`: not
/// there), as the waiting merge records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub kind: ConflictKind,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

/// One path a person must decide, as a pull reports it: each side's text,
/// `None` when that side has no file or is binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictText {
    pub path: String,
    pub kind: ConflictKind,
    /// The id of the waiting merge it belongs to ([`pending::Pending::id`]).
    pub merge: String,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    /// A side is binary (holds a NUL byte) or not UTF-8.
    pub binary: bool,
}

/// What a person chose for one conflicted path ([`resolve`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedChoice {
    /// Our side's version, a deletion included.
    Ours,
    /// Their side's version, a deletion included.
    Theirs,
    /// The person's own content, stored as this blob.
    Blob(String),
    /// No file at the path: the person deleted it.
    Removed,
}

impl ResolvedChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolvedChoice::Ours => "ours",
            ResolvedChoice::Theirs => "theirs",
            ResolvedChoice::Blob(_) => "file",
            ResolvedChoice::Removed => "removed",
        }
    }
}

/// How to sync.
#[derive(Debug, Clone, Copy)]
pub struct SyncRequest<'a> {
    /// Skip the `pre-merge-commit` and `commit-msg` hooks (`pre-push`
    /// always runs).
    pub no_verify: bool,
    /// For the maps rebuilt inside a merge.
    pub maps: DreamOptions<'a>,
    /// The time each fetch and push may take.
    pub network_time_limit: Duration,
    /// How long to wait for the work tree another command holds
    /// ([`claim::TreeClaim`]).
    pub tree_bound: Duration,
}

/// Why a pull or a push left the brain as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftReason {
    /// Uncommitted files are where the incoming changes would write.
    Uncommitted,
    /// The remote could not be reached, or not in time.
    Offline,
    /// The remote turned the brain away, or a ref is in a state Kluris
    /// will not move.
    Refused,
    /// Another command or git holds the work tree, a lock or the remote.
    Busy,
    /// A git merge, rebase, cherry-pick or revert is in progress.
    InProgress,
    /// A hook stopped the merge.
    Hook,
    /// git stopped partway, or a step failed.
    Failed,
    /// The branch has no upstream on a remote.
    NoUpstream,
}

impl LeftReason {
    pub fn as_str(self) -> &'static str {
        match self {
            LeftReason::Uncommitted => "uncommitted",
            LeftReason::Offline => "offline",
            LeftReason::Refused => "refused",
            LeftReason::Busy => "busy",
            LeftReason::InProgress => "in_progress",
            LeftReason::Hook => "hook",
            LeftReason::Failed => "failed",
            LeftReason::NoUpstream => "no_upstream",
        }
    }

    /// Whether `kluris sync` fails for it: everything but having no network
    /// or no upstream.
    pub fn fails(self) -> bool {
        !matches!(self, LeftReason::Offline | LeftReason::NoUpstream)
    }
}

/// Why the brain was left as it is: the reason, the paths in the way
/// (`uncommitted`), and the detail (git's line and the fix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftAsIs {
    pub reason: LeftReason,
    pub paths: Vec<String>,
    pub detail: Option<String>,
    /// The step's git was stopped at its deadline ([`GitError::stopped`]):
    /// it ran out of time, which says nothing of the brain. Only a git
    /// with a deadline is stopped, and the land's git has none.
    pub stopped: bool,
}

impl LeftAsIs {
    /// Left for `reason`, with `detail`.
    pub fn because(reason: LeftReason, detail: impl Into<String>) -> LeftAsIs {
        LeftAsIs {
            reason,
            paths: Vec::new(),
            detail: Some(detail.into()),
            stopped: false,
        }
    }
}

impl fmt::Display for LeftAsIs {
    /// The paths in the way, the detail, or the reason itself.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.reason, &self.detail) {
            (LeftReason::Uncommitted, _) => write!(
                f,
                "uncommitted changes are in the way: {}",
                self.paths.join(", ")
            ),
            (_, Some(detail)) => f.write_str(detail),
            (reason, None) => f.write_str(reason.as_str()),
        }
    }
}

/// What a pull (and a push) did to one brain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// The commits brought in.
    pub pulled: usize,
    /// Markdown files both sides changed, merged by the rules.
    pub auto_merged: Vec<String>,
    /// Why the brain was left as it is; `None` when nothing was left.
    pub left_as_is: Option<LeftAsIs>,
    /// The waiting merge's conflicts.
    pub conflicts: Vec<ConflictText>,
    /// The upstream's commits newer than the one the conflicts were found
    /// against.
    pub behind_conflict: usize,
    /// The commits the remote does not have yet.
    pub to_push: usize,
    pub pushed: bool,
    /// What the last background push did, as its record says.
    pub last_push: Option<background::LastPush>,
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
