//! The two claims that let commands share one checkout, each an advisory OS
//! lock (`flock`, `LockFileEx`) on a file in a git directory the caller
//! resolves:
//!
//! - [`RemoteClaim`] on [`REMOTE_CLAIM`] in the repository's common git
//!   directory (`git rev-parse --git-common-dir`): one network sync of a
//!   repository at a time. Its worktrees share its remote and its
//!   remote-tracking refs, so they share this one claim. A read never
//!   waits for it ([`RemoteClaim::try_take`]): one that finds it held
//!   leaves the network to the holder. An explicit command and the
//!   background push wait for it, tried every [`STEP`] up to
//!   [`COMMAND_BOUND`] ([`RemoteClaim::take`]).
//! - [`TreeClaim`] on [`TREE_CLAIM`] in the worktree's own git directory
//!   (`git rev-parse --absolute-git-dir`, where `--git-path` puts the name):
//!   each worktree has its own files, index and HEAD, so its own claim.
//!   Exclusive to change the work tree (a land), shared to read it. Two
//!   readers hold it together; a reader and a land, or two lands, never do.
//!   A busy tree claim is tried again every [`STEP`] up to the caller's
//!   bound: [`READ_BOUND`] for a read, [`COMMAND_BOUND`] for an explicit
//!   command.
//! - The tree gate, [`TREE_GATE`] beside it: an exclusive taker takes the
//!   gate first and holds it with the claim, so a land that is done can
//!   turn its claim shared ([`TreeClaim::downgrade`]) with no moment in
//!   which another land could start: no other exclusive taker gets past
//!   the gate meanwhile, and shared takers never take it.
//!
//! For the main worktree the two directories are one.
//!
//! No two holders wait for each other. No one waits for the tree claim
//! while holding the remote claim: a pull fetches under the remote claim,
//! lets it go, and only then takes the tree claim. The one holder of both
//! is an explicit command's push when it closes, which takes the remote
//! claim while it holds the tree claim; whoever holds the remote claim
//! meanwhile (another command's fetch, the background push) never waits
//! for a tree claim, so it lets the remote claim go, and the push gets it.
//! The background push never takes the tree claim at all.
//!
//! A lock goes with its file handle, so a command that ends, crashes or is
//! killed releases its claims and blocks no other. A claim dropped is
//! unlocked first: a process started at that moment shares the open file
//! until it runs, and a lock only closing would stay held with it. The
//! files are made when missing and never removed, so no claim is ever taken
//! on a file another process is about to unlink. Nothing is written into
//! them: on Windows a locked file cannot be read by another handle.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// The remote claim's file, relative to the repository's common git
/// directory.
pub const REMOTE_CLAIM: &str = "kluris-sync/remote-claim";

/// The tree claim's file, relative to the worktree's own git directory.
pub const TREE_CLAIM: &str = "kluris-sync/tree-claim";

/// The tree gate's file, relative to the worktree's own git directory.
pub const TREE_GATE: &str = "kluris-sync/tree-gate";

/// The time between two tries of a busy tree claim.
pub const STEP: Duration = Duration::from_millis(25);

/// How long a read waits for the tree claim.
pub const READ_BOUND: Duration = Duration::from_secs(3);

/// How long an explicit command waits for the tree claim.
pub const COMMAND_BOUND: Duration = Duration::from_secs(30);

/// Why a claim was not taken.
#[derive(Debug)]
pub enum ClaimError {
    /// Another holder has it: now for the remote claim, through the whole
    /// bound for the tree claim.
    Busy,
    /// The claim's file could not be made, opened or locked.
    Io(io::Error),
}

impl fmt::Display for ClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClaimError::Busy => f.write_str("the claim is held by another command"),
            ClaimError::Io(e) => write!(f, "the claim could not be taken: {e}"),
        }
    }
}

impl std::error::Error for ClaimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClaimError::Busy => None,
            ClaimError::Io(e) => Some(e),
        }
    }
}

impl From<fs::TryLockError> for ClaimError {
    fn from(error: fs::TryLockError) -> ClaimError {
        match error {
            fs::TryLockError::WouldBlock => ClaimError::Busy,
            fs::TryLockError::Error(e) => ClaimError::Io(e),
        }
    }
}

/// A locked claim file, unlocked when dropped.
#[derive(Debug)]
struct Held(fs::File);

impl Drop for Held {
    fn drop(&mut self) {
        // Closing the file unlocks it too, so nothing is lost if this fails.
        let _ = self.0.unlock();
    }
}

/// The remote claim, held until dropped.
#[derive(Debug)]
pub struct RemoteClaim {
    _held: Held,
}

impl RemoteClaim {
    /// Takes the remote claim of the repository whose common git directory
    /// is `common_git_dir`, without waiting.
    pub fn try_take(common_git_dir: &Path) -> Result<RemoteClaim, ClaimError> {
        let file = open(&common_git_dir.join(REMOTE_CLAIM))?;
        file.try_lock()?;
        Ok(RemoteClaim { _held: Held(file) })
    }

    /// Takes the remote claim of the repository whose common git directory
    /// is `common_git_dir`, trying every [`STEP`] until `bound` has passed.
    pub fn take(common_git_dir: &Path, bound: Duration) -> Result<RemoteClaim, ClaimError> {
        let path = common_git_dir.join(REMOTE_CLAIM);
        let held = take(&path, Instant::now() + bound, fs::File::try_lock)?;
        Ok(RemoteClaim { _held: held })
    }
}

/// The tree claim, exclusive (with the tree gate) or shared, held until
/// dropped.
#[derive(Debug)]
pub struct TreeClaim {
    // Dropped in this order: the claim, then the gate.
    held: Held,
    gate: Option<Held>,
}

/// One way to lock a file without waiting.
type TryLock = fn(&fs::File) -> Result<(), fs::TryLockError>;

impl TreeClaim {
    /// Takes the tree claim of the worktree whose own git directory is
    /// `worktree_git_dir` exclusively, to change its work tree: the tree
    /// gate, then the claim, each tried every [`STEP`] until `bound` has
    /// passed.
    pub fn exclusive(worktree_git_dir: &Path, bound: Duration) -> Result<TreeClaim, ClaimError> {
        let deadline = Instant::now() + bound;
        let gate = take(
            &worktree_git_dir.join(TREE_GATE),
            deadline,
            fs::File::try_lock,
        )?;
        let held = take(
            &worktree_git_dir.join(TREE_CLAIM),
            deadline,
            fs::File::try_lock,
        )?;
        Ok(TreeClaim {
            held,
            gate: Some(gate),
        })
    }

    /// Takes the tree claim of the worktree whose own git directory is
    /// `worktree_git_dir` shared, to read its work tree, trying every
    /// [`STEP`] until `bound` has passed.
    pub fn shared(worktree_git_dir: &Path, bound: Duration) -> Result<TreeClaim, ClaimError> {
        let deadline = Instant::now() + bound;
        let held = take(
            &worktree_git_dir.join(TREE_CLAIM),
            deadline,
            fs::File::try_lock_shared,
        )?;
        Ok(TreeClaim { held, gate: None })
    }

    /// This claim shared, to read the work tree it changed: unlocked, then
    /// locked shared on the same file, while the gate keeps every other
    /// exclusive taker out, so no land starts in between; then the gate
    /// goes. Only shared holders can hold the claim meanwhile, so the
    /// shared lock is had at once; `bound` is how long it is tried. A
    /// shared claim is given back as it is.
    pub fn downgrade(mut self, bound: Duration) -> Result<TreeClaim, ClaimError> {
        let Some(gate) = self.gate.take() else {
            return Ok(self);
        };
        self.held.0.unlock().map_err(ClaimError::Io)?;
        retry(
            &self.held.0,
            Instant::now() + bound,
            fs::File::try_lock_shared,
        )?;
        drop(gate);
        Ok(self)
    }
}

/// The file at `path` opened and locked with `lock`, tried every [`STEP`]
/// until `deadline`.
fn take(path: &Path, deadline: Instant, lock: TryLock) -> Result<Held, ClaimError> {
    let file = open(path)?;
    retry(&file, deadline, lock)?;
    Ok(Held(file))
}

/// `file` locked with `lock`, tried every [`STEP`] until `deadline`; busy
/// only once it has passed.
fn retry(file: &fs::File, deadline: Instant, lock: TryLock) -> Result<(), ClaimError> {
    loop {
        match lock(file) {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(error) => return Err(error.into()),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(ClaimError::Busy);
        }
        thread::sleep(STEP.min(deadline - now));
    }
}

/// The claim's file, made (with its folder) when missing, never truncated.
fn open(path: &Path) -> Result<fs::File, ClaimError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(ClaimError::Io)?;
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(ClaimError::Io)
}
