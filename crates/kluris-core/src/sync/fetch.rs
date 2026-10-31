//! The fetch of a brain's upstream, the lowest step of the sync engine.
//!
//! [`fetch`] learns the upstream's commit and brings it, never prompting,
//! by its deadline, reading nothing another git in the brain can rewrite,
//! then moves the remote-tracking ref there itself:
//! 1. the ref is checked ([`unmovable`]): one outside `refs/remotes/` (a
//!    fetch refspec that maps the branch onto a local branch, the
//!    checked-out one perhaps) or a symbolic ref would carry the move to a
//!    branch while its index and files stay where they were, so it is
//!    [`Fetch::Refused`] before anything changes;
//! 2. [`bring`]: the commit comes from this fetch's own `git ls-remote
//!    <remote> <branch>`, never from `FETCH_HEAD` or a re-read of the
//!    tracking ref; a fetch of that branch alone brings the objects and
//!    writes no ref and no `FETCH_HEAD`, so a fetch stopped at the deadline
//!    holds no ref lock; then the commit's presence is checked locally,
//!    lazy fetching off. When the remote moved its branch away from that
//!    commit before the fetch brought it, it fetches once more, and is
//!    refused if that happens again;
//! 3. the ref is checked again, then moved by compare-and-swap ([`advance`]:
//!    `git update-ref --no-deref <ref> <new> <old>`, from the value it held
//!    before the fetch), which replaces a ref made symbolic meanwhile
//!    instead of following it. One lost to another git (a concurrent `git
//!    fetch`) keeps that git's value, once the ref is checked again the same
//!    way; Kluris then fetches once more and goes on from that second
//!    fetch's commit, whether its compare-and-swap wins or loses too.
//!
//! Offline when a step is stopped at the deadline or the network is in the
//! way ([`super::git::network_failure`]); refused, with git's message and
//! the fix, when the remote turned the fetch away ([`refused_access`]) or a
//! ref lock stops the move ([`ref_locked`]), which Kluris never removes.
//! Nothing upstream (the remote has no such branch) is [`Fetch::NoBranch`]:
//! the tracking ref is left as it is and its value is not the upstream's.
//!
//! A fetch that brought the commit ends the back-off; [`fetched_recently`],
//! [`back_off`] and their files are the read throttle: at most one read's
//! fetch per [`THROTTLE`], and after a fetch stopped at the deadline none for
//! [`BACKOFF_FIRST`], each further stop doubling that up to [`BACKOFF_MAX`].

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::git::{RefLock, RemoteError, SyncGit, Upstream, network_failure, one_line, ref_lock};
use crate::atomic;
use crate::git::{GitError, Repo};

/// The stamp of the last read's fetch, relative to the git directory.
pub const STAMP: &str = "kluris-sync/last-fetch";

/// The least time between two reads' fetches of a brain.
pub const THROTTLE: Duration = Duration::from_secs(60);

/// The back-off after a fetch stopped at the deadline, relative to the git
/// directory: `<until> <window>\n`, both in Unix seconds and seconds.
pub const BACKOFF: &str = "kluris-sync/fetch-backoff";

/// The first back-off after a fetch stopped at the deadline.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(60 * 60);

/// The longest back-off: each stop doubles it up to this.
pub const BACKOFF_MAX: Duration = Duration::from_secs(24 * 60 * 60);

/// What a fetch of the upstream did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch {
    /// The upstream's commit is `new`, which the tracking ref held `old`
    /// before (`None`: no tracking ref yet). The ref holds `new`, or what
    /// another git moved it to meanwhile.
    Moved { old: Option<String>, new: String },
    /// Nothing to move: the tracking ref already holds the upstream's
    /// commit, `at`.
    Same { at: String },
    /// The remote has no such branch; the tracking ref is left as it is.
    NoBranch,
    /// Stopped at the deadline, or the network is in the way.
    Offline,
    /// The brain is left alone: why, and the fix.
    Refused { detail: String },
}

/// What [`bring`] brought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Brought {
    /// The commit the remote's branch is at, from this `ls-remote`, and here.
    Commit(String),
    /// The remote has no such branch.
    NoBranch,
    /// The commit `ls-remote` gave is not here after the fetch: the remote
    /// moved its branch away from it, or removed the branch, in between.
    Gone,
}

/// The upstream's commit and its objects, by `deadline`, changing no ref:
/// `git ls-remote <remote> <branch>` says the commit, then a fetch of the
/// branch brings the objects ([`SyncGit::fetch_branch`]), then the commit is
/// checked here, locally ([`SyncGit::has_commit`]).
pub fn bring<G: SyncGit>(
    git: &G,
    repo: &Repo,
    upstream: &Upstream,
    deadline: Instant,
) -> Result<Brought, RemoteError> {
    let Some(commit) = git.ls_remote(repo, upstream, deadline)? else {
        return Ok(Brought::NoBranch);
    };
    match git.fetch_branch(repo, upstream, deadline) {
        Ok(()) => {}
        // The branch went after `ls-remote` listed it.
        Err(RemoteError::Failed(e)) if e.detail.contains("couldn't find remote ref") => {
            return Ok(Brought::Gone);
        }
        Err(e) => return Err(e),
    }
    if git.has_commit(repo, &commit, deadline)? {
        Ok(Brought::Commit(commit))
    } else {
        Ok(Brought::Gone)
    }
}

/// Fetch the upstream and move its tracking ref, by `deadline` (see the
/// module docs). A local step that fails is the error.
pub fn fetch<G: SyncGit>(
    git: &G,
    repo: &Repo,
    upstream: &Upstream,
    deadline: Instant,
) -> Result<Fetch, GitError> {
    let (mut lost_once, mut gone_once) = (false, false);
    loop {
        if let Some(detail) = unmovable(git, repo, upstream)? {
            return Ok(Fetch::Refused { detail });
        }
        let before = git.ref_value(repo, &upstream.refname)?;
        let brought = match bring(git, repo, upstream, deadline) {
            Ok(brought) => brought,
            Err(e) => {
                return Ok(match fetch_refused(git, repo, &upstream.remote, &e) {
                    Some(detail) => Fetch::Refused { detail },
                    None => Fetch::Offline,
                });
            }
        };
        if let Ok(path) = git.git_dir_path(repo, BACKOFF) {
            end_backoff(&path);
        }
        if let Some(detail) = unmovable(git, repo, upstream)? {
            return Ok(Fetch::Refused { detail });
        }
        let new = match brought {
            Brought::Commit(new) => new,
            Brought::NoBranch => return Ok(Fetch::NoBranch),
            Brought::Gone if !gone_once => {
                gone_once = true;
                continue;
            }
            Brought::Gone => {
                return Ok(Fetch::Refused {
                    detail: remote_moved_away(upstream),
                });
            }
        };
        if before.as_ref() == Some(&new) {
            return Ok(Fetch::Same { at: new });
        }
        let message = format!("kluris sync: fetch {}", upstream.remote);
        match advance(git, repo, upstream, before.as_deref(), &new, &message)? {
            Advance::Moved => return Ok(Fetch::Moved { old: before, new }),
            Advance::Refused(detail) => return Ok(Fetch::Refused { detail }),
            // Another git moved the ref, a concurrent `git fetch`: its
            // value is kept, never merged.
            Advance::Lost if lost_once => return Ok(Fetch::Moved { old: before, new }),
            Advance::Lost => lost_once = true,
        }
    }
}

/// How [`advance`] left the tracking ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Advance {
    /// It holds the new value.
    Moved,
    /// Another git moved it meanwhile; its value is kept.
    Lost,
    /// It is left alone: why, and the fix.
    Refused(String),
}

/// Move the upstream's tracking ref from `old` to `new` by compare-and-swap
/// (`git update-ref --no-deref`), never following a symbolic ref. When the
/// swap fails, the ref is checked again ([`unmovable`]) before its value is
/// used: one still at `old` was stopped by a lock ([`ref_locked`]) or fails
/// with git's error; one another git moved is kept.
pub fn advance<G: SyncGit>(
    git: &G,
    repo: &Repo,
    upstream: &Upstream,
    old: Option<&str>,
    new: &str,
    message: &str,
) -> Result<Advance, GitError> {
    let Err(e) = git.move_ref(repo, &upstream.refname, new, old, message) else {
        return Ok(Advance::Moved);
    };
    if let Some(why) = unmovable(git, repo, upstream)? {
        return Ok(Advance::Refused(why));
    }
    if git.ref_value(repo, &upstream.refname)?.as_deref() == old {
        return match locked_ref(git, repo, &e.detail) {
            Some(lock) => Ok(Advance::Refused(ref_locked(&lock))),
            None => Err(e),
        };
    }
    Ok(Advance::Lost)
}

/// Why Kluris leaves a brain whose upstream's remote-tracking ref it will
/// not move, or `None`: a ref outside `refs/remotes/` or a symbolic ref
/// would carry the move to a branch, while its index and files stay where
/// they were. A tracking ref that does not exist yet is fine: the move
/// creates it.
pub fn unmovable<G: SyncGit>(
    git: &G,
    repo: &Repo,
    upstream: &Upstream,
) -> Result<Option<String>, GitError> {
    let (refname, remote) = (&upstream.refname, &upstream.remote);
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

/// Why Kluris leaves a brain whose remote moved its branch away from the
/// commit `ls-remote` gave, before the fetch brought it, on each try.
fn remote_moved_away(upstream: &Upstream) -> String {
    let branch = upstream
        .remote_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&upstream.remote_ref);
    format!(
        "{}'s {branch} moved while Kluris fetched it, on each try; nothing was merged or pushed: run kluris sync again",
        upstream.remote
    )
}

/// Why Kluris leaves a brain whose fetch found a ref locked alone: `lock`,
/// and how to clear it.
pub fn ref_locked(lock: &str) -> String {
    format!(
        "a ref is locked: {lock} exists, so git cannot update the brain's refs; if no git is running in this brain, remove that file and run kluris sync again"
    )
}

/// Why Kluris leaves a brain whose remote turned it away (a key or token
/// that does not work, no access, no such repository: anything but network
/// trouble): git's message on one line, then the fix. `command` is `fetch`
/// or `push`.
pub fn refused_access(command: &str, remote: &str, stderr: &str) -> String {
    let mut message = one_line(stderr);
    if message.is_empty() {
        message = format!("git {command} {remote} failed");
    }
    format!(
        "{message}; check that this machine can run `git {command} {remote}` in this brain without a prompt"
    )
}

/// Why a failed fetch leaves the brain alone, or `None` when it is offline:
/// the deadline, or network trouble ([`network_failure`]). A ref lock is
/// named ([`ref_locked`]); anything else is a refusal ([`refused_access`]),
/// shown, never taken for offline.
fn fetch_refused<G: SyncGit>(
    git: &G,
    repo: &Repo,
    remote: &str,
    error: &RemoteError,
) -> Option<String> {
    let RemoteError::Failed(e) = error else {
        return None;
    };
    if let Some(lock) = locked_ref(git, repo, &e.detail) {
        return Some(ref_locked(&lock));
    }
    let urls = git.remote_urls(repo, remote, false).unwrap_or_default();
    if network_failure(&urls, &e.detail) {
        return None;
    }
    Some(refused_access("fetch", remote, &e.detail))
}

/// The lock file a failed ref update was stopped by, if that is why it
/// failed (git's stderr in `detail`): the one git named, or the named ref's
/// `<ref>.lock` when it exists.
fn locked_ref<G: SyncGit>(git: &G, repo: &Repo, detail: &str) -> Option<String> {
    match ref_lock(detail)? {
        RefLock::File(path) => Some(path),
        RefLock::Ref(name) => {
            let path = git.git_dir_path(repo, &format!("{name}.lock")).ok()?;
            fs::symlink_metadata(&path)
                .is_ok()
                .then(|| path.display().to_string())
        }
    }
}

/// `time` in Unix seconds (0 before the epoch).
pub fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether the stamp at `stamp` records a time in the `throttle` before
/// `now`; a stamp that does not read, or from the future (the clock went
/// back), does not.
pub fn fetched_recently(stamp: &Path, now: SystemTime, throttle: Duration) -> bool {
    let Ok(text) = fs::read_to_string(stamp) else {
        return false;
    };
    let Ok(at) = text.trim().parse::<u64>() else {
        return false;
    };
    let now = unix_seconds(now);
    at <= now && now - at < throttle.as_secs()
}

/// Record a fetch attempt at `now`; a stamp that cannot be written only
/// costs a fetch.
pub fn write_stamp(stamp: &Path, now: SystemTime) {
    if let Some(parent) = stamp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = atomic::write_atomic(stamp, format!("{}\n", unix_seconds(now)).as_bytes());
}

/// The back-off's end and its window, as [`BACKOFF`] records them.
fn read_backoff(path: &Path) -> Option<(u64, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut fields = text.split_whitespace();
    let until = fields.next()?.parse::<u64>().ok()?;
    let window = fields.next()?.parse::<u64>().ok()?;
    Some((until, window))
}

/// Whether `now` is inside the back-off at `path`: before its end, and no
/// further from it than its window (at most [`BACKOFF_MAX`]); one that does
/// not read, or lies further ahead (the clock went back), never blocks.
pub fn backed_off(path: &Path, now: SystemTime) -> bool {
    let now = unix_seconds(now);
    read_backoff(path).is_some_and(|(until, window)| {
        let window = window.min(BACKOFF_MAX.as_secs());
        now < until && until - now <= window
    })
}

/// Record a fetch stopped at the deadline at `now`: the next window is
/// twice the last (the first [`BACKOFF_FIRST`]), at most [`BACKOFF_MAX`].
pub fn back_off(path: &Path, now: SystemTime) {
    let window = read_backoff(path).map_or(BACKOFF_FIRST.as_secs(), |(_, last)| {
        last.saturating_mul(2)
            .clamp(BACKOFF_FIRST.as_secs(), BACKOFF_MAX.as_secs())
    });
    let until = unix_seconds(now).saturating_add(window);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = atomic::write_atomic(path, format!("{until} {window}\n").as_bytes());
}

/// End the back-off: a fetch from the brain's remote succeeded.
pub fn end_backoff(path: &Path) {
    let _ = fs::remove_file(path);
}
