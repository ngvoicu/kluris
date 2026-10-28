//! Git auto-sync (SPEC Phase 7). `commit` commits approved paths with the
//! maps they change; `fresh` fetches a brain's upstream before `wake-up` and
//! `search` read it; [`sync`] fetches, merges and pushes a brain.
//!
//! [`sync`], for one brain at the root of its own git repo:
//! 1. refuses a brain nested in a larger repo, one that is not a work tree,
//!    one with a merge, rebase, cherry-pick or revert in progress, and one
//!    with an open sync conflict ([`PENDING`], for `kluris resolve`);
//! 2. learns the current branch's upstream commit and fetches it, never
//!    prompting, within the network time limit, reading nothing another
//!    git in the brain can rewrite: its own `git ls-remote <remote>
//!    <branch>` says the commit; a fetch of that branch alone (no tags,
//!    other branches, pruning or `refs/remotes/<remote>/HEAD`) brings the
//!    objects and writes nothing else (no ref, no `FETCH_HEAD`), so a fetch
//!    stopped at the limit holds no ref lock (a terminated job on Windows
//!    removes none); `git cat-file -e` then checks the commit is here,
//!    locally (a partial clone never fetches it for the check) and within
//!    the same limit.
//!    Offline (the time limit, or network trouble:
//!    [`plumbing::network_failure`]) or without an upstream nothing
//!    changes; any other failed `ls-remote` or fetch (a key or token
//!    turned away, no such repository) refuses the brain with git's
//!    message and the fix
//!    ([`refused_access`]), and so does a push git could not make for such
//!    a reason. It refuses a brain whose upstream's remote-tracking ref is
//!    outside `refs/remotes/` or a symbolic ref ([`unmovable_tracking`]),
//!    checked before the fetch, after it and right before each push:
//!    moving it would move a branch and leave its files behind. Then,
//!    outside the limit, it moves the remote-tracking ref to what was
//!    fetched itself ([`fetch_tracking`]: `git update-ref --no-deref` from
//!    the value the ref held before the fetch, a compare-and-swap that
//!    replaces a ref made symbolic meanwhile instead of following it), so
//!    it points where a plain `git fetch` leaves it. A concurrent `git
//!    fetch` that moved it meanwhile is kept, and Kluris fetches once more.
//!    An update git stopped on a ref lock refuses the brain, naming the
//!    lock file ([`ref_locked`]), which Kluris never removes. What sync
//!    merges, fast-forwards to and records in a conflict is the commit its
//!    own `ls-remote` gave, never a value read back from the ref or from
//!    `FETCH_HEAD`. When the remote moved its branch away from that commit
//!    before the fetch brought it, sync fetches once more, and refuses the
//!    brain if that happens again;
//! 3. behind only: fast-forwards; ahead only: pushes; diverged: merges in
//!    memory ([`merge`]) and, without conflicts, commits the merge
//!    (`commit-tree`, two parents, `kluris: sync` with `Kluris-Sync: auto`,
//!    after the `pre-merge-commit` hook), fast-forwards to it and pushes;
//!    with conflicts nothing changes and they are recorded in [`PENDING`];
//! 4. never overwrites a file with uncommitted changes: when the incoming
//!    changes touch one (or a new file would land on an untracked or
//!    ignored file), nothing changes and those paths are `blocked`;
//! 5. a push rejected because the remote moved fetches, merges and pushes
//!    again, [`PUSH_ATTEMPTS`] times at most; one stopped at the network
//!    time limit is offline. Once the remote took a push, git moves the
//!    remote-tracking ref itself (nothing turns that off): a push stopped
//!    at that moment on Windows can leave the ref's lock, and the next sync
//!    refuses the brain, naming it, as in step 2.

pub mod commit;
pub mod fresh;
pub mod frontmatter_merge;
pub mod merge;
pub mod plumbing;
pub mod resolve;
pub mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;
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
    /// A file on one side where the other has a directory, or a rename git
    /// could not settle: a `null` side has no file at the path (a directory
    /// or a rename there).
    Structure,
    /// Changed on both sides of a history with several merge bases, where no
    /// single base can say who changed what.
    AmbiguousBase,
}

impl ConflictKind {
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
    /// Skip the `pre-merge-commit` and `commit-msg` hooks (`pre-push`
    /// always runs).
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
    /// No upstream, or the remote could not be reached (network trouble,
    /// or the time limit).
    pub offline: bool,
    /// The uncommitted files the incoming changes would overwrite; when
    /// any, nothing changed.
    pub blocked: Vec<String>,
    /// Why the brain was left alone (an open conflict, a locked ref, a
    /// remote-tracking ref it will not move, a remote that turned it away),
    /// else `None`.
    pub refused: Option<String>,
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
    /// A hook stopped the merge.
    Hook {
        name: &'static str,
        code: i32,
    },
    /// A hook changed the merged index (staged files over the merge).
    HookChangedMerge {
        name: &'static str,
    },
    /// The merge landed locally but the push was refused.
    Push(String),
    /// Every push was rejected because the remote kept moving.
    RemoteKeptMoving,
    /// The open conflict could not be recorded, or the merge message file
    /// could not be written or read.
    Pending(String),
    /// The message hooks left the merge message empty.
    EmptyMessage,
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
            SyncError::Hook { name, code } => write!(
                f,
                "the {name} hook failed (exit {code}); nothing was merged"
            ),
            SyncError::HookChangedMerge { name } => write!(
                f,
                "the {name} hook changed the merge; nothing was merged (a merge hook may change the message, not the files)"
            ),
            SyncError::Push(detail) => {
                write!(f, "merged locally, but the push was refused: {detail}")
            }
            SyncError::RemoteKeptMoving => write!(
                f,
                "the remote kept moving: {PUSH_ATTEMPTS} pushes were rejected; run kluris sync again"
            ),
            SyncError::Pending(e) => write!(f, "cannot record the open conflict: {e}"),
            SyncError::EmptyMessage => {
                f.write_str("the message hooks left the merge message empty; nothing was merged")
            }
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
            SyncError::NotARepo | SyncError::Nested { .. } | SyncError::InProgress(_)
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

fn opt(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

/// One conflict as JSON (`pending.json`, `sync --json`, `status --json`).
pub fn conflict_json(c: &Conflict) -> Json {
    Json::Object(vec![
        ("path".into(), Json::Str(c.path.clone())),
        ("kind".into(), Json::Str(c.kind.as_str().into())),
        ("base".into(), opt(&c.base)),
        ("ours".into(), opt(&c.ours)),
        ("theirs".into(), opt(&c.theirs)),
    ])
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
}

impl ResolvedChoice {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolvedChoice::Ours => "ours",
            ResolvedChoice::Theirs => "theirs",
            ResolvedChoice::Blob(_) => "file",
        }
    }
}

/// An open sync conflict, as [`PENDING`] records it: the two commits, their
/// merge base, the conflicts, when it was opened, and the choices made so
/// far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub base: Option<String>,
    pub ours: String,
    pub theirs: String,
    pub conflicts: Vec<Conflict>,
    /// RFC 3339.
    pub created: String,
    /// In the order they were made; one per path.
    pub resolved: Vec<(String, ResolvedChoice)>,
}

fn str_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn opt_field(value: &serde_json::Value, key: &str) -> Option<Option<String>> {
    match value.get(key)? {
        serde_json::Value::Null => Some(None),
        serde_json::Value::String(s) => Some(Some(s.clone())),
        _ => None,
    }
}

impl Pending {
    /// A new conflict between `ours` and `theirs`, opened now.
    pub fn new(base: Option<String>, ours: &str, theirs: &str, conflicts: Vec<Conflict>) -> Self {
        Pending {
            base,
            ours: ours.to_string(),
            theirs: theirs.to_string(),
            conflicts,
            created: jiff::Timestamp::now().to_string(),
            resolved: Vec::new(),
        }
    }

    /// `{base, ours, theirs, conflicts, created}`, then `resolved` once a
    /// choice was made (`[{path, choice, blob}]`).
    pub fn to_json(&self) -> Json {
        let mut doc = vec![
            ("base".into(), opt(&self.base)),
            ("ours".into(), Json::Str(self.ours.clone())),
            ("theirs".into(), Json::Str(self.theirs.clone())),
            (
                "conflicts".into(),
                Json::Array(self.conflicts.iter().map(conflict_json).collect()),
            ),
            ("created".into(), Json::Str(self.created.clone())),
        ];
        if !self.resolved.is_empty() {
            let items = self
                .resolved
                .iter()
                .map(|(path, choice)| {
                    let blob = match choice {
                        ResolvedChoice::Blob(oid) => Json::Str(oid.clone()),
                        _ => Json::Null,
                    };
                    Json::Object(vec![
                        ("path".into(), Json::Str(path.clone())),
                        ("choice".into(), Json::Str(choice.as_str().into())),
                        ("blob".into(), blob),
                    ])
                })
                .collect();
            doc.push(("resolved".into(), Json::Array(items)));
        }
        Json::Object(doc)
    }

    /// The pending state from its JSON; `None` when it is not one.
    pub fn parse(text: &str) -> Option<Pending> {
        let doc: serde_json::Value = serde_json::from_str(text).ok()?;
        let kind = |k: &str| match k {
            "content" => Some(ConflictKind::Content),
            "delete_edit" => Some(ConflictKind::DeleteEdit),
            "parent" => Some(ConflictKind::Parent),
            "frontmatter" => Some(ConflictKind::Frontmatter),
            "structure" => Some(ConflictKind::Structure),
            "ambiguous_base" => Some(ConflictKind::AmbiguousBase),
            _ => None,
        };
        let mut conflicts = Vec::new();
        for c in doc.get("conflicts")?.as_array()? {
            conflicts.push(Conflict {
                path: str_field(c, "path")?,
                kind: kind(c.get("kind")?.as_str()?)?,
                base: opt_field(c, "base")?,
                ours: opt_field(c, "ours")?,
                theirs: opt_field(c, "theirs")?,
            });
        }
        let mut resolved = Vec::new();
        if let Some(items) = doc.get("resolved").and_then(|r| r.as_array()) {
            for item in items {
                let choice = match item.get("choice")?.as_str()? {
                    "ours" => ResolvedChoice::Ours,
                    "theirs" => ResolvedChoice::Theirs,
                    "file" => ResolvedChoice::Blob(str_field(item, "blob")?),
                    _ => return None,
                };
                resolved.push((str_field(item, "path")?, choice));
            }
        }
        Some(Pending {
            base: opt_field(&doc, "base")?,
            ours: str_field(&doc, "ours")?,
            theirs: str_field(&doc, "theirs")?,
            conflicts,
            created: str_field(&doc, "created").unwrap_or_default(),
            resolved,
        })
    }

    /// Write it to `path` atomically.
    pub fn write(&self, path: &Path) -> Result<(), SyncError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| SyncError::Pending(e.to_string()))?;
        }
        let mut text = to_string(&self.to_json());
        text.push('\n');
        atomic::write_atomic(path, text.as_bytes()).map_err(|e| SyncError::Pending(e.to_string()))
    }
}

/// The uncommitted files (in `dirty`) that moving from `ours` to `target`
/// would overwrite, and the untracked or ignored files a new file would
/// land on.
pub(crate) fn blocked_paths<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
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

/// Why sync leaves a brain with an open conflict alone (its entry also
/// lists the conflicts).
pub const OPEN_CONFLICT: &str =
    "a conflict is open: settle it with kluris resolve (or kluris resolve --abort)";

/// Run a hook that can stop the merge, against `index`.
fn gate<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    index: &crate::git::StagingIndex,
    name: &'static str,
    args: &[&str],
) -> Result<(), SyncError> {
    match git.run_hook(repo, Some(index), name, args)? {
        0 => Ok(()),
        code => Err(SyncError::Hook { name, code }),
    }
}

/// The merge commit of the tree in `index` on `ours` and `theirs`, with
/// the hooks git merge runs: `pre-merge-commit` (unless `no_verify`), then
/// `prepare-commit-msg` (with `merge`) and `commit-msg` (unless
/// `no_verify`) on a message file, against the merged index; the message is
/// what they leave there, cleaned up as `git commit` does. A failing hook
/// stops it with nothing committed, and so does a hook that changes the
/// merged index (`merged_tree`): the hooks see the old work tree, so an
/// ordinary `git add -A` would stage it over the incoming files (decision
/// of T-152). A message hook may change the message, never the files.
pub(crate) fn commit_merge<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    index: &crate::git::StagingIndex,
    merged_tree: &str,
    parents: [&str; 2],
    message: &str,
    no_verify: bool,
) -> Result<String, SyncError> {
    let unchanged = |name: &'static str| -> Result<(), SyncError> {
        if git.write_tree(repo, index)? == merged_tree {
            Ok(())
        } else {
            Err(SyncError::HookChangedMerge { name })
        }
    };
    if !no_verify {
        gate(git, repo, index, "pre-merge-commit", &[])?;
        unchanged("pre-merge-commit")?;
    }
    let file = git.git_dir_path(repo, &format!("KLURIS_MERGE_MSG-{}", std::process::id()))?;
    fs::write(&file, message).map_err(|e| SyncError::Pending(e.to_string()))?;
    let made = (|| {
        // Hooks are shell scripts (Git for Windows' sh too): `/` separators.
        let arg = file.to_string_lossy().replace('\\', "/");
        gate(git, repo, index, "prepare-commit-msg", &[&arg, "merge"])?;
        unchanged("prepare-commit-msg")?;
        if !no_verify {
            gate(git, repo, index, "commit-msg", &[&arg])?;
            unchanged("commit-msg")?;
        }
        let text = fs::read(&file).map_err(|e| SyncError::Pending(e.to_string()))?;
        let message = commit::cleanup_message(&String::from_utf8_lossy(&text));
        if message.is_empty() {
            return Err(SyncError::EmptyMessage);
        }
        fs::write(&file, &message).map_err(|e| SyncError::Pending(e.to_string()))?;
        Ok(git.commit_tree(repo, merged_tree, &parents, &file)?)
    })();
    let _ = fs::remove_file(&file);
    made
}

/// Move the branch to `to`, a descendant of `ours`, unless an uncommitted,
/// untracked or ignored file sits where it writes: checked right before the
/// fast-forward (a hook or an editor may have written one since the first
/// check) and again when git refuses (one written in between); the
/// fast-forward itself never overwrites an ignored file nor stashes. The
/// files in the way; none when the branch moved.
pub(crate) fn land<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    root: &Path,
    ours: &str,
    to: &str,
) -> Result<Vec<String>, SyncError> {
    let in_the_way = || -> Result<Vec<String>, SyncError> {
        let dirty = git.dirty(repo)?;
        Ok(blocked_paths(git, repo, root, &dirty, ours, to)?)
    };
    let blocked = in_the_way()?;
    if !blocked.is_empty() {
        return Ok(blocked);
    }
    match git.fast_forward_to(repo, to) {
        Ok(()) => Ok(Vec::new()),
        Err(e) => match in_the_way()? {
            blocked if blocked.is_empty() => Err(e.into()),
            blocked => Ok(blocked),
        },
    }
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
        // Reported in the brain's entry, so an agent sees the conflicts and
        // settles them (decision of T-146).
        let conflicts = fs::read_to_string(&pending)
            .ok()
            .and_then(|text| Pending::parse(&text))
            .map(|p| p.conflicts)
            .unwrap_or_default();
        return Ok(SyncReport {
            conflicts,
            refused: Some(OPEN_CONFLICT.to_string()),
            ..SyncReport::default()
        });
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

    for _ in 0..PUSH_ATTEMPTS {
        let theirs = match fetch_tracking(git, &repo, &tracking, request)? {
            Fetched::Commit(theirs) => theirs,
            Fetched::Offline => {
                report.offline = true;
                return Ok(report);
            }
            Fetched::Refused(why) => {
                report.refused = Some(why);
                return Ok(report);
            }
        };
        if let Ok(path) = git.git_dir_path(&repo, fresh::BACKOFF) {
            fresh::end_backoff(&path);
        }
        report.fetched = true;
        let ours = git
            .head(&repo)?
            .ok_or_else(|| SyncError::Git(no_commit_yet()))?;
        let has = |a: &str, b: &str| git.is_ancestor(&repo.dir, a, b, far_future());
        let behind = !has(&theirs, &ours).map_err(fresh_error)?;
        let ahead = !has(&ours, &theirs).map_err(fresh_error)?;

        if behind {
            // The tree the branch will hold: theirs, or the merge.
            let (target, merge) = if ahead {
                let none = BTreeMap::new();
                match merge_commits(git, &repo, &ours, &theirs, &request.maps, &none)? {
                    MergeOutcome::Conflicts { base, conflicts } => {
                        Pending::new(base, &ours, &theirs, conflicts.clone()).write(&pending)?;
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
            let blocked = blocked_paths(git, &repo, root, &report.skipped_dirty, &ours, &target)?;
            if !blocked.is_empty() {
                report.blocked = blocked;
                return Ok(report);
            }
            let to = match merge {
                None => theirs.clone(),
                Some((index, auto_merged)) => {
                    let commit = commit_merge(
                        git,
                        &repo,
                        &index,
                        &target,
                        [&ours, &theirs],
                        MESSAGE,
                        request.no_verify,
                    );
                    for path in auto_merged {
                        if !report.auto_merged.contains(&path) {
                            report.auto_merged.push(path);
                        }
                    }
                    commit?
                }
            };
            let blocked = land(git, &repo, root, &ours, &to)?;
            if !blocked.is_empty() {
                report.skipped_dirty = git.dirty(&repo)?;
                report.blocked = blocked;
                return Ok(report);
            }
            report.merged = true;
            if !ahead {
                return Ok(report);
            }
        } else if !ahead {
            return Ok(report);
        }

        // Once the remote takes the push, git moves the tracking ref itself,
        // following a symbolic ref: checked again right before.
        if let Some(why) = unmovable_tracking(git, &repo, &tracking)? {
            report.refused = Some(why);
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
            PushOutcome::Refused(stderr) => {
                report.refused = Some(refused_access("push", &tracking.remote, &stderr));
                return Ok(report);
            }
            PushOutcome::Failed(detail) => return Err(SyncError::Push(detail)),
        }
    }
    Err(SyncError::RemoteKeptMoving)
}

/// What sync goes on from after it fetched the upstream's branch.
enum Fetched {
    /// The commit Kluris's own fetch brought: what sync merges, fast-forwards
    /// to and records in a conflict. When the remote has no such branch,
    /// what the tracking ref held before the fetch.
    Commit(String),
    /// The fetch was stopped, or the network is in the way.
    Offline,
    /// Why the brain is left alone.
    Refused(String),
}

/// Fetch the upstream's branch within the network time limit, then move
/// its remote-tracking ref there outside it, only while the ref still holds
/// what it held before the fetch. Sync goes on from the commit its own
/// `git ls-remote` gave, once its fetch has brought it
/// ([`plumbing::SyncGit::fetch`]), never from a value read back from the
/// ref or from `FETCH_HEAD`. When the remote moved its branch away from
/// that commit before the fetch brought it, Kluris fetches once more, and
/// refuses the brain if that happens again ([`remote_moved_away`]).
///
/// The ref is checked ([`unmovable_tracking`]) before the fetch, again
/// after it whatever it brought, and again after a compare-and-swap that
/// failed: one that turned into a symbolic ref or left `refs/remotes/`
/// meanwhile refuses the brain before anything is merged. A compare-and-swap
/// lost to a concurrent `git fetch` keeps that fetch's value; Kluris then
/// fetches once more and goes on from that second fetch's commit, whether
/// its compare-and-swap wins or loses too. Offline when a fetch is stopped
/// or the network is in the way; refused when the remote turned the fetch
/// away ([`fetch_refused`]) or a lock stops the move ([`ref_locked`]).
fn fetch_tracking<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    tracking: &plumbing::Tracking,
    request: &SyncRequest,
) -> Result<Fetched, SyncError> {
    let (mut lost_once, mut gone_once) = (false, false);
    loop {
        if let Some(why) = unmovable_tracking(git, repo, tracking)? {
            return Ok(Fetched::Refused(why));
        }
        let before = git.ref_value(repo, &tracking.refname)?;
        let deadline = Instant::now() + request.network_time_limit;
        let fetched = match git.fetch(repo, tracking, deadline) {
            Ok(fetched) => fetched,
            Err(e) => {
                return Ok(match fetch_refused(git, repo, &tracking.remote, &e) {
                    Some(why) => Fetched::Refused(why),
                    None => Fetched::Offline,
                });
            }
        };
        if let Some(why) = unmovable_tracking(git, repo, tracking)? {
            return Ok(Fetched::Refused(why));
        }
        let fetched = match fetched {
            plumbing::Fetch::Commit(fetched) => fetched,
            plumbing::Fetch::NoBranch => {
                return match before {
                    Some(held) => Ok(Fetched::Commit(held)),
                    None => Err(SyncError::Git(no_remote_branch(tracking))),
                };
            }
            // The remote moved its branch away from the commit `ls-remote`
            // gave before the fetch brought it: fetch again.
            plumbing::Fetch::Gone if !gone_once => {
                gone_once = true;
                continue;
            }
            plumbing::Fetch::Gone => {
                return Ok(Fetched::Refused(remote_moved_away(tracking)));
            }
        };
        if before.as_ref() == Some(&fetched) {
            return Ok(Fetched::Commit(fetched));
        }
        let message = format!("kluris sync: fetch {}", tracking.remote);
        let Err(e) = git.move_ref(
            repo,
            &tracking.refname,
            &fetched,
            before.as_deref(),
            &message,
        ) else {
            return Ok(Fetched::Commit(fetched));
        };
        if let Some(why) = unmovable_tracking(git, repo, tracking)? {
            return Ok(Fetched::Refused(why));
        }
        if git.ref_value(repo, &tracking.refname)? == before {
            return match locked_ref(git, repo, &e.detail) {
                Some(lock) => Ok(Fetched::Refused(ref_locked(&lock))),
                None => Err(e.into()),
            };
        }
        // Another git moved the ref, a concurrent `git fetch`: its value is
        // kept, never merged.
        if lost_once {
            return Ok(Fetched::Commit(fetched));
        }
        lost_once = true;
    }
}

/// Why sync leaves a brain whose upstream's remote-tracking ref it will not
/// move, or `None`: a ref outside `refs/remotes/` (a fetch refspec that
/// maps the branch onto a local branch, the checked-out one perhaps) or a
/// symbolic ref would carry the move to a branch, while its index and
/// files stay where they were. A tracking ref that does not exist yet is
/// fine: the move creates it.
fn unmovable_tracking<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    tracking: &plumbing::Tracking,
) -> Result<Option<String>, GitError> {
    let (refname, remote) = (&tracking.refname, &tracking.remote);
    if !refname.starts_with("refs/remotes/") {
        return Ok(Some(format!(
            "the upstream's remote-tracking ref is {refname}, outside refs/remotes/: moving it would move that ref itself, so Kluris leaves it alone; map the branch into refs/remotes/{remote}/ with `git config --replace-all remote.{remote}.fetch '+refs/heads/*:refs/remotes/{remote}/*'` and run kluris sync again"
        )));
    }
    Ok(git.symbolic_target(repo, refname)?.map(|target| {
        format!(
            "the upstream's remote-tracking ref {refname} is a symbolic ref to {target}: moving it would move {target}, so Kluris leaves it alone; remove it with `git symbolic-ref --delete {refname}` and run kluris sync again"
        )
    }))
}

/// Why sync leaves a brain whose remote moved its branch away from the
/// commit `ls-remote` gave, before the fetch brought it, on each try.
fn remote_moved_away(tracking: &plumbing::Tracking) -> String {
    let branch = tracking
        .remote_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&tracking.remote_ref);
    format!(
        "{}'s {branch} moved while Kluris fetched it, on each try; nothing was merged or pushed: run kluris sync again",
        tracking.remote
    )
}

/// Why sync leaves a brain whose fetch found a ref locked alone: `lock`,
/// and how to clear it.
pub fn ref_locked(lock: &str) -> String {
    format!(
        "a ref is locked: {lock} exists, so git cannot update the brain's refs; if no git is running in this brain, remove that file and run kluris sync again"
    )
}

/// Why sync leaves a brain whose remote turned it away (a key or token
/// that does not work, no access, no such repository: anything but network
/// trouble): git's message on one line, then the fix. `command` is `fetch`
/// or `push`.
pub fn refused_access(command: &str, remote: &str, stderr: &str) -> String {
    let mut message = plumbing::one_line(stderr);
    if message.is_empty() {
        message = format!("git {command} {remote} failed");
    }
    format!(
        "{message}; check that this machine can run `git {command} {remote}` in this brain without a prompt"
    )
}

/// Why a failed fetch leaves the brain alone, or `None` when it is offline:
/// the time limit, or network trouble ([`plumbing::network_failure`]).
/// A ref lock is named ([`ref_locked`]); anything else is a refusal
/// ([`refused_access`]), shown, never taken for offline.
fn fetch_refused<G: SyncGit>(
    git: &G,
    repo: &crate::git::Repo,
    remote: &str,
    error: &fresh::FreshError,
) -> Option<String> {
    let fresh::FreshError::Failed(e) = error else {
        return None;
    };
    if let Some(lock) = locked_ref(git, repo, &e.detail) {
        return Some(ref_locked(&lock));
    }
    if plumbing::network_failure(&e.detail) {
        return None;
    }
    Some(refused_access("fetch", remote, &e.detail))
}

/// The lock file a failed ref update was stopped by, if that is why it
/// failed (git's stderr in `detail`): the one git named, or the named ref's
/// `<ref>.lock` when it exists.
fn locked_ref<G: SyncGit>(git: &G, repo: &crate::git::Repo, detail: &str) -> Option<String> {
    match plumbing::ref_lock(detail)? {
        plumbing::RefLock::File(path) => Some(path),
        plumbing::RefLock::Ref(name) => {
            let path = git.git_dir_path(repo, &format!("{name}.lock")).ok()?;
            fs::symlink_metadata(&path)
                .is_ok()
                .then(|| path.display().to_string())
        }
    }
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

/// The remote has no such branch and there is no tracking ref to go on
/// from.
fn no_remote_branch(tracking: &plumbing::Tracking) -> GitError {
    GitError {
        args: vec![
            "fetch".into(),
            tracking.remote.clone(),
            tracking.remote_ref.clone(),
        ],
        code: None,
        detail: format!(
            "the remote has no branch {} and there is no {} to sync from",
            tracking.remote_ref, tracking.refname
        ),
    }
}

fn no_commit_yet() -> GitError {
    GitError {
        args: vec!["rev-parse".into(), "HEAD".into()],
        code: None,
        detail: "the branch has no commit yet: commit first".into(),
    }
}
