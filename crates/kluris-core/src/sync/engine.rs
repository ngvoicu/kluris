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
//!
//! [`sync`] pulls again after a push the remote turned back because it
//! moved, [`PUSH_ATTEMPTS`] times at most. Only the fetch and the push are
//! bounded in time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::claim::{ClaimError, RemoteClaim, TreeClaim};
use super::fetch::{self, Fetch};
use super::git::{GitDirs, PushOutcome, SyncGit, Upstream, one_line};
use super::hooks::{HookError, MergeHookError, merge_commit};
use super::land::{blocked_paths, failed, land, unchanged, uncommitted};
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
    git: G,
    repo: Repo,
    dirs: GitDirs,
    /// `refs/heads/<name>`; `None` on a detached HEAD.
    branch: Option<String>,
    upstream: Option<Upstream>,
    request: SyncRequest<'a>,
}

impl<'a, G: SyncGit> Context<'a, G> {
    pub fn git(&self) -> &G {
        &self.git
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
            .ref_value(&self.repo, branch)
            .map_err(failed)?
            .ok_or_else(|| failed("the branch has no commit yet: commit first"))
    }

    /// The remote claim; `None` while another command holds it.
    fn take_remote(&self) -> Result<Option<RemoteClaim>, LeftAsIs> {
        match RemoteClaim::try_take(&self.dirs.common) {
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
    let repo = git.repo(root).ok_or(SyncError::NotARepo)?;
    if repo.nested() {
        return Err(SyncError::Nested {
            prefix: repo.prefix.clone(),
        });
    }
    // What the hooks of the gits it starts print reaches the user.
    let git = git.for_remote(&repo)?.showing_output();
    let dirs = git.git_dirs(&repo)?;
    let branch = git.branch(&repo)?;
    let upstream = match &branch {
        Some(branch) => git.upstream(&repo, branch)?,
        None => None,
    };
    Ok(Context {
        root: root.to_path_buf(),
        git,
        repo,
        dirs,
        branch,
        upstream,
        request: *request,
    })
}

/// Bring the upstream's commits into the brain (see the module docs).
pub fn pull<G: SyncGit>(ctx: &Context<G>) -> SyncReport {
    let mut report = SyncReport::default();
    if let Err(left) = pull_into(ctx, &mut report) {
        report.left_as_is = Some(left);
    }
    report
}

fn pull_into<G: SyncGit>(ctx: &Context<G>, report: &mut SyncReport) -> Result<(), LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    if let Some(op) = git.in_progress(repo).map_err(failed)? {
        return Err(LeftAsIs::because(
            LeftReason::InProgress,
            format!("a {op} is in progress; finish or abort it first"),
        ));
    }
    let upstream = upstream(ctx)?;
    let theirs = fetched(ctx, upstream)?;
    let _tree = ctx.claim_tree()?;
    let branch = upstream.branch.as_str();
    let ours = ctx.branch_commit(branch)?;
    unchanged(git, repo, branch, &ours)?;
    let count = |from: Option<&str>, to: &str| git.count(repo, from, to).map_err(failed);
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
    let behind = !git.is_ancestor(repo, &theirs, &ours).map_err(failed)?;
    let ahead = !git.is_ancestor(repo, &ours, &theirs).map_err(failed)?;
    if !behind {
        report.to_push = count(Some(&theirs), &ours)?;
        return drop_waiting(&pending_path, waiting);
    }
    if !ahead {
        land(git, repo, &ctx.root, branch, &ours, &theirs)?;
        report.pulled = count(Some(&ours), &theirs)?;
        return drop_waiting(&pending_path, waiting);
    }

    report.to_push = count(Some(&theirs), &ours)?;
    if git
        .merge_bases(repo, &ours, &theirs)
        .map_err(failed)?
        .is_empty()
    {
        return Err(unrelated(upstream));
    }
    if let Some(waiting) = waiting.as_ref().filter(|w| w.theirs == theirs) {
        report.conflicts = texts(git, repo, waiting).map_err(failed)?;
        return Ok(());
    }
    let none = BTreeMap::new();
    let maps = &ctx.request.maps;
    let (tree, index, auto_merged) =
        match merge_commits(git, repo, &ours, &theirs, maps, &none).map_err(failed)? {
            MergeOutcome::Clean {
                tree,
                index,
                auto_merged,
            } => (tree, index, auto_merged),
            MergeOutcome::Conflicts { base, conflicts } => {
                match waiting {
                    Some(waiting) => {
                        report.conflicts = texts(git, repo, &waiting).map_err(failed)?;
                        report.behind_conflict = count(Some(&waiting.theirs), &theirs)?;
                    }
                    None => {
                        let waiting = Pending::new(base, &ours, &theirs, conflicts);
                        report.conflicts = texts(git, repo, &waiting).map_err(failed)?;
                        waiting
                            .write(&pending_path)
                            .map_err(|e| failed(format!("cannot record the conflict: {e}")))?;
                    }
                }
                return Ok(());
            }
        };
    let dirty = git.dirty(repo).map_err(failed)?;
    let blocked = blocked_paths(git, repo, &ctx.root, &dirty, &ours, &tree).map_err(failed)?;
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
    )
    .map_err(merge_stopped)?;
    land(git, repo, &ctx.root, branch, &ours, &commit)?;
    report.pulled = count(Some(&ours), &theirs)?;
    report.auto_merged = auto_merged;
    report.to_push = count(Some(&theirs), &commit)?;
    drop_waiting(&pending_path, waiting)
}

/// Remove the waiting merge the pull settled.
fn drop_waiting(path: &Path, waiting: Option<Pending>) -> Result<(), LeftAsIs> {
    match waiting {
        Some(_) => pending::remove(path).map_err(failed),
        None => Ok(()),
    }
}

/// The pinned branch's upstream, or why there is none.
fn upstream<'c, G: SyncGit>(ctx: &'c Context<G>) -> Result<&'c Upstream, LeftAsIs> {
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
/// `ls-remote` named, under the remote claim; `None` when the remote has
/// no such branch. While another command holds the remote claim the
/// upstream's commit is not known, and the tracking ref, which any git can
/// rewrite, is never taken for it.
fn fetched<G: SyncGit>(ctx: &Context<G>, upstream: &Upstream) -> Result<Option<String>, LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let Some(_remote) = ctx.take_remote()? else {
        return Err(LeftAsIs::because(
            LeftReason::Busy,
            "another kluris command is fetching this brain's remote, so the upstream's commit is not known; nothing was pulled: run again",
        ));
    };
    let deadline = Instant::now() + ctx.request.network_time_limit;
    match fetch::fetch(git, repo, upstream, deadline).map_err(failed)? {
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

/// Why the merge hooks stopped the merge.
fn merge_stopped(e: MergeHookError) -> LeftAsIs {
    let reason = match &e {
        MergeHookError::Changed { .. }
        | MergeHookError::Hook(HookError::Failed { .. } | HookError::EmptyMessage) => {
            LeftReason::Hook
        }
        MergeHookError::Hook(HookError::MessageFile(_) | HookError::Git(_)) => LeftReason::Failed,
    };
    LeftAsIs::because(reason, e.to_string())
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
    let Some(_remote) = ctx.take_remote()? else {
        return Err(LeftAsIs::because(
            LeftReason::Busy,
            "another kluris command is talking to the remote; the commits wait for the next push",
        ));
    };
    // Checked right before: the ref git moves once the remote took the
    // push.
    if let Some(why) = fetch::unmovable(git, repo, upstream).map_err(failed)? {
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

/// Pull, then push what is committed; a push the remote turned back because
/// it moved pulls again, [`PUSH_ATTEMPTS`] times at most.
pub fn sync<G: SyncGit>(ctx: &Context<G>) -> SyncReport {
    let mut total = SyncReport::default();
    for _ in 0..PUSH_ATTEMPTS {
        let report = pull(ctx);
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
        if total.left_as_is.is_some() || !total.conflicts.is_empty() || total.to_push == 0 {
            return total;
        }
        match push(ctx) {
            Pushed::Pushed => {
                total.pushed = true;
                total.to_push = 0;
                return total;
            }
            Pushed::RemoteMoved => {}
            Pushed::Left(left) => {
                total.left_as_is = Some(left);
                return total;
            }
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

/// Finish the waiting merge with a person's `choices`: merge `ours` with
/// the pinned `theirs`, check the files in the way, make the merge commit
/// with `message` through the merge hooks, and land it; its commit. The
/// caller holds the tree claim (`_claim`) from reading the waiting merge to
/// removing it, so the merge and the record it answers stay one.
pub fn finish_merge<G: SyncGit>(
    ctx: &Context<G>,
    _claim: &TreeClaim,
    pending: &Pending,
    choices: &BTreeMap<String, ResolvedChoice>,
    message: &str,
) -> Result<String, LeftAsIs> {
    let (git, repo) = (&ctx.git, &ctx.repo);
    let branch = ctx.pinned()?;
    let (ours, theirs) = (&pending.ours, &pending.theirs);
    unchanged(git, repo, branch, ours)?;
    let maps = &ctx.request.maps;
    let MergeOutcome::Clean { tree, index, .. } =
        merge_commits(git, repo, ours, theirs, maps, choices).map_err(failed)?
    else {
        return Err(failed(
            "the merge has conflicts the recorded choices do not cover",
        ));
    };
    let dirty = git.dirty(repo).map_err(failed)?;
    let blocked = blocked_paths(git, repo, &ctx.root, &dirty, ours, &tree).map_err(failed)?;
    if !blocked.is_empty() {
        return Err(uncommitted(blocked));
    }
    let no_verify = ctx.request.no_verify;
    let commit = merge_commit(git, repo, &index, &tree, [ours, theirs], message, no_verify)
        .map_err(merge_stopped)?;
    land(git, repo, &ctx.root, branch, ours, &commit)?;
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
    /// (every push here is the command's own).
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
            ("last_push".into(), Json::Null),
        ])
    }
}
