//! `kluris commit` (SPEC Phase 7, item 44): commit the files a person
//! approved, plus the `map.md`/`brain.md` files the map step regenerates
//! because of them, and nothing else.
//!
//! The model is a [`CommitSet`]: the paths, the base revision it is built
//! on, the author and the approver (and the agent that co-authored it).
//! The commit is built with git plumbing in a temporary index, so the user's
//! index and every other dirty file stay as they were, and lands only if
//! `HEAD` is still at the base revision (compare-and-swap), which `sync`
//! relies on when it runs beside a commit.
//!
//! The maps are built from what is committed, never from the work tree: the
//! map step reads a [`TreeStore`] holding the base revision's files with the
//! given paths taken from the work tree, so a file that is uncommitted,
//! untracked or ignored never reaches a committed map (not even its title).
//! Which generated files the paths change ([`affected_generated`]): the map
//! of each path's directory; and, for every directory on the way up that the
//! paths create or remove (its `map.md` is new, or is in the base but the
//! directory is gone from the work tree), its parent's map (`brain.md` at the
//! top) and its siblings' maps (their `sideways` links). Only those are
//! committed. A map is known by the path in the tree that holds its bytes: a
//! `map.md` that is a symlink in the brain (`notes/map.md -> ../maps/x.md`)
//! is regenerated, staged and reported as its target (`maps/x.md`), and the
//! link stays as it is. Afterwards a regenerated map's work-tree copy gets
//! the committed bytes (its line endings kept) when it had no change of its
//! own, as a checkout would: its bytes and file identity are taken before
//! its cleanliness is checked and compared again right before it is
//! replaced, so a map edited in between (by a hook, an editor) is left as it
//! is, as is one with changes of its own; both are then in `left_dirty`.
//!
//! Hooks run as `git commit` runs them (`git hook run`, so `core.hooksPath`
//! counts), against the temporary index: `pre-commit` before the tree is
//! written, `prepare-commit-msg` and `commit-msg` on a message file (the
//! message is what they leave there, cleaned up as `git commit -m` does),
//! `post-commit` once `HEAD` has moved. A failing `pre-commit`,
//! `prepare-commit-msg` or `commit-msg` stops the commit with nothing
//! changed; `no_verify` skips `pre-commit` and `commit-msg`. A hook may
//! change what the commit takes for its paths (a formatter does), and that
//! is committed; a tree that changes any other path, in the brain or
//! elsewhere in its repo, stops the commit with nothing changed
//! ([`CommitError::Unapproved`]). That needs git
//! >= 2.36 (`git hook run`); `sync` needs 2.38, which `commit` checks first.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::atomic;
use crate::brain::sandbox;
use crate::brain::store::BrainStore;
use crate::brain::tree_store::{Node, TreeStore};
use crate::dream::{DreamOptions, map_changes};
use crate::frontmatter::{LineEnding, line_ending};
use crate::git::{
    GitError, Identity, InProgress, MIN_SYNC_VERSION, Plumbing, Repo, StagedBlob, StagingIndex,
    parse_version,
};

/// The longest default subject, in characters.
pub const SUBJECT_MAX: usize = 72;

const SUBJECT_PREFIX: &str = "kluris: update ";

/// Why a path given to `commit` is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    /// It names no file (`""`, `.`, `notes/..`).
    Empty,
    /// It is absolute (paths are relative to the brain).
    Absolute,
    /// Its `..` parts leave the brain.
    Outside,
    /// It is inside git's own directory.
    GitDir,
    /// A symlink on its way leads outside the brain (or loops).
    LinkOutside,
    /// It is a directory, not a file.
    Directory,
    /// It is neither in the work tree nor in the base revision.
    Missing,
}

impl fmt::Display for PathProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathProblem::Empty => "names no file",
            PathProblem::Absolute => "is absolute (paths are relative to the brain)",
            PathProblem::Outside => "is outside the brain",
            PathProblem::GitDir => "is inside git's own directory",
            PathProblem::LinkOutside => "is a symlink, or under one, that leads outside the brain",
            PathProblem::Directory => "is a directory, not a file",
            PathProblem::Missing => "does not exist and is not in the last commit",
        })
    }
}

/// A path as given, made brain-relative and `/`-separated: `.` and empty
/// parts dropped, `..` applied, which must not leave the brain. On Windows
/// `\\` separates too, and a `:` (a drive, a stream) makes it absolute.
pub fn normalize_path(raw: &str) -> Result<String, PathProblem> {
    let text = if cfg!(windows) {
        raw.replace('\\', "/")
    } else {
        raw.to_string()
    };
    if text.starts_with('/')
        || Path::new(raw).is_absolute()
        || (cfg!(windows) && text.contains(':'))
    {
        return Err(PathProblem::Absolute);
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or(PathProblem::Outside)?;
            }
            name => parts.push(name),
        }
    }
    if parts.is_empty() {
        return Err(PathProblem::Empty);
    }
    if parts.iter().any(|part| part.eq_ignore_ascii_case(".git")) {
        return Err(PathProblem::GitDir);
    }
    Ok(parts.join("/"))
}

/// Check a normalized path against the work tree: inside the brain once
/// symlinks are followed, a file (or a symlink) or a file the base revision
/// has that is gone now.
fn check_on_disk(root: &Path, rel: &str, in_base: bool) -> Result<(), PathProblem> {
    let path = rel
        .split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part));
    let resolved_root = sandbox::resolve(root).map_err(|_| PathProblem::LinkOutside)?;
    let resolved = sandbox::resolve(&path).map_err(|_| PathProblem::LinkOutside)?;
    if !sandbox::is_within(&resolved, &resolved_root) {
        return Err(PathProblem::LinkOutside);
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => Err(PathProblem::Directory),
        Ok(_) => Ok(()),
        Err(_) if in_base => Ok(()),
        Err(_) => Err(PathProblem::Missing),
    }
}

fn parent_dir(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn map_of(dir: &str) -> String {
    format!("{dir}/map.md")
}

/// The maps of every directory above each path, and `brain.md`: the
/// generated files whose presence in the base revision
/// [`affected_generated`] asks about.
pub fn generated_candidates(paths: &[String]) -> Vec<String> {
    let mut out = BTreeSet::from(["brain.md".to_string()]);
    for path in paths {
        let mut dir = parent_dir(path);
        while !dir.is_empty() {
            out.insert(map_of(dir));
            dir = parent_dir(dir);
        }
    }
    out.into_iter().collect()
}

/// The generated files the paths change (see the module docs). `dirs` are
/// the directories the walker sees now (each gets a map), `in_base` the
/// generated files of the base revision, of [`generated_candidates`] at
/// least.
pub fn affected_generated(
    paths: &[String],
    dirs: &[String],
    in_base: &BTreeSet<String>,
) -> BTreeSet<String> {
    let now: BTreeSet<&str> = dirs.iter().map(String::as_str).collect();
    let mut out = BTreeSet::new();
    for path in paths {
        let mut dir = parent_dir(path);
        if dir.is_empty() {
            // A file at the root is in no map.
            continue;
        }
        if now.contains(dir) || in_base.contains(&map_of(dir)) {
            out.insert(map_of(dir));
        }
        if base_name(path) == "map.md" && !dir.contains('/') {
            // brain.md shows a lobe map's description.
            out.insert("brain.md".to_string());
        }
        loop {
            let was = in_base.contains(&map_of(dir));
            if now.contains(dir) == was {
                break;
            }
            // `dir` appeared or disappeared: its parent lists it, its
            // siblings link to it.
            out.insert(map_of(dir));
            let up = parent_dir(dir);
            out.insert(if up.is_empty() {
                "brain.md".to_string()
            } else {
                map_of(up)
            });
            for sibling in dirs {
                if sibling != dir && parent_dir(sibling) == up {
                    out.insert(map_of(sibling));
                }
            }
            if up.is_empty() {
                break;
            }
            dir = up;
        }
    }
    out
}

/// The default message: `kluris: update <paths>`, shortened to
/// [`SUBJECT_MAX`] characters when long (`<first paths> and N more`, the
/// file name of a single path, or `N files`), and then followed by every
/// path, one per line.
pub fn default_message(paths: &[String]) -> String {
    let fits = |subject: &String| subject.chars().count() <= SUBJECT_MAX;
    let all = format!("{SUBJECT_PREFIX}{}", paths.join(", "));
    if fits(&all) {
        return all;
    }
    let n = paths.len();
    let subject = match paths {
        [one] => format!("{SUBJECT_PREFIX}{}", base_name(one)),
        _ => (1..n)
            .rev()
            .map(|k| {
                format!(
                    "{SUBJECT_PREFIX}{} and {} more",
                    paths[..k].join(", "),
                    n - k
                )
            })
            .find(fits)
            .unwrap_or_else(|| format!("{SUBJECT_PREFIX}{n} files")),
    };
    format!("{subject}\n\n{}", paths.join("\n"))
}

/// A trailer value on one line: line breaks become spaces.
fn one_line(value: &str) -> String {
    value
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One commit of approved changes: what `commit` makes, and what `sync`
/// and `resolve` will make for a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSet {
    /// Every brain-relative path the commit takes from the work tree: the
    /// given ones, then the generated files they changed.
    pub paths: Vec<String>,
    /// The revision the commit is built on, which `HEAD` must still name
    /// when it lands; `None` on a branch with no commit yet.
    pub base: Option<String>,
    pub author: Identity,
    /// Who approved the change (`Kluris-Approved-By`).
    pub approver: String,
    /// The agent that wrote it (`Co-Authored-By`), from `KLURIS_AGENT`.
    pub co_author: Option<String>,
    /// The message without its trailers.
    pub message: String,
}

impl CommitSet {
    /// The first line of the message.
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }

    /// The message, then a blank line and the trailers.
    pub fn full_message(&self) -> String {
        let mut out = format!(
            "{}\n\nKluris-Approved-By: {}\n",
            self.message.trim_end(),
            one_line(&self.approver)
        );
        if let Some(agent) = &self.co_author {
            out.push_str(&format!("Co-Authored-By: {}\n", one_line(agent)));
        }
        out
    }
}

/// A commit message cleaned up as `git commit -m` cleans one up (its
/// `whitespace` mode): trailing whitespace dropped from every line, runs of
/// blank lines made one, leading and trailing blank lines dropped, one final
/// newline. `#` lines stay. Empty when nothing is left.
pub fn cleanup_message(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blank = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if blank {
            out.push("");
            blank = false;
        }
        out.push(line);
    }
    if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    }
}

/// What to commit.
#[derive(Debug, Clone, Copy)]
pub struct CommitRequest<'a> {
    /// Brain-relative paths as given (files written or deleted).
    pub paths: &'a [String],
    /// The message; `None` or blank for [`default_message`].
    pub message: Option<&'a str>,
    /// `KLURIS_AGENT`: the agent named in `Co-Authored-By`, when not blank.
    pub agent: Option<&'a str>,
    /// `--no-verify`: skip the `pre-commit` and `commit-msg` hooks, as
    /// `git commit --no-verify` does.
    pub no_verify: bool,
    /// For the map step (`brain.md`'s name and description, today's date).
    pub maps: DreamOptions<'a>,
}

/// What `commit` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReport {
    /// The new commit; `None` when there was nothing to commit.
    pub commit: Option<String>,
    /// The given paths, normalized, in order, without repeats.
    pub paths: Vec<String>,
    /// The generated files committed because of them, sorted.
    pub regenerated: Vec<String>,
    /// The brain's files still changed or untracked after the commit,
    /// sorted.
    pub left_dirty: Vec<String>,
    /// The commit as it was made.
    pub set: Option<CommitSet>,
    /// Its message as the hooks left it.
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// git is older than [`MIN_SYNC_VERSION`] (`found`: `git --version`).
    GitTooOld {
        found: String,
    },
    /// The brain is not in a git work tree.
    NotARepo,
    /// The work tree is in the middle of a merge, rebase, ...
    InProgress(InProgress),
    /// No path was given.
    NoPaths,
    /// A path is refused (`path` as given).
    Path {
        path: String,
        problem: PathProblem,
    },
    /// A map the paths change could not be regenerated.
    Regenerate {
        rel: String,
        reason: String,
    },
    /// A hook that can stop a commit failed with this exit code.
    Hook {
        name: &'static str,
        code: i32,
    },
    /// The tree the hooks left changes paths that are neither given nor
    /// regenerated for them (brain-relative; `../` for one outside a nested
    /// brain); nothing was committed.
    Unapproved {
        paths: Vec<String>,
    },
    /// The hooks left the message empty.
    EmptyMessage,
    /// The message file in the git directory could not be written or read.
    MessageFile(String),
    /// `HEAD` moved while the commit was built; nothing was committed.
    HeadMoved(GitError),
    /// The commit was made, but the user's index could not be updated for
    /// its paths.
    IndexNotUpdated {
        commit: String,
        error: GitError,
    },
    Git(GitError),
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitError::GitTooOld { found } => {
                let (major, minor, _) = MIN_SYNC_VERSION;
                write!(
                    f,
                    "kluris commit needs git {major}.{minor} or newer; this is {found}"
                )
            }
            CommitError::NotARepo => f.write_str("it is not a git repository"),
            CommitError::InProgress(op) => write!(
                f,
                "it is in the middle of a {op}; finish or abort it first (nothing was committed)"
            ),
            CommitError::NoPaths => f.write_str("no paths given (--paths a.md,b.md)"),
            CommitError::Path { path, problem } => {
                write!(f, "{path} {problem}; nothing was committed")
            }
            CommitError::Regenerate { rel, reason } => write!(
                f,
                "cannot regenerate {rel}: {reason}; nothing was committed"
            ),
            CommitError::Hook { name, code } => write!(
                f,
                "the {name} hook failed (exit {code}); nothing was committed"
            ),
            CommitError::Unapproved { paths } => write!(
                f,
                "a hook staged files you didn't approve: {}; nothing was committed",
                paths.join(", ")
            ),
            CommitError::EmptyMessage => {
                f.write_str("the commit message is empty; nothing was committed")
            }
            CommitError::MessageFile(e) => write!(
                f,
                "cannot write the commit message file: {e}; nothing was committed"
            ),
            CommitError::HeadMoved(e) => write!(
                f,
                "HEAD moved while the commit was made; nothing was committed ({e})"
            ),
            CommitError::IndexNotUpdated { commit, error } => write!(
                f,
                "committed {commit}, but the index was not updated for its paths ({error})"
            ),
            CommitError::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CommitError {}

impl From<GitError> for CommitError {
    fn from(e: GitError) -> Self {
        CommitError::Git(e)
    }
}

/// Whether a map reads a file's contents: markdown and YAML files.
fn is_text_suffix(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    [".md", ".yml", ".yaml"].iter().any(|s| lower.ends_with(s))
}

/// A tree's files as the view holds them, and each file's git mode.
type Files = (BTreeMap<String, Node>, BTreeMap<String, String>);

/// The files of `rev` under the brain, as the view holds them, and their
/// modes.
fn base_nodes<G: Plumbing>(git: &G, repo: &Repo, rev: &str) -> Result<Files, GitError> {
    let entries = git.tree_entries(repo, rev)?;
    let wanted: Vec<String> = entries
        .iter()
        .filter(|e| e.is_symlink() || is_text_suffix(&e.rel))
        .map(|e| e.oid.clone())
        .collect();
    let blobs = git.read_blobs(repo, &wanted)?;
    let mut nodes = BTreeMap::new();
    let mut modes = BTreeMap::new();
    for entry in entries {
        let bytes = blobs.get(&entry.oid).cloned();
        let node = if entry.is_symlink() {
            Node::Link(String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned())
        } else {
            Node::File(bytes)
        };
        modes.insert(entry.rel.clone(), entry.mode);
        nodes.insert(entry.rel, node);
    }
    Ok((nodes, modes))
}

fn join(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// A given path as the work tree has it, into the view: a file, a symlink,
/// or (gone) nothing.
fn overlay_from_disk(root: &Path, rel: &str, nodes: &mut BTreeMap<String, Node>) {
    let path = join(root, rel);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            let target = fs::read_link(&path)
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            let target = if cfg!(windows) {
                target.replace('\\', "/")
            } else {
                target
            };
            nodes.insert(rel.to_string(), Node::Link(target));
        }
        Ok(_) => {
            nodes.insert(rel.to_string(), Node::File(fs::read(&path).ok()));
        }
        Err(_) => {
            nodes.remove(rel);
        }
    }
}

/// A lobe on the way to a given path that the work tree no longer has, and
/// whose only files left in the view are maps, goes with them: they are
/// taken out of the view and returned, to be deleted in the commit.
fn drop_removed_lobes(
    root: &Path,
    paths: &[String],
    nodes: &mut BTreeMap<String, Node>,
) -> Vec<String> {
    let mut removed = BTreeSet::new();
    for path in paths {
        let mut dir = parent_dir(path);
        while !dir.is_empty() {
            let prefix = format!("{dir}/");
            let on_disk = fs::symlink_metadata(join(root, dir)).is_ok_and(|m| m.is_dir());
            let left: Vec<&String> = nodes
                .keys()
                .filter(|rel| rel.starts_with(&prefix))
                .collect();
            if !on_disk
                && nodes.contains_key(&map_of(dir))
                && left.iter().all(|rel| base_name(rel) == "map.md")
            {
                let maps: Vec<String> = left.into_iter().cloned().collect();
                for rel in maps {
                    nodes.remove(&rel);
                    removed.insert(rel);
                }
            }
            dir = parent_dir(dir);
        }
    }
    removed.into_iter().collect()
}

/// Which of `paths` differ between `base` (none: an empty tree) and `tree`.
fn changed_paths<G: Plumbing>(
    git: &G,
    repo: &Repo,
    base: Option<&str>,
    tree: &str,
    paths: &[String],
) -> Result<BTreeSet<String>, GitError> {
    match base {
        Some(rev) => {
            let base_tree = git.tree_of(repo, rev)?;
            if base_tree == tree {
                Ok(BTreeSet::new())
            } else {
                git.changed(repo, &base_tree, tree, paths)
            }
        }
        None => git.tracked(repo, tree, paths),
    }
}

/// `text` with every line ending made `ending`.
fn with_line_ending(text: &[u8], ending: LineEnding) -> Vec<u8> {
    let text = String::from_utf8_lossy(text).replace("\r\n", "\n");
    match ending {
        LineEnding::Lf => text.into_bytes(),
        LineEnding::Crlf => text.replace('\n', "\r\n").into_bytes(),
    }
}

/// A work-tree file's identity: what it is, its size and modification time
/// (on Unix its device and inode too), so a file replaced or rewritten since
/// is told apart.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    link: bool,
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    id: (u64, u64),
}

/// A work-tree file as it was when its cleanliness was checked (`None`:
/// not there).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen(Option<(Stamp, Option<Vec<u8>>)>);

impl Seen {
    fn of(path: &Path) -> Seen {
        Seen(fs::symlink_metadata(path).ok().map(|meta| {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt as _;
            let stamp = Stamp {
                link: meta.file_type().is_symlink(),
                len: meta.len(),
                modified: meta.modified().ok(),
                #[cfg(unix)]
                id: (meta.dev(), meta.ino()),
            };
            (stamp, fs::read(path).ok())
        }))
    }
}

/// Bring a regenerated map's work-tree copy to the committed bytes when it
/// had no change of its own (`clean`) and is still as it was then (`seen`),
/// keeping its line endings, as a checkout would; a map with changes of its
/// own, or changed since, is left as it is. A new map is created only where
/// its directory is.
fn update_worktree_map(root: &Path, rel: &str, committed: &[u8], clean: bool, seen: &Seen) {
    let path = join(root, rel);
    if !clean || Seen::of(&path) != *seen {
        return;
    }
    let bytes = match &seen.0 {
        Some((_, Some(old))) => {
            let ending = line_ending(&String::from_utf8_lossy(old));
            let new = with_line_ending(committed, ending);
            if new == *old {
                return;
            }
            new
        }
        Some((_, None)) => return,
        None if path.parent().is_some_and(Path::is_dir) => committed.to_vec(),
        None => return,
    };
    // A failed write leaves the map as it was; it is then listed in
    // `left_dirty`.
    let _ = atomic::write_atomic(&path, &bytes);
}

/// The repo of a brain seen from its work tree's root, for listings of the
/// whole tree: the brain directory with one `..` per part of its prefix
/// (the prefix is git's, so the parents are the real ones).
fn whole_repo(repo: &Repo) -> Repo {
    let dir = repo
        .prefix
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(repo.dir.clone(), |dir, _| dir.join(".."));
    Repo {
        dir,
        prefix: String::new(),
    }
}

/// A tree's files by path: their mode and blob id.
fn entry_map<G: Plumbing>(
    git: &G,
    repo: &Repo,
    rev: &str,
) -> Result<BTreeMap<String, (String, String)>, GitError> {
    Ok(git
        .tree_entries(repo, rev)?
        .into_iter()
        .map(|e| (e.rel, (e.mode, e.oid)))
        .collect())
}

/// The paths `tree` changes beyond `stage` (each taken as `tree` has it),
/// compared with `base` (none: an empty tree): empty when the commit
/// changes nothing else. Brain-relative; one outside a nested brain with
/// `../` parts.
fn unapproved_changes<G: Plumbing>(
    git: &G,
    repo: &Repo,
    base: Option<&str>,
    tree: &str,
    stage: &[String],
) -> Result<Vec<String>, GitError> {
    // What the commit may be: the base with the stage paths as `tree` has
    // them. Any other difference is not approved.
    let entries = entry_map(git, repo, tree)?;
    let expected = git.staging_index(repo, base)?;
    let mut taken = Vec::new();
    let mut gone = Vec::new();
    for rel in stage {
        match entries.get(rel) {
            Some((mode, oid)) => taken.push((rel.clone(), mode.clone(), oid.clone())),
            None => gone.push(StagedBlob {
                rel: rel.clone(),
                mode: "100644".into(),
                bytes: None,
            }),
        }
    }
    git.stage_entries(repo, &expected, &taken)?;
    git.stage_blobs(repo, &expected, &gone)?;
    if git.write_tree(repo, &expected)? == tree {
        return Ok(Vec::new());
    }

    // Name them, over the whole repo.
    let whole = whole_repo(repo);
    let before = match base {
        Some(rev) => entry_map(git, &whole, rev)?,
        None => BTreeMap::new(),
    };
    let after = entry_map(git, &whole, tree)?;
    let up = "../".repeat(repo.prefix.split('/').filter(|p| !p.is_empty()).count());
    let mut out: Vec<String> = before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|rel| before.get(*rel) != after.get(*rel))
        .map(|rel| match rel.strip_prefix(&repo.prefix) {
            Some(inside) => inside.to_string(),
            None => format!("{up}{rel}"),
        })
        .filter(|rel| !stage.contains(rel))
        .collect();
    if out.is_empty() {
        // Only an entry the listing leaves out (a submodule) differs.
        out.push("a submodule entry".into());
    }
    Ok(out)
}

/// Removes a file when dropped.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Run a hook that can stop the commit.
fn gate<G: Plumbing>(
    git: &G,
    repo: &Repo,
    index: &StagingIndex,
    name: &'static str,
    args: &[&str],
) -> Result<(), CommitError> {
    match git.run_hook(repo, Some(index), name, args)? {
        0 => Ok(()),
        code => Err(CommitError::Hook { name, code }),
    }
}

/// Commit `request.paths` and the generated files they change, in the brain
/// at `root` (see the module docs). Nothing to commit is not an error: the
/// report's `commit` is `None`.
pub fn commit<G: Plumbing>(
    root: &Path,
    git: &G,
    request: &CommitRequest,
) -> Result<CommitReport, CommitError> {
    let found = git.version()?;
    if !parse_version(&found).is_some_and(|v| v >= MIN_SYNC_VERSION) {
        return Err(CommitError::GitTooOld { found });
    }
    let repo = git.repo(root).ok_or(CommitError::NotARepo)?;
    if let Some(op) = git.in_progress(&repo)? {
        return Err(CommitError::InProgress(op));
    }
    if request.paths.is_empty() {
        return Err(CommitError::NoPaths);
    }
    let mut given: Vec<(&str, String)> = Vec::new();
    for raw in request.paths {
        let rel = normalize_path(raw).map_err(|problem| CommitError::Path {
            path: raw.clone(),
            problem,
        })?;
        if !given.iter().any(|(_, r)| *r == rel) {
            given.push((raw, rel));
        }
    }
    let paths: Vec<String> = given.iter().map(|(_, rel)| rel.clone()).collect();

    let base = git.head(&repo)?;
    let in_base = |rels: &[String]| match &base {
        Some(rev) => git.tracked(&repo, rev, rels),
        None => Ok(BTreeSet::new()),
    };
    let tracked = in_base(&paths)?;
    for (raw, rel) in &given {
        check_on_disk(root, rel, tracked.contains(rel)).map_err(|problem| CommitError::Path {
            path: raw.to_string(),
            problem,
        })?;
    }

    // The view: the base revision's files, the given paths from the work
    // tree, nothing else.
    let (mut nodes, modes) = match &base {
        Some(rev) => base_nodes(git, &repo, rev)?,
        None => (BTreeMap::new(), BTreeMap::new()),
    };
    for rel in &paths {
        overlay_from_disk(root, rel, &mut nodes);
    }
    let removed_maps = drop_removed_lobes(root, &paths, &mut nodes);
    let view = TreeStore::new(root, nodes);

    // The map step over the view, limited to the generated files the paths
    // change.
    let generated_in_base = in_base(&generated_candidates(&paths))?;
    let mut affected = affected_generated(&paths, view.dirs(), &generated_in_base);
    // A removed lobe's sub-lobe maps go with it, wherever they are.
    affected.extend(removed_maps.iter().cloned());
    // The map step writes a linked map at its target: each map by the path
    // in the tree that holds its bytes.
    let affected: BTreeSet<String> = affected
        .into_iter()
        .flat_map(|rel| [view.real_rel(&rel), Some(rel)])
        .flatten()
        .collect();
    let (changes, skipped) = map_changes(&view, &request.maps);
    if let Some((rel, reason)) = skipped.iter().find(|(rel, _)| affected.contains(*rel)) {
        return Err(CommitError::Regenerate {
            rel: rel.clone(),
            reason: reason.clone(),
        });
    }
    let mode = |rel: &str| modes.get(rel).cloned().unwrap_or_else(|| "100644".into());
    let mut generated: Vec<StagedBlob> = changes
        .writes
        .into_iter()
        .filter(|write| affected.contains(&write.rel))
        .map(|write| StagedBlob {
            mode: mode(&write.rel),
            rel: write.rel,
            bytes: Some(write.contents),
        })
        .collect();
    generated.extend(removed_maps.iter().map(|rel| StagedBlob {
        rel: rel.clone(),
        mode: mode(rel),
        bytes: None,
    }));
    // Each map as it is before its cleanliness is checked, so one changed
    // after the check is never replaced.
    let seen: BTreeMap<String, Seen> = generated
        .iter()
        .map(|blob| (blob.rel.clone(), Seen::of(&join(root, &blob.rel))))
        .collect();
    let dirty_before: BTreeSet<String> = git.dirty(&repo)?.into_iter().collect();

    let index = git.staging_index(&repo, base.as_deref())?;
    git.stage_worktree(&repo, &index, &paths)?;
    git.stage_blobs(&repo, &index, &generated)?;
    let mut stage = paths.clone();
    stage.extend(
        generated
            .iter()
            .filter(|blob| !paths.contains(&blob.rel))
            .map(|blob| blob.rel.clone()),
    );
    let tree = git.write_tree(&repo, &index)?;
    if changed_paths(git, &repo, base.as_deref(), &tree, &stage)?.is_empty() {
        return Ok(CommitReport {
            commit: None,
            paths,
            regenerated: Vec::new(),
            left_dirty: git.dirty(&repo)?,
            set: None,
            message: None,
        });
    }

    if !request.no_verify {
        gate(git, &repo, &index, "pre-commit", &[])?;
    }
    // The hook may have changed the index, as with `git commit`, but only
    // for the paths approved and the maps regenerated for them.
    let tree = git.write_tree(&repo, &index)?;
    let unapproved = unapproved_changes(git, &repo, base.as_deref(), &tree, &stage)?;
    if !unapproved.is_empty() {
        return Err(CommitError::Unapproved { paths: unapproved });
    }
    let changed = changed_paths(git, &repo, base.as_deref(), &tree, &stage)?;
    let regenerated: Vec<String> = affected
        .iter()
        .filter(|rel| changed.contains(*rel) && !paths.contains(rel))
        .cloned()
        .collect();

    let author = git.author(&repo)?;
    let approver = git
        .config(&repo, "user.name")
        .unwrap_or_else(|| author.name.clone());
    let message = match request.message {
        Some(message) if !message.trim().is_empty() => message.to_string(),
        _ => default_message(&paths),
    };
    let set = CommitSet {
        paths: stage,
        base,
        author,
        approver,
        co_author: request
            .agent
            .map(str::trim)
            .filter(|agent| !agent.is_empty())
            .map(str::to_string),
        message,
    };

    // The message hooks work on a file, as with `git commit`.
    let file =
        TempFile(git.git_dir_path(&repo, &format!("KLURIS_EDITMSG-{}", std::process::id()))?);
    // Hooks are shell scripts (Git for Windows' sh too): `/` separators.
    let file_arg = file.0.to_string_lossy().replace('\\', "/");
    fs::write(&file.0, set.full_message()).map_err(|e| CommitError::MessageFile(e.to_string()))?;
    gate(
        git,
        &repo,
        &index,
        "prepare-commit-msg",
        &[&file_arg, "message"],
    )?;
    if !request.no_verify {
        gate(git, &repo, &index, "commit-msg", &[&file_arg])?;
    }
    let final_message = fs::read(&file.0)
        .map(|bytes| cleanup_message(&String::from_utf8_lossy(&bytes)))
        .map_err(|e| CommitError::MessageFile(e.to_string()))?;
    if final_message.is_empty() {
        return Err(CommitError::EmptyMessage);
    }
    fs::write(&file.0, &final_message).map_err(|e| CommitError::MessageFile(e.to_string()))?;

    let parents: Vec<&str> = set.base.iter().map(String::as_str).collect();
    let new = git.commit_tree(&repo, &tree, &parents, &file.0)?;
    let subject = final_message.lines().next().unwrap_or_default();
    git.update_head(
        &repo,
        &new,
        set.base.as_deref(),
        &format!("kluris commit: {subject}"),
    )
    .map_err(CommitError::HeadMoved)?;
    drop(index);
    git.reset_paths(&repo, &new, &set.paths)
        .map_err(|error| CommitError::IndexNotUpdated {
            commit: new.clone(),
            error,
        })?;
    for blob in &generated {
        if let Some(bytes) = &blob.bytes
            && regenerated.contains(&blob.rel)
            && let Some(seen) = seen.get(&blob.rel)
        {
            let clean = !dirty_before.contains(&blob.rel);
            update_worktree_map(root, &blob.rel, bytes, clean, seen);
        }
    }
    // As with `git commit`, post-commit cannot undo or fail the commit.
    let _ = git.run_hook(&repo, None, "post-commit", &[]);
    Ok(CommitReport {
        commit: Some(new),
        paths,
        regenerated,
        left_dirty: git.dirty(&repo)?,
        set: Some(set),
        message: Some(final_message),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalize_keeps_plain_paths_and_drops_dots() {
        assert_eq!(normalize_path("a.md"), Ok("a.md".into()));
        assert_eq!(normalize_path("notes/./a.md"), Ok("notes/a.md".into()));
        assert_eq!(normalize_path("notes//a.md"), Ok("notes/a.md".into()));
        assert_eq!(normalize_path("x/../notes/a.md"), Ok("notes/a.md".into()));
        assert_eq!(normalize_path("été/naïve.md"), Ok("été/naïve.md".into()));
        // On POSIX a backslash is part of a name; on Windows a separator.
        let expected = if cfg!(windows) {
            "notes/a.md"
        } else {
            "notes\\a.md"
        };
        assert_eq!(normalize_path("notes\\a.md"), Ok(expected.into()));
    }

    #[test]
    fn normalize_refuses_what_leaves_the_brain() {
        assert_eq!(normalize_path(""), Err(PathProblem::Empty));
        assert_eq!(normalize_path("."), Err(PathProblem::Empty));
        assert_eq!(normalize_path("notes/.."), Err(PathProblem::Empty));
        assert_eq!(normalize_path("/etc/passwd"), Err(PathProblem::Absolute));
        assert_eq!(normalize_path("../a.md"), Err(PathProblem::Outside));
        assert_eq!(
            normalize_path("notes/../../a.md"),
            Err(PathProblem::Outside)
        );
        assert_eq!(normalize_path(".git/config"), Err(PathProblem::GitDir));
        assert_eq!(normalize_path("notes/.GIT/x"), Err(PathProblem::GitDir));
        if cfg!(windows) {
            assert_eq!(normalize_path("C:\\a.md"), Err(PathProblem::Absolute));
            assert_eq!(normalize_path("C:a.md"), Err(PathProblem::Absolute));
            assert_eq!(normalize_path("\\a.md"), Err(PathProblem::Absolute));
            assert_eq!(
                normalize_path("\\\\server\\share\\a.md"),
                Err(PathProblem::Absolute)
            );
            assert_eq!(normalize_path("..\\a.md"), Err(PathProblem::Outside));
        }
    }

    #[test]
    fn the_default_message_lists_the_paths() {
        assert_eq!(default_message(&strings(&["a.md"])), "kluris: update a.md");
        assert_eq!(
            default_message(&strings(&["notes/a.md", "b.md"])),
            "kluris: update notes/a.md, b.md"
        );
    }

    #[test]
    fn a_long_default_message_is_shortened_with_every_path_in_the_body() {
        let paths: Vec<String> = (1..=8)
            .map(|n| format!("notes/neuron-number-{n:02}.md"))
            .collect();
        let message = default_message(&paths);
        let mut expected = String::from("kluris: update notes/neuron-number-01.md and 7 more\n\n");
        expected.push_str(&paths.join("\n"));
        assert_eq!(message, expected);
        assert!(message.lines().next().unwrap().chars().count() <= SUBJECT_MAX);

        // No path fits: the count alone.
        let long: Vec<String> = (1..=3)
            .map(|n| format!("{}{n}.md", "x".repeat(70)))
            .collect();
        let message = default_message(&long);
        assert!(
            message.starts_with("kluris: update 3 files\n\n"),
            "{message}"
        );
        // One path too long for the subject: its file name.
        let one = vec![format!("{}/name.md", "d".repeat(80))];
        assert_eq!(
            default_message(&one),
            format!("kluris: update name.md\n\n{}", one[0])
        );
    }

    fn commit_set(message: &str, co_author: Option<&str>) -> CommitSet {
        CommitSet {
            paths: strings(&["a.md"]),
            base: Some("abc".into()),
            author: Identity {
                name: "Ana".into(),
                email: "ana@example.com".into(),
            },
            approver: "Ana Tester".into(),
            co_author: co_author.map(str::to_string),
            message: message.into(),
        }
    }

    #[test]
    fn the_message_ends_with_the_trailers() {
        assert_eq!(
            commit_set("kluris: update a.md", None).full_message(),
            "kluris: update a.md\n\nKluris-Approved-By: Ana Tester\n"
        );
        assert_eq!(
            commit_set("Subject\n\nBody.\n\n", Some("Claude <c@x>")).full_message(),
            "Subject\n\nBody.\n\nKluris-Approved-By: Ana Tester\nCo-Authored-By: Claude <c@x>\n"
        );
    }

    #[test]
    fn a_trailer_value_is_one_line() {
        let set = CommitSet {
            approver: "Ana\nEvil: yes".into(),
            ..commit_set("s", Some("A\r\nB"))
        };
        assert_eq!(
            set.full_message(),
            "s\n\nKluris-Approved-By: Ana Evil: yes\nCo-Authored-By: A B\n"
        );
    }

    #[test]
    fn a_message_is_cleaned_up_as_git_commit_does() {
        assert_eq!(
            cleanup_message("Subject  \n\n\n\nBody\t\n\n"),
            "Subject\n\nBody\n"
        );
        assert_eq!(cleanup_message("\n\n  \nSubject\r\n"), "Subject\n");
        assert_eq!(cleanup_message(" \n\n"), "");
        assert_eq!(cleanup_message("# kept\nline"), "# kept\nline\n");
    }

    #[test]
    fn affected_maps_of_a_neuron_in_an_existing_lobe() {
        let dirs = strings(&["ideas", "notes"]);
        let head = set(&["brain.md", "ideas/map.md", "notes/map.md"]);
        assert_eq!(
            affected_generated(&strings(&["notes/a.md"]), &dirs, &head),
            set(&["notes/map.md"])
        );
        // A file at the root changes no map.
        assert_eq!(
            affected_generated(&strings(&["glossary.md"]), &dirs, &head),
            set(&[])
        );
    }

    #[test]
    fn affected_maps_of_a_new_lobe() {
        let dirs = strings(&["ideas", "new", "notes"]);
        let head = set(&["brain.md", "ideas/map.md", "notes/map.md"]);
        assert_eq!(
            affected_generated(&strings(&["new/a.md"]), &dirs, &head),
            set(&["brain.md", "ideas/map.md", "new/map.md", "notes/map.md"])
        );
    }

    #[test]
    fn affected_maps_of_new_nested_lobes() {
        let dirs = strings(&[
            "ideas",
            "notes",
            "notes/deep",
            "notes/deep/inner",
            "notes/side",
        ]);
        let head = set(&[
            "brain.md",
            "ideas/map.md",
            "notes/map.md",
            "notes/side/map.md",
        ]);
        assert_eq!(
            affected_generated(&strings(&["notes/deep/inner/x.md"]), &dirs, &head),
            set(&[
                "notes/deep/inner/map.md",
                "notes/deep/map.md",
                "notes/map.md",
                "notes/side/map.md"
            ])
        );
    }

    #[test]
    fn affected_maps_of_a_removed_lobe() {
        let dirs = strings(&["notes"]);
        let head = set(&["brain.md", "ideas/map.md", "notes/map.md"]);
        assert_eq!(
            affected_generated(&strings(&["ideas/idea.md"]), &dirs, &head),
            set(&["brain.md", "ideas/map.md", "notes/map.md"])
        );
    }

    #[test]
    fn a_top_level_map_given_as_a_path_changes_brain_md() {
        let dirs = strings(&["notes"]);
        let head = set(&["brain.md", "notes/map.md"]);
        assert_eq!(
            affected_generated(&strings(&["notes/map.md"]), &dirs, &head),
            set(&["brain.md", "notes/map.md"])
        );
    }

    #[test]
    fn a_path_inside_a_pruned_directory_changes_no_map() {
        // `.github/x.md`: the walker prunes dot directories, they get no map.
        let dirs = strings(&["notes"]);
        let head = set(&["brain.md", "notes/map.md"]);
        assert_eq!(
            affected_generated(
                &strings(&[".github/x.md", "node_modules/y.md"]),
                &dirs,
                &head
            ),
            set(&[])
        );
    }
}
