//! The sync engine: every command that opens a brain pulls teammates'
//! commits in before it works ([`pull`]) and shares what is committed when
//! it is done ([`push`]); [`sync`] does both, and [`finish_merge`] lands a
//! merge a person settled.
//!
//! [`open`] pins the branch HEAD names and its upstream: every later step
//! pulls into that branch and pushes that branch's commit, and never reads
//! HEAD for a commit again.
//!
//! [`pull`], local but for the fetch:
//! 1. a git merge, rebase, cherry-pick or revert in progress, or a branch
//!    without an upstream, leaves the brain as it is;
//! 2. the upstream is fetched ([`fetch::fetch`]) under the remote claim; the
//!    only commit that may land is the one this fetch's own `ls-remote`
//!    named. When another command holds the remote claim, the upstream's
//!    commit is not known, so the pull lands nothing (`busy`);
//! 3. under the exclusive tree claim it checks that HEAD still names the
//!    pinned branch and no git operation started ([`land::changed`]), then
//!    decides and moves the work tree:
//!    behind only, it fast-forwards; diverged, it merges in memory
//!    ([`merge_commits`]) and, without conflicts, makes the merge commit
//!    (`kluris: sync` with `Kluris-Sync: auto`, through the merge hooks) and
//!    lands it ([`land`], which checks the branch once more, and that it
//!    still holds the commit the merge was made for). The moment between
//!    that last check and git's own write stays open, since git's
//!    fast-forward moves whichever branch HEAD names when it runs: a commit
//!    or an edit made then makes git refuse (or the second check finds the
//!    edit), and a switch made then moves the other branch, which the
//!    check after the write finds and reports (`busy`, naming the branch
//!    that moved and to what); nothing is undone. Ahead only, there is
//!    nothing to pull. A remote without the branch is ahead only, every
//!    commit from the root to push. Unrelated histories are refused before
//!    any merge;
//! 4. a conflict writes the waiting merge ([`pending`]) and nothing else.
//!    A waiting merge whose `ours` is not HEAD is stale and replaced; one
//!    the upstream now merges cleanly with is dropped and the pull goes on;
//!    otherwise its conflicts are reported again, with the upstream's newer
//!    commits counted in `behind_conflict`.
//!
//! [`push`] is network only, under the remote claim: git's own push of
//! the pinned branch ([`SyncGit::push`]), `git push <remote> <branch>`, so
//! the `pre-push` hooks run as for any push of yours, once for each push
//! URL, and stop with the push at its time limit. Right before it, the
//! engine checks the remote-tracking ref git moves once the remote took
//! the push ([`fetch::unmovable`]); one a `pre-push` hook makes symbolic
//! during the push, git follows, and the next sync refuses it. The commits
//! to push are counted against the remote's fetch URL (`to_push`), so a
//! push URL of its own is pushed to only when that count is not zero.
//! Once the push ends, however it ends, `to_push` is counted again from
//! the pinned branch's ref against the tracking ref git moved, so a commit
//! made while the push ran stays to push. A `pre-push` hook's refusal
//! stops the transfer to the URL it ran for; it undoes nothing the sync
//! pulled or merged, nor a push another URL already took.
//!
//! [`sync`] pulls again after a push the remote turned back because it
//! moved, [`PUSH_ATTEMPTS`] times at most ([`share`]). Only the fetch and
//! the push are bounded in time, unless the caller bounds every step before
//! the land too ([`Context::until`]: the merge hooks and the land's own
//! checks before it writes included); the land and what follows it never
//! are.
//!
//! A caller that fetches and lands apart (the session) has the two halves
//! of [`pull`]: [`fetch_upstream`] under the remote claim, and
//! [`pull_fetched`] under the tree claim it holds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::background::LastPush;
use super::claim::{ClaimError, RemoteClaim, TreeClaim};
use super::fetch::{self, Fetch};
use super::git::{GitDirs, PushOutcome, SyncGit, Upstream, one_line};
use super::hooks::{HookError, MergeHookError, MergeHooks, merge_commit};
use super::land::{Over, Settled, blocked_paths, failed, land, unchanged, uncommitted};
use super::merge::is_generated;
use super::merge::{MergeOutcome, merge_commits};
use super::pending::{self, Found, Pending};
use super::{
    Conflict, ConflictText, LeftAsIs, LeftReason, PUSH_ATTEMPTS, ResolvedChoice, SyncError,
    SyncReport, SyncRequest,
};
use crate::git::{Git, GitError, MIN_SYNC_VERSION, Plumbing, Repo, parse_version};
use crate::json::Json;

const MESSAGE: &str = "kluris: sync\n\nKluris-Sync: auto\n";

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

/// One brain opened for the engine: its repo, its git directories, a git
/// with the remote environment (so no git the engine starts prompts), and
/// the branch HEAD named when it was opened, with that branch's upstream.
#[derive(Debug, Clone)]
pub struct Context<'a, G> {
    root: PathBuf,
    /// The git of every step before the land, the land's checks before it
    /// writes included, stopped at the deadline it was given
    /// ([`Context::until`]).
    git: G,
    /// The git of the land's write and every step after it, never stopped:
    /// a land stopped partway would leave the work tree half moved.
    landing: G,
    repo: Repo,
    dirs: GitDirs,
    /// How long the remote claim another command holds is waited for
    /// ([`Context::wait_for_remote`]); zero: not at all.
    remote_bound: Duration,
    /// `refs/heads/<name>`; `None` on a detached HEAD.
    branch: Option<String>,
    upstream: Option<Upstream>,
    request: SyncRequest<'a>,
}

impl<'a, G: SyncGit> Context<'a, G> {
    pub fn git(&self) -> &G {
        &self.git
    }

    /// Stop every git of the steps before the land at `deadline` (`None`:
    /// never); the land and what follows it are never stopped.
    pub fn until(&mut self, deadline: Option<Instant>) {
        self.git = self.git.clone().with_deadline(deadline);
    }

    /// Wait for the remote claim another command holds up to `bound`
    /// before it is busy.
    pub fn wait_for_remote(&mut self, bound: Duration) {
        self.remote_bound = bound;
    }

    /// The repository's two git directories.
    pub fn dirs(&self) -> &GitDirs {
        &self.dirs
    }

    /// The pinned branch's upstream; `None` without one.
    pub fn upstream(&self) -> Option<&Upstream> {
        self.upstream.as_ref()
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    /// The pinned branch (`refs/heads/<name>`); `None` on a detached HEAD.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Where the worktree's waiting merge is.
    pub fn pending(&self) -> PathBuf {
        pending::path(&self.dirs)
    }

    /// The exclusive tree claim, waited for up to the request's bound;
    /// busy while another command holds it past that.
    pub fn claim_tree(&self) -> Result<TreeClaim, LeftAsIs> {
        TreeClaim::exclusive(&self.dirs.own, self.request.tree_bound).map_err(|e| match e {
            ClaimError::Busy => LeftAsIs::because(
                LeftReason::Busy,
                "another kluris command is changing this brain's files; run again once it is done",
            ),
            ClaimError::Io(e) => failed(format!("cannot claim the work tree: {e}")),
        })
    }

    /// The pinned branch, or why there is none.
    fn pinned(&self) -> Result<&str, LeftAsIs> {
        self.branch().ok_or_else(|| {
            LeftAsIs::because(
                LeftReason::Busy,
                "HEAD is detached; switch to the branch to sync, then run again",
            )
        })
    }

    /// The pinned branch's commit; refused while it has none.
    fn branch_commit(&self, branch: &str) -> Result<String, LeftAsIs> {
        self.git
            .ref_value(&self.repo, branch)?
            .ok_or_else(|| failed("the branch has no commit yet: commit first"))
    }

    /// The remote claim; `None` while another command holds it.
    pub fn take_remote(&self) -> Result<Option<RemoteClaim>, LeftAsIs> {
        let taken = if self.remote_bound.is_zero() {
            RemoteClaim::try_take(&self.dirs.common)
        } else {
            RemoteClaim::take(&self.dirs.common, self.remote_bound)
        };
        match taken {
            Ok(claim) => Ok(Some(claim)),
            Err(ClaimError::Busy) => Ok(None),
            Err(ClaimError::Io(e)) => Err(failed(format!("cannot claim the remote: {e}"))),
        }
    }
}

/// Open the brain at `root`: git new enough, a repository that is the brain
/// itself.
pub fn open<'a, G: SyncGit>(
    root: &Path,
    git: &G,
    request: &SyncRequest<'a>,
) -> Result<Context<'a, G>, SyncError> {
    check_git(git)?;
    let repo = git.discover(root)?.ok_or(SyncError::NotARepo)?;
    if repo.nested() {
        return Err(SyncError::Nested {
            prefix: repo.prefix.clone(),
        });
    }
    // What the hooks of the gits it starts print reaches the user.
    let git = git.for_remote(&repo)?.showing_output();
    let landing = git.clone().with_deadline(None);
    let dirs = git.git_dirs(&repo)?;
    let branch = git.branch(&repo)?;
    let upstream = match &branch {
        Some(branch) => git.upstream(&repo, branch)?,
        None => None,
    };
    Ok(Context {
        root: root.to_path_buf(),
        git,
        landing,
        repo,
        dirs,
        remote_bound: Duration::ZERO,
        branch,
        upstream,
        request: *request,
    })
}

/// Bring the upstream's commits into the brain (see the module docs).
pub fn pull<G: SyncGit>(ctx: &Context<G>) -> SyncReport {
    reported(|report| {
        let theirs = fetch_step(ctx)?;
        let tree = ctx.claim_tree()?;
        land_into(ctx, &tree, theirs, report)
    })
}

/// [`pull`] into a work tree whose claim the caller holds (`tree`).
pub fn pull_claimed<G: SyncGit>(ctx: &Context<G>, tree: &TreeClaim) -> SyncReport {
    reported(|report| {
        let theirs = fetch_step(ctx)?;
        land_into(ctx, tree, theirs, report)
    })
}

/// The rest of a pull once its fetch named `theirs`, the upstream's commit
/// ([`fetch_upstream`]; `None`: the remote has no such branch), into a work
/// tree whose claim the caller holds (`tree`).
pub fn pull_fetched<G: SyncGit>(
    ctx: &Context<G>,
    tree: &TreeClaim,
    theirs: Option<String>,
) -> SyncReport {
    reported(|report| land_into(ctx, tree, theirs, report))
}

/// The report of a pull `step` filled, with why it stopped.
fn reported(step: impl FnOnce(&mut SyncReport) -> Result<(), LeftAsIs>) -> SyncReport {
    let mut report = SyncReport::default();
    if let Err(left) = step(&mut report) {
        report.left_as_is = Some(left);
    }
    report
}

/// The pinned branch's upstream, once no git merge, rebase, cherry-pick or
/// revert is in progress; or why a pull cannot start.
pub fn ready<'c, G: SyncGit>(ctx: &'c Context<G>) -> Result<&'c Upstream, LeftAsIs> {
    if let Some(op) = ctx.git.in_progress(&ctx.repo)? {
        return Err(LeftAsIs::because(
            LeftReason::InProgress,
            format!("a {op} is in progress; finish or abort it first"),
        ));
    }
    upstream(ctx)
}

/// The upstream's commit a pull may land, fetched under the remote claim.
fn fetch_step<G: SyncGit>(ctx: &Context<G>) -> Result<Option<String>, LeftAsIs> {
    let upstream = ready(ctx)?;
    let Some(remote) = ctx.take_remote()? else {
        return Err(remote_busy());
    };
    fetch_upstream(ctx, upstream, &remote)
}

/// Why a pull lands nothing while another command holds the remote claim:
/// the upstream's commit is not known, and the tracking ref, which any git
/// can rewrite, is never taken for it.
pub fn remote_busy() -> LeftAsIs {
    LeftAsIs::because(
        LeftReason::Busy,
        "another kluris command is fetching this brain's remote, so the upstream's commit is not known; nothing was pulled: run again",
    )
}

fn land_into<G: SyncGit>(
    ctx: &Context<G>,
    _tree: &TreeClaim,
    theirs: Option<String>,
    report: &mut SyncReport,
) -> Result<(), LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let upstream = upstream(ctx)?;
    let branch = upstream.branch.as_str();
    let ours = ctx.branch_commit(branch)?;
    unchanged(git, repo, branch, &ours)?;
    let count = |from: Option<&str>, to: &str| git.count(repo, from, to).map_err(LeftAsIs::from);
    // From the land on, nothing is stopped partway.
    let landing = &ctx.landing;
    let landed =
        |from: Option<&str>, to: &str| landing.count(repo, from, to).map_err(LeftAsIs::from);
    let pending_path = ctx.pending();
    let waiting = match Pending::read(&pending_path) {
        Found::None => None,
        Found::Waiting(waiting) if waiting.ours == ours => Some(waiting),
        Found::Waiting(_) | Found::Unreadable => {
            pending::remove(&pending_path).map_err(failed)?;
            None
        }
    };
    let Some(theirs) = theirs else {
        // No tracking ref yet: the push makes the branch.
        report.to_push = count(None, &ours)?;
        return drop_waiting(&pending_path, waiting);
    };
    if theirs == ours {
        // Nothing new either way: nothing to pull or push.
        return drop_waiting(&pending_path, waiting);
    }
    let behind = !git.is_ancestor(repo, &theirs, &ours)?;
    let ahead = !git.is_ancestor(repo, &ours, &theirs)?;
    if !behind {
        report.to_push = count(Some(&theirs), &ours)?;
        return drop_waiting(&pending_path, waiting);
    }
    if !ahead {
        land(
            git,
            landing,
            repo,
            &ctx.root,
            branch,
            &ours,
            &theirs,
            &Over::default(),
        )?;
        report.pulled = landed(Some(&ours), &theirs)?;
        return drop_waiting(&pending_path, waiting);
    }

    report.to_push = count(Some(&theirs), &ours)?;
    let bases = git.merge_bases(repo, &ours, &theirs)?;
    if bases.is_empty() {
        return Err(unrelated(upstream));
    }
    if let Some(waiting) = waiting.as_ref().filter(|w| w.theirs == theirs) {
        report.conflicts = texts(git, repo, waiting)?;
        return Ok(());
    }
    let none = BTreeMap::new();
    let maps = &ctx.request.maps;
    let (tree, index, auto_merged) =
        match merge_commits(git, repo, &ours, &theirs, &bases, maps, &none)? {
            MergeOutcome::Clean {
                tree,
                index,
                auto_merged,
            } => (tree, index, auto_merged),
            MergeOutcome::Conflicts { base, conflicts } => {
                match waiting {
                    Some(waiting) => {
                        report.conflicts = texts(git, repo, &waiting)?;
                        report.behind_conflict = count(Some(&waiting.theirs), &theirs)?;
                    }
                    None => {
                        let waiting = Pending::new(base, &ours, &theirs, conflicts);
                        report.conflicts = texts(git, repo, &waiting)?;
                        waiting
                            .write(&pending_path)
                            .map_err(|e| failed(format!("cannot record the conflict: {e}")))?;
                    }
                }
                return Ok(());
            }
        };
    let dirty = git.dirty(repo)?;
    let blocked = blocked_paths(git, repo, &ctx.root, &dirty, &ours, &tree)?;
    if !blocked.is_empty() {
        return Err(uncommitted(blocked));
    }
    let no_verify = ctx.request.no_verify;
    let commit = merge_commit(
        git,
        repo,
        &index,
        &tree,
        [&ours, &theirs],
        MESSAGE,
        no_verify,
        MergeHooks::Merge,
    )
    .map_err(merge_stopped)?;
    land(
        git,
        landing,
        repo,
        &ctx.root,
        branch,
        &ours,
        &commit,
        &Over::default(),
    )?;
    report.pulled = landed(Some(&ours), &theirs)?;
    report.auto_merged = auto_merged;
    report.to_push = landed(Some(&theirs), &commit)?;
    drop_waiting(&pending_path, waiting)
}

/// Remove the waiting merge the pull settled.
fn drop_waiting(path: &Path, waiting: Option<Pending>) -> Result<(), LeftAsIs> {
    match waiting {
        Some(_) => pending::remove(path).map_err(failed),
        None => Ok(()),
    }
}

/// The pinned branch's upstream, or why there is none (`no_upstream`, with
/// the command that sets one).
pub fn upstream<'c, G: SyncGit>(ctx: &'c Context<G>) -> Result<&'c Upstream, LeftAsIs> {
    if let Some(upstream) = &ctx.upstream {
        return Ok(upstream);
    }
    let branch = ctx
        .branch()
        .and_then(|b| b.strip_prefix("refs/heads/"))
        .unwrap_or("<branch>");
    Err(LeftAsIs::because(
        LeftReason::NoUpstream,
        format!("set one with git push -u origin {branch}"),
    ))
}

/// The upstream's commit to pull from: the one this fetch's own
/// `ls-remote` named, under the remote claim the caller holds (`_remote`);
/// `None` when the remote has no such branch.
pub fn fetch_upstream<G: SyncGit>(
    ctx: &Context<G>,
    upstream: &Upstream,
    _remote: &RemoteClaim,
) -> Result<Option<String>, LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let deadline = Instant::now() + ctx.request.network_time_limit;
    match fetch::fetch(git, repo, upstream, &ctx.dirs.common, deadline)? {
        Fetch::Moved { new, .. } => Ok(Some(new)),
        Fetch::Same { at } => Ok(Some(at)),
        Fetch::NoBranch => Ok(None),
        Fetch::Offline => Err(offline()),
        Fetch::Refused { detail } => Err(LeftAsIs::because(LeftReason::Refused, detail)),
    }
}

fn offline() -> LeftAsIs {
    LeftAsIs {
        reason: LeftReason::Offline,
        paths: Vec::new(),
        detail: None,
        stopped: false,
    }
}

fn unrelated(upstream: &Upstream) -> LeftAsIs {
    let branch = upstream
        .remote_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&upstream.remote_ref);
    LeftAsIs::because(
        LeftReason::Refused,
        format!(
            "this brain's history and {}'s {branch} have no commit in common, so nothing was merged; check that the remote is this brain's",
            upstream.remote
        ),
    )
}

/// Why the merge hooks stopped the merge: a hook's refusal, or a git
/// (a hook's included) that failed or was stopped at its deadline.
fn merge_stopped(e: MergeHookError) -> LeftAsIs {
    match e {
        MergeHookError::Hook(HookError::Git(e)) => e.into(),
        MergeHookError::Hook(HookError::MessageFile(_)) => failed(e),
        MergeHookError::Changed { .. }
        | MergeHookError::Hook(HookError::Failed { .. } | HookError::EmptyMessage) => {
            LeftAsIs::because(LeftReason::Hook, e.to_string())
        }
    }
}

/// The conflicts of the worktree's waiting merge, with each side's text
/// ([`texts`]), when it was found for the pinned branch's commit now;
/// none when no merge waits, or one waits for a commit the branch has
/// moved on from (it stays as it is).
pub fn waiting<G: SyncGit>(ctx: &Context<G>) -> Result<Vec<ConflictText>, LeftAsIs> {
    match waiting_merge(ctx)? {
        Some(waiting) => Ok(texts(&ctx.git, &ctx.repo, &waiting)?),
        None => Ok(Vec::new()),
    }
}

/// The worktree's waiting merge, when it was found for the pinned branch's
/// commit now; `None` when no merge waits, or one waits for a commit the
/// branch has moved on from (it stays as it is).
pub fn waiting_merge<G: SyncGit>(ctx: &Context<G>) -> Result<Option<Pending>, LeftAsIs> {
    let Some(branch) = ctx.branch() else {
        return Ok(None);
    };
    let Found::Waiting(waiting) = Pending::read(&ctx.pending()) else {
        return Ok(None);
    };
    if ctx.git.ref_value(&ctx.repo, branch)?.as_deref() != Some(waiting.ours.as_str()) {
        return Ok(None);
    }
    Ok(Some(waiting))
}

/// The waiting merge's conflicts with each side's text and the merge's id:
/// `None` for a side with no file, or one that is binary (a NUL byte) or
/// not UTF-8, which marks it `binary`.
pub fn texts<G: Plumbing>(
    git: &G,
    repo: &Repo,
    waiting: &Pending,
) -> Result<Vec<ConflictText>, GitError> {
    let conflicts: &[Conflict] = &waiting.conflicts;
    let merge = waiting.id();
    let oids: Vec<String> = conflicts
        .iter()
        .flat_map(|c| [&c.base, &c.ours, &c.theirs])
        .flatten()
        .cloned()
        .collect();
    let blobs = git.read_blobs(repo, &oids)?;
    Ok(conflicts
        .iter()
        .map(|c| {
            let mut binary = false;
            let mut text = |side: &Option<String>| {
                let bytes = blobs.get(side.as_ref()?)?;
                match std::str::from_utf8(bytes) {
                    Ok(text) if !text.contains('\0') => Some(text.to_string()),
                    _ => {
                        binary = true;
                        None
                    }
                }
            };
            let (base, ours, theirs) = (text(&c.base), text(&c.ours), text(&c.theirs));
            ConflictText {
                path: c.path.clone(),
                kind: c.kind,
                merge: merge.clone(),
                base,
                ours,
                theirs,
                binary,
            }
        })
        .collect())
}

/// How a push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pushed {
    /// The remote took the push.
    Pushed,
    /// The remote turned it back because it moved: pull, then push again.
    RemoteMoved,
    Left(LeftAsIs),
}

/// Push the pinned branch to its upstream under the remote claim (see the
/// module docs).
pub fn push<G: SyncGit>(ctx: &Context<G>) -> Pushed {
    let remote = match ctx.take_remote() {
        Ok(Some(remote)) => remote,
        Ok(None) => {
            return Pushed::Left(LeftAsIs::because(
                LeftReason::Busy,
                "another kluris command is talking to the remote; the commits wait for the next push",
            ));
        }
        Err(left) => return Pushed::Left(left),
    };
    push_claimed(ctx, &remote)
}

/// [`push`] under the remote claim the caller holds (`_remote`).
pub fn push_claimed<G: SyncGit>(ctx: &Context<G>, _remote: &RemoteClaim) -> Pushed {
    match push_head(ctx) {
        Ok(pushed) => pushed,
        Err(left) => Pushed::Left(left),
    }
}

/// Why the push did not go through, as the report says it.
fn push_left(outcome: PushOutcome, remote: &str) -> LeftAsIs {
    match outcome {
        PushOutcome::Unreachable => offline(),
        PushOutcome::Refused(stderr) => LeftAsIs::because(
            LeftReason::Refused,
            fetch::refused_access("push", remote, &stderr),
        ),
        PushOutcome::Hook { url, output } => {
            let to = url.map_or_else(|| remote.to_string(), |url| format!("{remote} {url}"));
            let output = one_line(&output);
            let said = if output.is_empty() {
                String::new()
            } else {
                format!(": {output}")
            };
            LeftAsIs::because(
                LeftReason::Hook,
                format!("the pre-push hook refused the push to {to}{said}"),
            )
        }
        PushOutcome::Failed(detail) => LeftAsIs::because(
            LeftReason::Refused,
            format!("the push was refused: {detail}"),
        ),
        PushOutcome::Pushed | PushOutcome::RemoteMoved => {
            failed("the push ended without a refusal to report")
        }
    }
}

fn push_head<G: SyncGit>(ctx: &Context<G>) -> Result<Pushed, LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let upstream = upstream(ctx)?;
    // Checked right before: the ref git moves once the remote took the
    // push.
    if let Some(why) = fetch::unmovable(git, repo, upstream)? {
        return Err(LeftAsIs::because(LeftReason::Refused, why));
    }
    // git pushes the pinned branch by its ref: it must have a commit.
    ctx.branch_commit(&upstream.branch)?;
    let deadline = Instant::now() + ctx.request.network_time_limit;
    match git.push(repo, upstream, deadline) {
        PushOutcome::Pushed => Ok(Pushed::Pushed),
        PushOutcome::RemoteMoved => Ok(Pushed::RemoteMoved),
        outcome => Err(push_left(outcome, &upstream.remote)),
    }
}

/// Pull, then push what is committed ([`share`]).
pub fn sync<G: SyncGit>(ctx: &Context<G>) -> SyncReport {
    share(ctx, None, pull(ctx))
}

/// Push what `pulled`, a pull's report, left to push, unless it left the
/// brain as it is or found a conflict. A push the remote turned back
/// because it moved pulls again, into the work tree whose claim the caller
/// holds (`tree`; without one the pull takes it), then pushes again,
/// [`PUSH_ATTEMPTS`] pushes at most.
pub fn share<G: SyncGit>(
    ctx: &Context<G>,
    tree: Option<&TreeClaim>,
    pulled: SyncReport,
) -> SyncReport {
    let mut total = pulled;
    for attempt in 1..=PUSH_ATTEMPTS {
        if total.left_as_is.is_some() || !total.conflicts.is_empty() || total.to_push == 0 {
            return total;
        }
        let pushed = push(ctx);
        if let Some(count) = to_push(ctx) {
            total.to_push = count;
        }
        match pushed {
            Pushed::Pushed => {
                total.pushed = true;
                return total;
            }
            Pushed::Left(left) => {
                total.left_as_is = Some(left);
                return total;
            }
            Pushed::RemoteMoved if attempt < PUSH_ATTEMPTS => {
                let again = match tree {
                    Some(tree) => pull_claimed(ctx, tree),
                    None => pull(ctx),
                };
                absorb(&mut total, again);
            }
            Pushed::RemoteMoved => {}
        }
    }
    total.left_as_is = Some(LeftAsIs::because(
        LeftReason::Refused,
        format!(
            "the remote kept moving: {PUSH_ATTEMPTS} pushes were rejected; run kluris sync again"
        ),
    ));
    total
}

/// `total` with a later pull's `report`: the commits and paths it brought
/// added, the rest as it says now.
pub fn absorb(total: &mut SyncReport, report: SyncReport) {
    total.pulled += report.pulled;
    for path in report.auto_merged {
        if !total.auto_merged.contains(&path) {
            total.auto_merged.push(path);
        }
    }
    total.left_as_is = report.left_as_is;
    total.conflicts = report.conflicts;
    total.behind_conflict = report.behind_conflict;
    total.to_push = report.to_push;
}

/// What the pinned branch has that the remote does not, read now: the
/// commits from the upstream's remote-tracking ref (which git moves to what
/// the remote took; every commit while there is none) to the pinned
/// branch's ref, never `HEAD`, so one made while a push ran (by a
/// `pre-push` hook, say) is still to push. `None` without an upstream, or
/// when it cannot be read.
pub fn to_push<G: SyncGit>(ctx: &Context<G>) -> Option<usize> {
    let (git, repo) = (&ctx.landing, &ctx.repo);
    let upstream = ctx.upstream.as_ref()?;
    let tip = git.ref_value(repo, &upstream.branch).ok()??;
    let remote = git.ref_value(repo, &upstream.refname).ok()?;
    git.count(repo, remote.as_deref(), &tip).ok()
}

/// How a waiting merge is finished ([`finish_merge`]).
#[derive(Debug, Clone, Copy)]
pub struct Finish<'f> {
    /// A choice for every conflicted path.
    pub choices: &'f BTreeMap<String, ResolvedChoice>,
    /// The merge commit's message, its trailers included.
    pub message: &'f str,
    /// The hooks the merge commit runs: `git merge`'s, or `git commit`'s
    /// concluding a merge (`post-commit` once it landed).
    pub hooks: MergeHooks,
    /// The conflicted paths settled in the work tree, each with what its
    /// file holds: never in the way while it still does, and in the index
    /// as the merge has them when it lands ([`land`]). Empty when the
    /// choices were made apart from the work tree.
    pub settled: &'f Settled,
    /// A generated file (`map.md`, `brain.md`) with changes of its own in
    /// the way of the land, a map `kluris dream` rewrote say, keeps its
    /// work-tree copy, which stays changed, and the merge's own is
    /// committed; otherwise it is in the way. One the merge removes is
    /// always in the way.
    pub keep_generated: bool,
}

/// Finish the waiting merge with a person's choices: merge `ours` with the
/// pinned `theirs`, check the files in the way, make the merge commit with
/// its message through its hooks, and land it; its commit. The caller holds
/// the tree claim (`_claim`) from reading the waiting merge to removing it,
/// so the merge and the record it answers stay one.
pub fn finish_merge<G: SyncGit>(
    ctx: &Context<G>,
    _claim: &TreeClaim,
    pending: &Pending,
    finish: &Finish,
) -> Result<String, LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let branch = ctx.pinned()?;
    let (ours, theirs) = (&pending.ours, &pending.theirs);
    unchanged(git, repo, branch, ours)?;
    let maps = &ctx.request.maps;
    let bases = git.merge_bases(repo, ours, theirs)?;
    let MergeOutcome::Clean { tree, index, .. } =
        merge_commits(git, repo, ours, theirs, &bases, maps, finish.choices)?
    else {
        return Err(failed(
            "the merge has conflicts the recorded choices do not cover",
        ));
    };
    let dirty = git.dirty(repo)?;
    let mut over = Over {
        settled: finish.settled.clone(),
        kept: Default::default(),
    };
    let blocked = blocked_paths(git, repo, &ctx.root, &dirty, ours, &tree)?;
    let mut blocked = over.without(&ctx.root, blocked);
    if finish.keep_generated {
        let generated: Vec<String> = blocked
            .iter()
            .filter(|rel| is_generated(rel))
            .cloned()
            .collect();
        over.kept = git.tracked(repo, &tree, &generated)?;
        blocked.retain(|rel| !over.kept.contains(rel));
    }
    if !blocked.is_empty() {
        return Err(uncommitted(blocked));
    }
    let no_verify = ctx.request.no_verify;
    let commit = merge_commit(
        git,
        repo,
        &index,
        &tree,
        [ours, theirs],
        finish.message,
        no_verify,
        finish.hooks,
    )
    .map_err(merge_stopped)?;
    let landing = &ctx.landing;
    land(git, landing, repo, &ctx.root, branch, ours, &commit, &over)?;
    if finish.hooks == MergeHooks::Commit {
        // As with `git commit`, post-commit cannot undo or fail the commit.
        let _ = landing.run_hook(repo, None, "post-commit", &[]);
    }
    Ok(commit)
}

fn count_json(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn opt(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().cloned().map(Json::Str).collect())
}

impl ConflictText {
    /// `{path, kind, merge, ours, theirs, base, binary}`.
    pub fn to_json(&self) -> Json {
        Json::Object(vec![
            ("path".into(), Json::Str(self.path.clone())),
            ("kind".into(), Json::Str(self.kind.as_str().into())),
            ("merge".into(), Json::Str(self.merge.clone())),
            ("ours".into(), opt(&self.ours)),
            ("theirs".into(), opt(&self.theirs)),
            ("base".into(), opt(&self.base)),
            ("binary".into(), Json::Bool(self.binary)),
        ])
    }
}

impl SyncReport {
    /// The `sync` object every command prints: all eight keys, `left_as_is`
    /// with a `null` reason when nothing was left, and `last_push` `null`
    /// when there is no background push to report.
    pub fn to_json(&self) -> Json {
        let (reason, paths, detail) = match &self.left_as_is {
            Some(left) => (
                Json::Str(left.reason.as_str().into()),
                strings(&left.paths),
                opt(&left.detail),
            ),
            None => (Json::Null, Json::Array(Vec::new()), Json::Null),
        };
        Json::Object(vec![
            ("pulled".into(), count_json(self.pulled)),
            ("auto_merged".into(), strings(&self.auto_merged)),
            (
                "left_as_is".into(),
                Json::Object(vec![
                    ("reason".into(), reason),
                    ("paths".into(), paths),
                    ("detail".into(), detail),
                ]),
            ),
            (
                "conflicts".into(),
                Json::Array(self.conflicts.iter().map(ConflictText::to_json).collect()),
            ),
            ("behind_conflict".into(), count_json(self.behind_conflict)),
            ("to_push".into(), count_json(self.to_push)),
            ("pushed".into(), Json::Bool(self.pushed)),
            (
                "last_push".into(),
                self.last_push
                    .as_ref()
                    .map_or(Json::Null, LastPush::to_json),
            ),
        ])
    }
}
