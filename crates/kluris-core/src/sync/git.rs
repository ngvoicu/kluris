//! The git layer of the sync engine: [`SyncGit`] (the steps beyond
//! [`Plumbing`]) and its [`SystemGit`].
//!
//! - **One upstream query:** [`SyncGit::upstream`], the current branch's
//!   remote, remote-tracking ref and branch on the remote ([`Upstream`]).
//! - **One remote environment:** [`SyncGit::for_remote`] gives a git whose
//!   every run has [`remote_env`] (no terminal prompt, no askpass, no
//!   interactive credential manager, ssh in batch mode unless the user chose
//!   an ssh command) and [`NO_BACKGROUND`]. The engine takes it once per
//!   brain, after the local queries that open it (git's version, the
//!   repository, `core.sshCommand`), which talk to no remote; no git it
//!   starts after them can prompt: not a fetch or a push, and not a merge
//!   or a checkout whose lazy fetch (a partial clone) talks to the remote.
//!   The steps that talk to a remote take it themselves when given a git
//!   without it.
//! - **stdin closed** for every git ([`SystemGit`]'s runs), so a hook that
//!   reads it never waits.
//! - **git's own push:** [`SyncGit::push`] runs `git push --porcelain
//!   --no-recurse-submodules <remote> refs/heads/<branch>:<remote branch>`,
//!   the pinned branch named by its ref, never `HEAD`. git goes to each
//!   push URL, runs the `pre-push` hooks for each as for any push of yours
//!   (Kluris's `--no-verify` never skips them), and moves the
//!   remote-tracking ref after a push the remote took, following a symbolic
//!   ref: the caller checks that ref right before
//!   ([`super::fetch::unmovable`]). The push stops at its deadline with
//!   every process it started that stays in its process group (Unix) or
//!   job (Windows), its hooks among them.
//! - **What git says, not what a hook prints:** a hook's stdout reaches
//!   git's own, so the push's outcome is read from git's trace2 event
//!   stream (`GIT_TRACE2_EVENT`, a temporary file) for which `pre-push`
//!   runs refused, and from its porcelain report only where no hook can
//!   have printed it ([`SystemGit::push_failure`]).

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::merge::TextMerger;
use crate::git::{GitError, Plumbing, Repo, SystemGit, TreeEntry, unique_name};
use crate::process::{Finished, RunError, run_until};

/// Where a local branch pulls from and pushes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// The local branch (`refs/heads/main`).
    pub branch: String,
    /// The remote (`%(upstream:remotename)`, `origin`).
    pub remote: String,
    /// The remote-tracking ref (`refs/remotes/origin/main`).
    pub refname: String,
    /// The branch on the remote (`branch.<name>.merge`, `refs/heads/main`).
    pub remote_ref: String,
}

/// A worktree's two git directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDirs {
    /// The worktree's own (`git rev-parse --absolute-git-dir`): its index,
    /// its HEAD, and what belongs to its files.
    pub own: PathBuf,
    /// The repository's, which every linked worktree shares (`git rev-parse
    /// --git-common-dir`): its objects, refs and remotes.
    pub common: PathBuf,
}

/// `git merge-tree --write-tree` of two commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeTree {
    /// The merged tree; conflicted paths hold git's conflict markers.
    pub tree: String,
    /// The paths git could not merge, brain-relative.
    pub conflicted: BTreeSet<String>,
    /// What each side has at a conflicted path, as git's index stages say
    /// (after rename detection): `[base, ours, theirs]`.
    pub stages: BTreeMap<String, [Option<TreeEntry>; 3]>,
    /// git's conflict messages: the paths each concerns and its type
    /// (`CONFLICT (directory rename suggested)`, ...).
    pub messages: Vec<(Vec<String>, String)>,
}

/// `git merge-tree --write-tree -z` output: the tree; then, with conflicts,
/// `<mode> <oid> <stage>\t<path>` records, an empty field, and messages of
/// `<count>`, that many paths, a type and a text.
fn parse_merge_tree(out: &[u8]) -> MergeTree {
    let mut fields = out
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let tree = fields.next().unwrap_or_default();
    let mut stages: BTreeMap<String, [Option<TreeEntry>; 3]> = BTreeMap::new();
    for field in fields.by_ref() {
        if field.is_empty() {
            break;
        }
        let Some((head, path)) = field.split_once('\t') else {
            continue;
        };
        let mut parts = head.split(' ');
        let (Some(mode), Some(oid), Some(stage)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(slot) = stage
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .filter(|n| *n < 3)
        else {
            continue;
        };
        stages.entry(path.to_string()).or_default()[slot] = Some(TreeEntry {
            rel: path.to_string(),
            mode: mode.to_string(),
            oid: oid.to_string(),
        });
    }
    let mut messages = Vec::new();
    while let Some(count) = fields.next() {
        let Ok(count) = count.parse::<usize>() else {
            break;
        };
        let paths: Vec<String> = fields.by_ref().take(count).collect();
        let kind = fields.next().unwrap_or_default();
        let _text = fields.next();
        messages.push((paths, kind));
    }
    MergeTree {
        tree,
        conflicted: stages.keys().cloned().collect(),
        stages,
        messages,
    }
}

/// How a push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    /// Rejected because the remote branch moved: fetch, merge, push again.
    RemoteMoved,
    /// The remote could not be reached ([`network_failure`]) or not in time.
    Unreachable,
    /// git could not talk to the remote for another reason (a key or token
    /// turned away, no such repository, no write access): git's stderr.
    Refused(String),
    /// A `pre-push` hook refused, as git's trace2 stream records it: the
    /// URL git ran it for, and what the hooks printed.
    Hook {
        url: Option<String>,
        output: String,
    },
    /// Refused for another reason (the remote's rules, its own hooks), or
    /// for one that cannot be told apart: all git printed.
    Failed(String),
}

/// A git step bounded by a deadline that did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// The deadline passed and git was stopped.
    TimedOut,
    /// git could not run or failed.
    Failed(GitError),
}

/// The git steps of the sync engine (see the module docs).
pub trait SyncGit: Plumbing + TextMerger + Clone + Sized {
    /// This git, forwarding to this process's stderr what each of its runs
    /// that succeeds printed on stderr (a hook's output, which git sends
    /// there): the engine's gits, so no hook output is lost. A run that
    /// fails keeps its stderr for the error.
    fn showing_output(self) -> Self;

    /// This git with every run stopped, with every process it started, at
    /// `deadline` (its error [`GitError::stopped`]); `None`: never. The
    /// fast-forward ([`SyncGit::fast_forward_to`]) is never stopped.
    fn with_deadline(self, deadline: Option<Instant>) -> Self;

    /// The work tree `dir` is in, as git answers it (`git rev-parse
    /// --is-inside-work-tree --show-prefix`): `None` outside one or inside a
    /// `.git` directory; an error when git was stopped, could not start or
    /// was killed.
    fn discover(&self, dir: &Path) -> Result<Option<Repo>, GitError>;

    /// This git with the remote environment for `repo`: [`remote_env`]
    /// (`core.sshCommand` read once, here) and [`NO_BACKGROUND`] on every
    /// run. A git that has it already is given back as it is.
    fn for_remote(&self, repo: &Repo) -> Result<Self, GitError>;

    /// The branch HEAD names (`refs/heads/main`); `None` on a detached HEAD.
    fn branch(&self, repo: &Repo) -> Result<Option<String>, GitError>;

    /// The upstream on a remote of the local `branch` (`refs/heads/main`);
    /// `None` without one, or with a local branch as upstream.
    fn upstream(&self, repo: &Repo, branch: &str) -> Result<Option<Upstream>, GitError>;

    /// The worktree's own git directory and the repository's common one.
    fn git_dirs(&self, repo: &Repo) -> Result<GitDirs, GitError>;

    /// Whether the repository is a shallow clone (`git rev-parse
    /// --is-shallow-repository`).
    fn is_shallow(&self, repo: &Repo) -> Result<bool, GitError>;

    /// How many commits `to` has that `from` has not (`git rev-list
    /// --count`); every commit of `to` when `from` is `None`.
    fn count(&self, repo: &Repo, from: Option<&str>, to: &str) -> Result<usize, GitError>;

    /// The commit the upstream's branch is at, from this git's own `git
    /// ls-remote <remote> <branch>`, by `deadline`; `None` when the remote
    /// has no such branch. `ls-remote <remote>` takes the remote's own
    /// settings: its first URL after `insteadOf`, its `uploadpack`, and the
    /// http and ssh config.
    fn ls_remote(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        deadline: Instant,
    ) -> Result<Option<String>, RemoteError>;

    /// Fetch the objects of the upstream's branch by `deadline` and write
    /// nothing else: no ref, `FETCH_HEAD`, tag, `packed-refs`, commit-graph
    /// or bundle refs, so a fetch stopped at the deadline holds no ref lock
    /// (a terminated job on Windows removes none).
    fn fetch_branch(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        deadline: Instant,
    ) -> Result<(), RemoteError>;

    /// Whether `commit` is here as a commit, checked locally by `deadline`:
    /// lazy fetching off (`GIT_NO_LAZY_FETCH`, git 2.45 and newer), so a
    /// partial clone reports a missing commit absent instead of fetching it.
    fn has_commit(&self, repo: &Repo, commit: &str, deadline: Instant)
    -> Result<bool, RemoteError>;

    /// The URL a fetch from `remote` goes to, after `insteadOf`, one per
    /// line of git's answer (`git ls-remote --get-url`). Read from config
    /// alone: no network.
    fn remote_urls(&self, repo: &Repo, remote: &str) -> Result<Vec<String>, GitError>;

    /// The names of the repository's remotes (`git remote`), from config
    /// alone.
    fn remotes(&self, repo: &Repo) -> Result<Vec<String>, GitError>;

    /// The object a ref holds; `None` when there is no such ref.
    fn ref_value(&self, repo: &Repo, refname: &str) -> Result<Option<String>, GitError>;

    /// The ref a symbolic ref points to (`git symbolic-ref --quiet`);
    /// `None` when `refname` is a plain ref or there is no such ref.
    fn symbolic_target(&self, repo: &Repo, refname: &str) -> Result<Option<String>, GitError>;

    /// `git update-ref --no-deref -m <message> <refname> <new> <old>`: move
    /// a ref only while it still holds `old` (`None`: only while there is
    /// no such ref). A symbolic ref is replaced, never followed, so the
    /// update cannot reach the ref it points to. Local and quick: no
    /// deadline.
    fn move_ref(
        &self,
        repo: &Repo,
        refname: &str,
        new: &str,
        old: Option<&str>,
        message: &str,
    ) -> Result<(), GitError>;

    /// Whether `ancestor` is an ancestor of (or is) `descendant`.
    fn is_ancestor(&self, repo: &Repo, ancestor: &str, descendant: &str) -> Result<bool, GitError>;

    /// `git merge-base --all`: every best common ancestor, git's order.
    fn merge_bases(&self, repo: &Repo, a: &str, b: &str) -> Result<Vec<String>, GitError>;

    /// `git merge-tree --write-tree` (git >= 2.38): the merge, in memory.
    fn merge_tree(&self, repo: &Repo, ours: &str, theirs: &str) -> Result<MergeTree, GitError>;

    /// The bytes checkout writes at `path` for `blob` (`git cat-file
    /// --filters --path`): the blob through the path's smudge filter and
    /// line-ending conversion (`core.autocrlf`, the `eol` attribute).
    fn checked_out(&self, repo: &Repo, path: &str, blob: &str) -> Result<Vec<u8>, GitError>;

    /// The paths that differ between two trees (or commits), brain-relative.
    fn diff_names(&self, repo: &Repo, from: &str, to: &str) -> Result<Vec<String>, GitError>;

    /// Move the current branch, the index and the work tree to `to`, a
    /// descendant of `HEAD` (`git merge --ff-only --no-overwrite-ignore`,
    /// autostash off): git refuses if `HEAD` moved meanwhile or a local
    /// change, an untracked or an ignored file would be overwritten, and
    /// runs the `post-merge` hook. git's output and the hook's go to this
    /// process's stderr; when git fails, its message is the error's detail.
    /// Never stopped, whatever this git's deadline.
    fn fast_forward_to(&self, repo: &Repo, to: &str) -> Result<(), GitError>;

    /// Push the upstream's local branch, named by its ref, to its branch
    /// on the remote with git's own push (see the module docs), never
    /// prompting, by `deadline` (`Unreachable` once it passed). What a hook
    /// printed and git's stderr go to this process's stderr.
    fn push(&self, repo: &Repo, upstream: &Upstream, deadline: Instant) -> PushOutcome;
}

/// A fetch git stopped because a ref is locked: the lock file git named,
/// or the ref whose lock is to be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefLock {
    File(String),
    Ref(String),
}

/// Whether git's stderr says a fetch failed on a ref lock: `Unable to
/// create '<lock>': File exists`, `cannot lock ref '<ref>'`, or `fetching
/// ref <ref> failed: reference already exists` (Git for Windows after a
/// fetch was terminated while it held the ref).
pub(crate) fn ref_lock(stderr: &str) -> Option<RefLock> {
    let between = |text: &str, start: &str, end: &str| -> Option<String> {
        let rest = &text[text.find(start)? + start.len()..];
        Some(rest[..rest.find(end)?].to_string())
    };
    if let Some(path) = between(stderr, "Unable to create '", "': File exists") {
        return Some(RefLock::File(path));
    }
    if let Some(name) = between(stderr, "cannot lock ref '", "'") {
        return Some(RefLock::Ref(name));
    }
    between(stderr, "fetching ref ", " failed: reference already exists").map(RefLock::Ref)
}

/// Whether git's stderr says the network is in the way, in a form git, or
/// the ssh or curl it runs, prints it, each from its source:
///
/// - curl's reason after git's `unable to access '<url>': `
///   (`remote-curl.c`), the text after the last `': ` on that line, or
///   after git's `RPC failed; curl <n> ` when an http transfer failed once
///   it had started (`remote-curl.c`; with an `HTTP <code>` before `curl`
///   the server answered, so it is never this form), when it holds `Could
///   not resolve host` or `Could not resolve proxy` (`lib/hostip.c`, curl
///   8.11), `Failed to connect to ` (`lib/connect.c`), `timed out`
///   (`Resolving timed out after`, `Connection timed out after`, `Operation
///   timed out after` from `lib/multi.c`, `name lookup timed out` from
///   `lib/hostip.c`), `Operation too slow` (`lib/progress.c`, under
///   `http.lowSpeedLimit`), `Timeout was reached` (`lib/strerror.c`), or
///   the C library's words for the errors themselves, `Connection refused`,
///   `Network is unreachable`, `No route to host`;
/// - OpenSSH's `ssh: connect to host <host> port <port>: <strerror>`
///   (`sshconnect.c`, `ssh_connect_direct`, 9.9p1) and `ssh: Could not
///   resolve hostname <host>: <reason>` (`ssh.c`);
/// - git's own `unable to look up <host> (port <port>) (<reason>)` and
///   `unable to connect to <host>:` for `git://` (`connect.c`,
///   `git_tcp_connect_sock`, 2.55).
///
/// A remote git reaches without a network ([`local_url`]: a path or a
/// `file://` URL) is never offline: with any of `urls` local, or none
/// known, this is false before any diagnostic is read.
///
/// Each line is read from its start, after git's `fatal: ` or `error: `:
/// a line the remote sent (`remote: ...`) is never one of them, and a
/// quoted URL or path is never read for the words, so a remote or a path
/// named `timed out.git` stays what git says it is. No line after a
/// `fatal: ` or `error: ` line that quotes a field (a `'` at the start of a
/// word) is read: the field may hold `'` and newlines, so where it ends
/// cannot be told, and a line that reads like ssh's may be the field's.
/// Anything else (`Permission denied (publickey).`, `Authentication
/// failed`, `does not appear to be a git repository`, `Repository not
/// found`, and git's `Could not read from remote repository.` after any of
/// them) is not network trouble. git translates its own messages (`unable
/// to access`, `unable to look up`, `unable to connect to`); under another
/// language those read as a refusal, shown rather than hidden.
pub(crate) fn network_failure(urls: &[String], stderr: &str) -> bool {
    if urls.is_empty() || urls.iter().any(|url| local_url(url)) {
        return false;
    }
    for line in stderr.lines() {
        let line = line.trim();
        let message = ["fatal: ", "error: "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix));
        if network_line(message.unwrap_or(line)) {
            return true;
        }
        if message.is_some_and(quotes_a_field) {
            return false;
        }
    }
    false
}

/// Whether git reaches `url` without a network, as git tells it
/// (`transport.c`, `transport_get`; `connect.c`, `url_is_local_not_ssh`,
/// 2.55): a `file://` URL, or a path: neither `<helper>::` nor
/// `<scheme>://` before it, and no `:`, or a `/` before the first one
/// (else it is ssh's `host:path`), or on Windows a drive letter (`C:`).
fn local_url(url: &str) -> bool {
    let scheme = url
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_alphanumeric() || (i > 0 && "+-.".contains(c)))
        .count();
    let after = &url[scheme..];
    if scheme > 0 && after.starts_with("::") {
        return false;
    }
    if url.starts_with("file://") {
        return true;
    }
    if scheme > 0 && after.starts_with("://") {
        return false;
    }
    let Some(colon) = url.find(':') else {
        return true;
    };
    let drive = colon == 1 && url.as_bytes()[0].is_ascii_alphabetic();
    url.find('/').is_some_and(|slash| slash < colon) || (cfg!(windows) && drive)
}

/// Whether git's message quotes a field: a `'` at the start of a word.
fn quotes_a_field(message: &str) -> bool {
    message.split(' ').any(|word| word.starts_with('\''))
}

/// One message of [`network_failure`], without git's `fatal: ` or `error: `.
fn network_line(line: &str) -> bool {
    const BY_THEMSELVES: [&str; 4] = [
        "ssh: connect to host ",
        "ssh: Could not resolve hostname ",
        "unable to look up ",
        "unable to connect to ",
    ];
    if BY_THEMSELVES.iter().any(|form| line.starts_with(form)) {
        return true;
    }
    if let Some(rest) = line.strip_prefix("RPC failed; curl ") {
        let reason = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        return reason.len() < rest.len() && reason.strip_prefix(' ').is_some_and(curl_reason);
    }
    let Some(rest) = line.strip_prefix("unable to access '") else {
        return false;
    };
    rest.rsplit_once("': ")
        .is_some_and(|(_url, reason)| curl_reason(reason))
}

/// Whether curl's reason, as git prints it, says the network is in the way
/// (see [`network_failure`]).
fn curl_reason(reason: &str) -> bool {
    const CURL_REASONS: [&str; 9] = [
        "Could not resolve host",
        "Could not resolve proxy",
        "Failed to connect to ",
        "Connection refused",
        "Network is unreachable",
        "No route to host",
        "timed out",
        "Operation too slow",
        "Timeout was reached",
    ];
    CURL_REASONS.iter().any(|words| reason.contains(words))
}

/// git's stderr on one line: its lines trimmed, the empty ones dropped,
/// joined by a space; without a final full stop.
pub(crate) fn one_line(stderr: &str) -> String {
    let joined = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined.trim_end_matches('.').to_string()
}

/// The commit `git ls-remote` lists for exactly `remote_ref` (its lines
/// are `<oid>\t<ref>`; a pattern also matches longer names that end in
/// it); `None` when there is none.
fn listed(out: &[u8], remote_ref: &str) -> Option<String> {
    String::from_utf8_lossy(out).lines().find_map(|line| {
        let (oid, name) = line.split_once('\t')?;
        (name == remote_ref && !oid.is_empty()).then(|| oid.to_string())
    })
}

/// What `git push --porcelain` printed on stdout before its own reports:
/// anything a hook printed there. A report is found by its structure, at
/// the end: a `To <destination>` line, its ref lines (a flag, a tab), and
/// `Done`; each such block at the end is git's, and everything before the
/// first of them is kept. A hook line that starts with `To ` stays, and
/// without a complete report at the end (a refused push prints no `Done`)
/// everything is kept.
fn before_porcelain(stdout: &[u8]) -> &[u8] {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in stdout.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start, &stdout[start..i]));
            start = i + 1;
        }
    }
    if start < stdout.len() {
        lines.push((start, &stdout[start..]));
    }
    let line = |i: usize| -> &[u8] { lines[i].1.strip_suffix(b"\r").unwrap_or(lines[i].1) };
    let ref_line = |l: &[u8]| l.len() > 1 && b" +-*!=".contains(&l[0]) && l[1] == b'\t';
    let mut end = lines.len();
    while end > 0 && line(end - 1) == b"Done" {
        let mut first = end - 1;
        while first > 0 && ref_line(line(first - 1)) {
            first -= 1;
        }
        if first == 0 || !line(first - 1).starts_with(b"To ") {
            break;
        }
        end = first - 1;
    }
    if end == lines.len() {
        stdout
    } else {
        &stdout[..lines[end].0]
    }
}

/// A porcelain ref line: a flag, a tab.
fn ref_line(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() > 1 && b" +-*!=".contains(&b[0]) && b[1] == b'\t'
}

/// `git push --porcelain`'s stdout without git's reports (each a `To
/// <destination>` line, its ref lines, and `Done` when git printed one):
/// what the `pre-push` hooks printed there, for the report. Found by
/// structure, so it decides nothing.
fn without_reports(stdout: &str) -> String {
    let lines: Vec<&str> = stdout.split_inclusive('\n').collect();
    let bare = |line: &str| line.trim_end_matches(['\n', '\r']).to_string();
    let (mut rest, mut i) = (String::new(), 0);
    while i < lines.len() {
        let mut end = i + 1;
        if bare(lines[i]).starts_with("To ") {
            while end < lines.len() && ref_line(&bare(lines[end])) {
                end += 1;
            }
        }
        if end > i + 1 {
            if end < lines.len() && bare(lines[end]) == "Done" {
                end += 1;
            }
        } else {
            rest.push_str(lines[i]);
        }
        i = end;
    }
    rest
}

/// git's own report at the end of `git push --porcelain`'s stdout: the
/// last line, when it starts with `To ` and only ref lines (one at least)
/// and `Done` follow it. git prints it once the last push URL's `pre-push`
/// hooks have run, so no hook printed it. `None` when the end is anything
/// else (a hook's line joined to git's `To `, say).
fn last_report(stdout: &str) -> Option<String> {
    let lines: Vec<&str> = stdout.lines().map(|l| l.trim_end_matches('\r')).collect();
    let mut end = lines.len();
    if end > 0 && lines[end - 1] == "Done" {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && ref_line(lines[start - 1]) {
        start -= 1;
    }
    (start < end && start > 0 && lines[start - 1].starts_with("To "))
        .then(|| lines[start - 1..end].join("\n"))
}

/// One `pre-push` run of a push, from git's trace2 event stream: its
/// arguments (the hook, the remote, the URL) and its exit code, `None`
/// when the stream records none.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PrePushRun {
    argv: Vec<String>,
    code: Option<i64>,
}

/// The `pre-push` runs git's trace2 event stream (`GIT_TRACE2_EVENT`)
/// records for the push itself (git 2.38 to 2.55 alike): its
/// `child_start` events with `child_class` `hook` and `hook_name`
/// `pre-push`, with their `child_exit` codes. Only the push's own events
/// count: its `sid` is the shortest, since every git it or a hook starts
/// has the push's `sid`, a `/` and its own. A line that is no event is
/// skipped; `None` when no line is one.
fn pre_push_runs(events: &str) -> Option<Vec<PrePushRun>> {
    let events: Vec<serde_json::Value> = events
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let own = events
        .iter()
        .filter_map(|event| event["sid"].as_str())
        .min_by_key(|sid| sid.len())?;
    let mut runs: BTreeMap<i64, PrePushRun> = BTreeMap::new();
    for event in events.iter().filter(|event| event["sid"] == own) {
        let Some(id) = event["child_id"].as_i64() else {
            continue;
        };
        match event["event"].as_str() {
            Some("child_start")
                if event["child_class"] == "hook" && event["hook_name"] == "pre-push" =>
            {
                let argv = event["argv"]
                    .as_array()
                    .map(|argv| {
                        argv.iter()
                            .filter_map(|arg| arg.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                runs.insert(id, PrePushRun { argv, code: None });
            }
            Some("child_exit") => {
                if let Some(run) = runs.get_mut(&id) {
                    run.code = event["code"].as_i64();
                }
            }
            _ => {}
        }
    }
    Some(runs.into_values().collect())
}

/// A file removed when dropped, however the step that wrote it ended.
struct TempFile(PathBuf);

impl TempFile {
    /// A new, unique path in the system's temporary folder, absolute (as
    /// `GIT_TRACE2_EVENT` needs it).
    fn new(stem: &str) -> TempFile {
        let path = std::env::temp_dir().join(unique_name(stem));
        TempFile(std::path::absolute(&path).unwrap_or(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What the `pre-push` hooks printed: their stdout (`hook_stdout`, git's
/// reports taken out) and stderr, without git's own `failed to push some
/// refs` line.
fn hook_output(hook_stdout: &str, stderr: &str) -> String {
    let mut output = hook_stdout.to_string();
    for line in stderr.split_inclusive('\n') {
        if !line.contains("failed to push some refs to '") {
            output.push_str(line);
        }
    }
    output
}

/// NUL-separated names, the empty ones dropped.
fn nul_separated(out: &[u8]) -> Vec<String> {
    out.split(|b| *b == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect()
}

fn first_line(out: &[u8]) -> String {
    String::from_utf8_lossy(out)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Whether git's porcelain `report` turned `refspec` back because the
/// remote has commits we do not: a `!` line for exactly that refspec with
/// a plain rejection (`fetch first`, `non-fast-forward`, `stale info`), or
/// the remote's refusal when its branch moved while the push ran
/// (`incorrect old value provided`), never a remote's own refusal (a
/// hook, a protected branch).
fn remote_moved(report: &str, refspec: &str) -> bool {
    report.lines().any(|line| {
        let mut fields = line.trim_end_matches('\r').split('\t');
        let (Some("!"), Some(spec), Some(summary)) = (fields.next(), fields.next(), fields.next())
        else {
            return false;
        };
        spec == refspec
            && ((summary.starts_with("[rejected]")
                && ["fetch first", "non-fast-forward", "stale info"]
                    .iter()
                    .any(|why| summary.contains(why)))
                || (summary.starts_with("[remote rejected]")
                    && summary.contains("incorrect old value provided")))
    })
}

/// The settings that keep a git from starting a background process (which
/// would be stopped with the step): no automatic maintenance or gc, no
/// fsmonitor daemon.
pub const NO_BACKGROUND: [&str; 3] = [
    "maintenance.auto=false",
    "gc.auto=0",
    "core.fsmonitor=false",
];

/// The environment that keeps a git from prompting, given the caller's
/// environment (`get`) and whether git config sets `core.sshCommand`:
/// `GIT_TERMINAL_PROMPT=0` and an empty `GIT_ASKPASS` (git asks no askpass
/// helper, `core.askPass` and `SSH_ASKPASS` included, and fails instead)
/// always; `GCM_INTERACTIVE=never` unless the user set it;
/// `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` unless the user chose an ssh
/// command (`GIT_SSH_COMMAND`, `GIT_SSH` or `core.sshCommand`, which the
/// variable would override).
pub fn remote_env(
    get: impl Fn(&str) -> Option<OsString>,
    ssh_command_configured: bool,
) -> Vec<(&'static str, &'static str)> {
    let set = |key: &str| get(key).is_some_and(|v| !v.is_empty());
    let mut env = vec![("GIT_TERMINAL_PROMPT", "0"), ("GIT_ASKPASS", "")];
    if !set("GCM_INTERACTIVE") {
        env.push(("GCM_INTERACTIVE", "never"));
    }
    if !(ssh_command_configured || set("GIT_SSH_COMMAND") || set("GIT_SSH")) {
        env.push(("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"));
    }
    env
}

impl SystemGit {
    /// What a push that did not succeed means, from what git itself says
    /// (`runs`: the push's `pre-push` runs, `None` without its trace2
    /// stream):
    /// - a `pre-push` run that exited with anything but 0: that hook
    ///   refused (`Hook`, with the URL git ran it for). It stopped the
    ///   transfer to that URL only: another URL may have taken the push;
    /// - 128: git died (`die()`) before any ref went to the URL it was at,
    ///   though an earlier URL may have taken the push: network trouble
    ///   ([`network_failure`], the push URLs asked by `deadline`,
    ///   `Unreachable` once it passed) or another refusal (`Refused`);
    /// - a ref turned back because the remote moved (`RemoteMoved`), read
    ///   only where no hook can have printed it: anywhere on stdout when no
    ///   `pre-push` ran, else in git's last report and only when every run
    ///   exited 0;
    /// - anything else (the remote's rules, or what cannot be told apart):
    ///   `Failed`, with all git printed.
    fn push_failure(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        done: &Finished,
        runs: Option<&[PrePushRun]>,
        deadline: Instant,
    ) -> PushOutcome {
        let stdout = String::from_utf8_lossy(&done.stdout);
        let stderr = String::from_utf8_lossy(&done.stderr);
        let refused =
            runs.and_then(|runs| runs.iter().find(|run| run.code.is_some_and(|c| c != 0)));
        if let Some(run) = refused {
            return PushOutcome::Hook {
                url: run.argv.get(2).cloned(),
                output: hook_output(&without_reports(&stdout), &stderr),
            };
        }
        if done.status.code() == Some(128) {
            let args = ["remote", "get-url", "--push", "--all", &upstream.remote];
            return match self.checked(&repo.dir, &args, deadline) {
                Err(RemoteError::TimedOut) => PushOutcome::Unreachable,
                Ok(out) => {
                    let urls: Vec<String> = String::from_utf8_lossy(&out)
                        .lines()
                        .map(str::to_string)
                        .collect();
                    if network_failure(&urls, &stderr) {
                        PushOutcome::Unreachable
                    } else {
                        PushOutcome::Refused(stderr.into_owned())
                    }
                }
                Err(RemoteError::Failed(_)) => PushOutcome::Refused(stderr.into_owned()),
            };
        }
        let refspec = format!("{}:{}", upstream.branch, upstream.remote_ref);
        let moved = match runs {
            Some([]) => remote_moved(&stdout, &refspec),
            Some(runs) if runs.iter().all(|run| run.code == Some(0)) => {
                last_report(&stdout).is_some_and(|report| remote_moved(&report, &refspec))
            }
            _ => false,
        };
        if moved {
            return PushOutcome::RemoteMoved;
        }
        let detail = [stdout.trim(), stderr.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        PushOutcome::Failed(detail)
    }

    /// git with `args` in `dir`, stopped with every process it started at
    /// `deadline`.
    pub(crate) fn bounded(
        &self,
        dir: &Path,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Finished, RemoteError> {
        match run_until(self.command(Some(dir), args), self.bound(deadline)) {
            Ok(done) => Ok(done),
            Err(RunError::TimedOut) => Err(RemoteError::TimedOut),
            Err(RunError::Spawn(e)) => Err(RemoteError::Failed(GitError::new(
                args,
                None,
                e.to_string(),
            ))),
        }
    }

    /// [`SystemGit::bounded`], failing unless git exits 0.
    pub(crate) fn checked(
        &self,
        dir: &Path,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Vec<u8>, RemoteError> {
        let done = self.bounded(dir, args, deadline)?;
        if done.status.success() {
            self.forward(&done.stderr);
            Ok(done.stdout)
        } else {
            Err(RemoteError::Failed(GitError::new(
                args,
                done.status.code(),
                String::from_utf8_lossy(&done.stderr).trim(),
            )))
        }
    }

    /// This git when it has the remote environment, else one with it.
    fn remote(&self, repo: &Repo) -> Result<Cow<'_, SystemGit>, RemoteError> {
        if self.remote_ready {
            Ok(Cow::Borrowed(self))
        } else {
            self.for_remote(repo)
                .map(Cow::Owned)
                .map_err(RemoteError::Failed)
        }
    }
}

impl SyncGit for SystemGit {
    fn showing_output(self) -> Self {
        self.forwarding_stderr()
    }

    fn with_deadline(mut self, deadline: Option<Instant>) -> Self {
        self.deadline = deadline;
        self
    }

    fn discover(&self, dir: &Path) -> Result<Option<Repo>, GitError> {
        SystemGit::discover(self, dir)
    }

    fn for_remote(&self, repo: &Repo) -> Result<Self, GitError> {
        if self.remote_ready {
            return Ok(self.clone());
        }
        let args = ["config", "--get", "core.sshCommand"];
        let configured = match self.run(&repo.dir, &args) {
            Ok(out) => !out.trim_ascii().is_empty(),
            Err(e) if e.code == Some(1) => false,
            Err(e) => return Err(e),
        };
        let mut git = self.clone();
        for (key, value) in remote_env(|key| self.env_var(key), configured) {
            git = git.env(key, value);
        }
        for setting in NO_BACKGROUND {
            git = git.config(setting);
        }
        git.remote_ready = true;
        Ok(git)
    }

    fn git_dirs(&self, repo: &Repo) -> Result<GitDirs, GitError> {
        let args = ["rev-parse", "--absolute-git-dir", "--git-common-dir"];
        let out = self.run(&repo.dir, &args)?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        let (Some(own), Some(common)) = (lines.next(), lines.next()) else {
            return Err(GitError::new(
                &args,
                None,
                format!("unexpected output: {text}"),
            ));
        };
        // A relative common dir is relative to where git ran.
        Ok(GitDirs {
            own: PathBuf::from(own),
            common: repo.dir.join(common),
        })
    }

    fn is_shallow(&self, repo: &Repo) -> Result<bool, GitError> {
        let out = self.run(&repo.dir, &["rev-parse", "--is-shallow-repository"])?;
        Ok(first_line(&out) == "true")
    }

    fn count(&self, repo: &Repo, from: Option<&str>, to: &str) -> Result<usize, GitError> {
        let range = match from {
            Some(from) => format!("{from}..{to}"),
            None => to.to_string(),
        };
        let args = ["rev-list", "--count", &range];
        let out = self.run(&repo.dir, &args)?;
        first_line(&out).parse().map_err(|_| {
            GitError::new(
                &args,
                None,
                format!("not a count: {}", String::from_utf8_lossy(&out)),
            )
        })
    }

    fn branch(&self, repo: &Repo) -> Result<Option<String>, GitError> {
        let args = ["symbolic-ref", "--quiet", "HEAD"];
        match self.run(&repo.dir, &args) {
            Ok(out) => Ok(Some(first_line(&out)).filter(|b| b.starts_with("refs/heads/"))),
            // A detached HEAD.
            Err(e) if e.code == Some(1) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn upstream(&self, repo: &Repo, branch: &str) -> Result<Option<Upstream>, GitError> {
        let format = "--format=%(upstream:remotename)%00%(upstream)%00%(upstream:remoteref)";
        let out = self.run(&repo.dir, &["for-each-ref", format, branch])?;
        let text = String::from_utf8_lossy(&out);
        let line = text.lines().next().unwrap_or_default();
        let mut fields = line.split('\0');
        let (Some(remote), Some(refname), Some(remote_ref)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Ok(None);
        };
        if remote.is_empty() || remote == "." || refname.is_empty() || remote_ref.is_empty() {
            return Ok(None);
        }
        Ok(Some(Upstream {
            branch: branch.to_string(),
            remote: remote.to_string(),
            refname: refname.to_string(),
            remote_ref: remote_ref.to_string(),
        }))
    }

    fn ls_remote(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        deadline: Instant,
    ) -> Result<Option<String>, RemoteError> {
        let args = [
            "ls-remote",
            "--quiet",
            "--refs",
            &upstream.remote,
            &upstream.remote_ref,
        ];
        let out = self.remote(repo)?.checked(&repo.dir, &args, deadline)?;
        Ok(listed(&out, &upstream.remote_ref))
    }

    fn fetch_branch(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        deadline: Instant,
    ) -> Result<(), RemoteError> {
        // The source alone (no `:dst`) with an empty `--refmap=` updates no
        // remote-tracking ref (`remote.<name>.fetch` is ignored, and so is
        // `remote.<name>.followRemoteHEAD`); no tags, no pruning, no
        // `FETCH_HEAD`; no commit-graph, and no bundle URI (which writes
        // `refs/bundles/*` and `fetch.bundleCreationToken`).
        let args = [
            "-c",
            "fetch.writeCommitGraph=false",
            "-c",
            "fetch.bundleURI=",
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-prune",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--refmap=",
            &upstream.remote,
            &upstream.remote_ref,
        ];
        self.remote(repo)?
            .checked(&repo.dir, &args, deadline)
            .map(drop)
    }

    fn has_commit(
        &self,
        repo: &Repo,
        commit: &str,
        deadline: Instant,
    ) -> Result<bool, RemoteError> {
        let spec = format!("{commit}^{{commit}}");
        let git = self
            .remote(repo)?
            .into_owned()
            .env("GIT_NO_LAZY_FETCH", "1");
        match git.checked(&repo.dir, &["cat-file", "-e", &spec], deadline) {
            Ok(_) => Ok(true),
            // Not here (git cannot name it, or it is no commit).
            Err(RemoteError::Failed(e)) if e.code.is_some() => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn remote_urls(&self, repo: &Repo, remote: &str) -> Result<Vec<String>, GitError> {
        let out = self.run(&repo.dir, &["ls-remote", "--get-url", remote])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(str::to_string)
            .collect())
    }

    fn remotes(&self, repo: &Repo) -> Result<Vec<String>, GitError> {
        let out = self.run(&repo.dir, &["remote"])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(str::to_string)
            .collect())
    }

    fn ref_value(&self, repo: &Repo, refname: &str) -> Result<Option<String>, GitError> {
        let args = ["rev-parse", "--verify", "--quiet", refname];
        let out = self.output(self.command(Some(&repo.dir), &args), &args);
        match out {
            Ok(out) => Ok(Some(first_line(&out.stdout))),
            Err(e) if e.code == Some(1) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn symbolic_target(&self, repo: &Repo, refname: &str) -> Result<Option<String>, GitError> {
        let args = ["symbolic-ref", "--quiet", refname];
        let out = self.output(self.command(Some(&repo.dir), &args), &args);
        match out {
            Ok(out) => Ok(Some(first_line(&out.stdout))),
            // A plain ref, or none.
            Err(e) if e.code == Some(1) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn move_ref(
        &self,
        repo: &Repo,
        refname: &str,
        new: &str,
        old: Option<&str>,
        message: &str,
    ) -> Result<(), GitError> {
        // An empty old value: the ref must not exist.
        let old = old.unwrap_or("");
        let args = ["update-ref", "--no-deref", "-m", message, refname, new, old];
        self.run(&repo.dir, &args).map(drop)
    }

    fn is_ancestor(&self, repo: &Repo, ancestor: &str, descendant: &str) -> Result<bool, GitError> {
        let args = ["merge-base", "--is-ancestor", ancestor, descendant];
        match self.output(self.command(Some(&repo.dir), &args), &args) {
            Ok(_) => Ok(true),
            Err(e) if e.code == Some(1) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn merge_bases(&self, repo: &Repo, a: &str, b: &str) -> Result<Vec<String>, GitError> {
        let args = ["merge-base", "--all", a, b];
        let out = self.output(self.command(Some(&repo.dir), &args), &args);
        match out {
            Ok(out) => Ok(String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()),
            // No common ancestor: unrelated histories.
            Err(e) if e.code == Some(1) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn merge_tree(&self, repo: &Repo, ours: &str, theirs: &str) -> Result<MergeTree, GitError> {
        let args = ["merge-tree", "--write-tree", "-z", ours, theirs];
        let out = self.finish(self.command(Some(&repo.dir), &args), &args, None)?;
        // 0: clean; 1: conflicts, listed after the tree; anything else fails.
        if matches!(out.status.code(), Some(0 | 1)) {
            self.forward(&out.stderr);
        } else {
            return Err(GitError::new(
                &args,
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).trim(),
            ));
        }
        Ok(parse_merge_tree(&out.stdout))
    }

    fn checked_out(&self, repo: &Repo, path: &str, blob: &str) -> Result<Vec<u8>, GitError> {
        // One git per path: `--batch --filters` gives the unconverted size
        // in its header, so its output cannot be split.
        let path = format!("--path={path}");
        self.run(&repo.dir, &["cat-file", "--filters", &path, blob])
    }

    fn diff_names(&self, repo: &Repo, from: &str, to: &str) -> Result<Vec<String>, GitError> {
        let args = [
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            from,
            to,
        ];
        Ok(nul_separated(&self.run(&repo.dir, &args)?))
    }

    fn fast_forward_to(&self, repo: &Repo, to: &str) -> Result<(), GitError> {
        // Whatever the user's config says: an ignored file in the way stops
        // it (git overwrites one by default), and a local change stops it
        // instead of being stashed and applied back with conflict markers.
        let args = [
            "-c",
            "merge.autostash=false",
            "-c",
            "rebase.autoStash=false",
            "merge",
            "--ff-only",
            "--no-overwrite-ignore",
            "--quiet",
            to,
        ];
        // The post-merge hook's output reaches the user, all of it on
        // stderr so `--json` stays one document; git's own message too, and
        // it is kept for the error. The land is never stopped, whatever
        // this git's deadline: stopped partway, it would leave the work
        // tree half moved.
        let out = self
            .command(Some(&repo.dir), &args)
            .output()
            .map_err(|e| GitError::new(&args, None, e.to_string()))?;
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(&out.stdout);
        let _ = stderr.write_all(&out.stderr);
        if out.status.success() {
            Ok(())
        } else {
            Err(GitError::new(
                &args,
                out.status.code(),
                one_line(&String::from_utf8_lossy(&out.stderr)),
            ))
        }
    }

    fn push(&self, repo: &Repo, upstream: &Upstream, deadline: Instant) -> PushOutcome {
        let git = match self.remote(repo) {
            Ok(git) => git,
            Err(RemoteError::TimedOut) => return PushOutcome::Unreachable,
            Err(RemoteError::Failed(e)) => return PushOutcome::Failed(e.to_string()),
        };
        let refspec = format!("{}:{}", upstream.branch, upstream.remote_ref);
        let args = [
            "push",
            "--porcelain",
            "--no-recurse-submodules",
            &upstream.remote,
            &refspec,
        ];
        // Which `pre-push` runs refused, from git itself; removed however
        // the push ends.
        let trace = TempFile::new("kluris-push-trace2");
        let traced = git
            .as_ref()
            .clone()
            .env("GIT_TRACE2_EVENT", trace.0.as_os_str());
        let done = traced.bounded(&repo.dir, &args, deadline);
        // What a hook printed and git's stderr reach the user, on stderr so
        // `--json` stays one document; both are kept for the outcome.
        if let Ok(done) = &done {
            let mut stderr = std::io::stderr();
            let _ = stderr.write_all(before_porcelain(&done.stdout));
            let _ = stderr.write_all(&done.stderr);
        }
        match done {
            Err(RemoteError::TimedOut) => PushOutcome::Unreachable,
            Err(RemoteError::Failed(e)) => PushOutcome::Failed(e.to_string()),
            Ok(done) if done.status.success() => PushOutcome::Pushed,
            Ok(done) => {
                let runs = std::fs::read_to_string(&trace.0)
                    .ok()
                    .and_then(|events| pre_push_runs(&events));
                git.push_failure(repo, upstream, &done, runs.as_deref(), deadline)
            }
        }
    }
}

impl TextMerger for SystemGit {
    fn merge_text(&self, base: &str, ours: &str, theirs: &str) -> Result<Option<String>, GitError> {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kluris-merge-{}-{n}", std::process::id()));
        let error = |detail: String| GitError::new(&["merge-file"], None, detail);
        std::fs::create_dir_all(&dir).map_err(|e| error(e.to_string()))?;
        let result = merge_files(self, &dir, base, ours, theirs, &error);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

/// `git merge-file -p` on three files in `dir`: the merged text, or `None`
/// when a change overlaps.
fn merge_files(
    git: &SystemGit,
    dir: &Path,
    base: &str,
    ours: &str,
    theirs: &str,
    error: &dyn Fn(String) -> GitError,
) -> Result<Option<String>, GitError> {
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|e| error(e.to_string()))?;
        Ok::<_, GitError>(path.to_string_lossy().into_owned())
    };
    let (o, b, t) = (
        write("ours", ours)?,
        write("base", base)?,
        write("theirs", theirs)?,
    );
    let args = ["merge-file", "-p", &o, &b, &t];
    let out = git.finish(git.command(Some(dir), &args), &args, None)?;
    match out.status.code() {
        Some(0) => String::from_utf8(out.stdout)
            .map(Some)
            .map_err(|_| error("the merged text is not UTF-8".into())),
        // The number of conflicts (git caps it at 127).
        Some(1..=127) => Ok(None),
        code => Err(GitError::new(
            &args,
            code,
            String::from_utf8_lossy(&out.stderr).trim(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tree_output_keeps_stages_and_messages() {
        let clean = parse_merge_tree(b"abc\0");
        assert_eq!(clean.tree, "abc");
        assert!(clean.stages.is_empty() && clean.messages.is_empty());
        let out = b"t1\x00100644 aaa 3\tnew/c.md\0\x002\0new/c.md\0old/c.md\0CONFLICT (directory rename suggested)\0text\0";
        let m = parse_merge_tree(out);
        assert_eq!(m.tree, "t1");
        let s = &m.stages["new/c.md"];
        assert!(s[0].is_none() && s[1].is_none());
        assert_eq!(s[2].as_ref().map(|e| e.oid.as_str()), Some("aaa"));
        assert_eq!(
            m.messages,
            [(
                vec!["new/c.md".to_string(), "old/c.md".to_string()],
                "CONFLICT (directory rename suggested)".to_string()
            )]
        );
        assert!(m.conflicted.contains("new/c.md"));
    }

    #[test]
    fn a_rejected_push_is_told_apart_from_a_refused_one() {
        let spec = "refs/heads/main:refs/heads/main";
        let moved =
            "To /r.git\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\nDone\n";
        assert!(remote_moved(moved, spec));
        assert!(remote_moved(
            "!\trefs/heads/main:refs/heads/main\t[rejected] (non-fast-forward)\r\n",
            spec
        ));
        assert!(remote_moved(
            "!\trefs/heads/main:refs/heads/main\t[remote rejected] (incorrect old value provided)\n",
            spec
        ));
        assert!(!remote_moved(
            "!\trefs/heads/main:refs/heads/main\t[remote rejected] (pre-receive hook declined)\n",
            spec
        ));
        assert!(!remote_moved(
            "=\trefs/heads/main:refs/heads/main\t[up to date]\n",
            spec
        ));
        // Another ref's rejection, or another push's, is not this one's.
        assert!(!remote_moved(
            "!\trefs/heads/other:refs/heads/other\t[rejected] (fetch first)\n",
            spec
        ));
        assert!(!remote_moved(
            "gate: !\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\n",
            spec
        ));
    }

    #[test]
    fn a_fetch_stopped_by_a_ref_lock_is_told_apart() {
        // git 2.38 to 2.55 on macOS and Linux.
        let unix = "error: cannot lock ref 'refs/remotes/origin/main': Unable to create '/b/.git/refs/remotes/origin/main.lock': File exists.\n\nAnother git process seems to be running in this repository, or the lock file may be stale";
        assert_eq!(
            ref_lock(unix),
            Some(RefLock::File(
                "/b/.git/refs/remotes/origin/main.lock".into()
            ))
        );
        let packed = "error: Unable to create 'C:/b/.git/packed-refs.lock': File exists.";
        assert_eq!(
            ref_lock(packed),
            Some(RefLock::File("C:/b/.git/packed-refs.lock".into()))
        );
        assert_eq!(
            ref_lock("error: cannot lock ref 'refs/remotes/origin/main': is at 1 but expected 2"),
            Some(RefLock::Ref("refs/remotes/origin/main".into()))
        );
        // Git for Windows after a terminated fetch.
        assert_eq!(
            ref_lock(
                "From C:/r\n   a8a671a..3b9f47e  main       -> origin/main\nerror: fetching ref refs/remotes/origin/main failed: reference already exists\n"
            ),
            Some(RefLock::Ref("refs/remotes/origin/main".into()))
        );
        for other in [
            "fatal: unable to access 'https://h/r.git/': Could not resolve host: h",
            "fatal: '/gone.git' does not appear to be a git repository",
            "",
        ] {
            assert_eq!(ref_lock(other), None, "{other}");
        }
    }

    /// [`network_failure`] for a remote over ssh: the diagnostics decide.
    fn over_ssh(stderr: &str) -> bool {
        network_failure(&["ssh://h/x.git".to_string()], stderr)
    }

    /// A missing local path that holds an apostrophe, then a newline, then
    /// words that read like ssh's, and git 2.55's message for it on macOS.
    const QUOTED_PATH: &str =
        "/tmp/x/missing'\nssh: Could not resolve hostname this-is-path-text.git";
    const QUOTED_PATH_STDERR: &str = "fatal: '/tmp/x/missing'\nssh: Could not resolve hostname this-is-path-text.git' does not appear to be a git repository\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n";

    #[test]
    fn a_url_is_local_when_git_reaches_it_without_a_network() {
        for (url, local) in [
            ("/srv/brain.git", true),
            ("brain.git", true),
            ("./brain", true),
            ("../brain.git", true),
            ("/srv/a:b/brain.git", true),
            ("file:///srv/brain.git", true),
            ("file://srv/brain.git", true),
            ("\\\\server\\share\\brain.git", true),
            (QUOTED_PATH, true),
            ("", true),
            ("ssh://h/brain.git", false),
            ("ssh://h/srv/a:b", false),
            ("git+ssh://h/brain.git", false),
            ("git@h:brain.git", false),
            ("host:brain.git", false),
            ("host:/srv/brain.git", false),
            ("https://h/brain.git", false),
            ("http://127.0.0.1:9/brain.git", false),
            ("git://h/brain.git", false),
            ("ext::ssh -i key h %S brain.git", false),
            ("persistent-https::https://h/brain.git", false),
            ("FILE:///srv/brain.git", false),
            // A drive letter (one letter, then `:`) is a path on Windows,
            // ssh's `host:path` elsewhere.
            ("C:/srv/brain.git", cfg!(windows)),
            ("C:\\srv\\brain.git", cfg!(windows)),
            ("c:brain.git", cfg!(windows)),
            ("h:/srv/brain.git", cfg!(windows)),
        ] {
            assert_eq!(local_url(url), local, "{url:?}");
        }
    }

    #[test]
    fn a_local_remote_is_never_offline() {
        let read = "\nfatal: Could not read from remote repository.\n";
        for url in ["/srv/brain.git", "file:///srv/brain.git", QUOTED_PATH] {
            for stderr in [
                format!("ssh: Could not resolve hostname h: nodename nor servname provided, or not known{read}"),
                format!("ssh: connect to host h port 22: Connection refused{read}"),
                "fatal: unable to access 'https://h/x.git/': Could not resolve host: h".into(),
                "fatal: unable to look up h (port 9418) (nodename nor servname provided, or not known)".into(),
                QUOTED_PATH_STDERR.into(),
            ] {
                assert!(!network_failure(&[url.to_string()], &stderr), "{url:?}: {stderr}");
                // One local URL among a push's URLs is enough.
                let urls = ["ssh://h/x.git".to_string(), url.to_string()];
                assert!(!network_failure(&urls, &stderr), "{url:?}: {stderr}");
            }
        }
    }

    #[test]
    fn a_remote_whose_url_git_could_not_say_is_never_offline() {
        let stderr = "ssh: connect to host h port 22: Connection refused\nfatal: Could not read from remote repository.\n";
        assert!(over_ssh(stderr), "over ssh this is network trouble");
        assert!(!network_failure(&[], stderr));
    }

    #[test]
    fn no_line_after_a_quoted_field_is_read_for_network_trouble() {
        // A quoted path or URL may hold `'` and newlines, so where the
        // field ends cannot be told: what follows it may be the field.
        let ssh = "ssh: connect to host h port 22: Connection refused";
        for refused in [
            QUOTED_PATH_STDERR.to_string(),
            format!("fatal: '/h/a' b\n{ssh}' does not appear to be a git repository\n"),
            format!("fatal: '/h/it's\n{ssh}' does not appear to be a git repository\n"),
            format!("fatal: '/h/a'\nb'\n{ssh}' does not appear to be a git repository\n"),
            format!("error: '/h/a' x\n{ssh}\n"),
            format!("fatal: Authentication failed for 'https://h/x.git/'\n{ssh}\n"),
        ] {
            assert!(!over_ssh(&refused), "{refused}");
        }
        // The words on the field's own line are curl's reason, after it.
        assert!(over_ssh(
            "fatal: unable to access 'https://h/x.git/': Could not resolve host: h\n"
        ));
    }

    #[test]
    fn network_trouble_is_told_apart_from_a_refusal() {
        // What git 2.55 printed on macOS for each.
        let read = "\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n";
        for network in [
            format!("ssh: Could not resolve hostname nonexistent.invalid: nodename nor servname provided, or not known{read}"),
            "fatal: unable to access 'https://nonexistent.invalid/x.git/': Could not resolve host: nonexistent.invalid".into(),
            "fatal: unable to access 'http://127.0.0.1:9/x.git/': Failed to connect to 127.0.0.1 port 9 after 0 ms: Couldn't connect to server".into(),
            "fatal: unable to connect to 127.0.0.1:\n127.0.0.1[0: 127.0.0.1]: errno=Connection refused\n".into(),
            "fatal: unable to look up nonexistent.invalid (port 9418) (nodename nor servname provided, or not known)".into(),
            format!("ssh: connect to host 127.0.0.1 port 9: Connection refused{read}"),
            format!("ssh: connect to host 10.0.0.1 port 22: Operation timed out{read}"),
            format!("ssh: connect to host 10.0.0.1 port 22: Network is unreachable{read}"),
            "fatal: unable to access 'https://h/x.git/': Connection timed out after 300001 milliseconds".into(),
            "fatal: unable to access 'https://h/x.git/': Could not resolve proxy: p".into(),
            // An http transfer that failed after it started: curl's
            // `lib/multi.c` text after git's `RPC failed; curl <n> `.
            "error: RPC failed; curl 28 Operation timed out after 300000 milliseconds with 0 bytes received\nfetch-pack: unexpected disconnect while reading sideband packet\nfatal: early EOF\n".into(),
        ] {
            assert!(over_ssh(&network), "{network}");
        }
        for refused in [
            format!("Permission denied (publickey).{read}"),
            format!("fatal: '/gone.git' does not appear to be a git repository{read}"),
            "remote: Repository not found.\nfatal: repository 'https://github.com/a/b.git/' not found".into(),
            "fatal: Authentication failed for 'https://h/x.git/'".into(),
            "fatal: unable to access 'https://h/x.git/': The requested URL returned error: 403".into(),
            format!("ERROR: Permission to a/b.git denied to deploy-key.{read}"),
            "fatal: could not read Username for 'https://h': terminal prompts disabled".into(),
            // An http transfer that failed after it started, for a reason
            // that is not network trouble.
            "error: RPC failed; curl 22 The requested URL returned error: 403\nfatal: the remote end hung up unexpectedly\n".into(),
            "error: RPC failed; HTTP 403 curl 22 The requested URL returned error: 403\nfatal: the remote end hung up unexpectedly\n".into(),
            "error: RPC failed; curl 18 transfer closed with 99992 bytes remaining to read\nfetch-pack: unexpected disconnect while reading sideband packet\nfatal: protocol error: bad pack header\n".into(),
            String::new(),
        ] {
            assert!(!over_ssh(&refused), "{refused}");
        }
    }

    #[test]
    fn network_words_in_a_path_a_url_or_a_remote_line_are_a_refusal() {
        // What git 2.55 printed on macOS for each.
        let read = "\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n";
        for refused in [
            format!("fatal: '/h/timed out.git' does not appear to be a git repository{read}"),
            format!(
                "fatal: '/h/Connection refused.git' does not appear to be a git repository{read}"
            ),
            format!("fatal: '/x/No route to host.git' does not appear to be a git repository{read}"),
            "fatal: unable to access 'https://github.com/a/Could not resolve host.git/': URL rejected: Malformed input to a URL function\n".into(),
            "remote: Repository not found.\nfatal: repository 'https://github.com/a/Could%20not%20resolve%20host.git/' not found\n".into(),
            // A URL that holds `': ` and the words: curl's reason is after
            // the last `': `.
            "fatal: unable to access 'https://h/a': Failed to connect to x/': The requested URL returned error: 404\n".into(),
            // A fetch whose upload-pack failed with these words.
            "remote: fatal: the operation timed out on the server        \nerror: git upload-pack: git-pack-objects died with error.\nfatal: git upload-pack: aborting due to possible repository corruption on the remote side.\nremote: aborting due to possible repository corruption on the remote side.\nfatal: protocol error: bad pack header\n".into(),
            // A push a pre-receive hook turned away with these words.
            "remote: fatal: unable to access x: Connection timed out after 300001 milliseconds        \nremote: ssh: connect to host h port 22: Operation timed out        \nerror: failed to push some refs to '/h/hooked.git'\n".into(),
        ] {
            assert!(!over_ssh(&refused), "{refused}");
        }
    }

    #[test]
    fn network_words_inside_a_quoted_field_that_spans_lines_are_a_refusal() {
        // What git 2.55 printed on macOS for a remote path that holds a
        // newline: the second line is inside the quoted path.
        let read = "\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n";
        for refused in [
            format!(
                "fatal: '/h/a\nssh: connect to host h port 22: Connection refused' does not appear to be a git repository{read}"
            ),
            format!(
                "fatal: '/h/a\nb\nunable to access 'x': Could not resolve host: h' does not appear to be a git repository{read}"
            ),
        ] {
            assert!(!over_ssh(&refused), "{refused}");
        }
        // An apostrophe inside a word opens no field: the lines after it
        // are read.
        for network in [
            "fatal: couldn't find remote ref refs/heads/x\nssh: connect to host h port 22: Connection refused\n",
            "fatal: unable to access 'http://127.0.0.1:9/x.git/': Failed to connect to 127.0.0.1 port 9 after 0 ms: Couldn't connect to server\n",
        ] {
            assert!(over_ssh(network), "{network}");
        }
    }

    #[test]
    fn a_transfer_too_slow_for_http_low_speed_limit_is_network_trouble() {
        // curl's `lib/progress.c` under `http.lowSpeedLimit`.
        for network in [
            "fatal: unable to access 'http://127.0.0.1:9/x.git/': Operation too slow. Less than 1000 bytes/sec transferred the last 1 seconds\n",
            "error: RPC failed; curl 28 Operation too slow. Less than 1000 bytes/sec transferred the last 1 seconds\nfatal: expected flush after ref listing\n",
        ] {
            assert!(over_ssh(network), "{network}");
        }
    }

    #[test]
    fn a_push_forwards_what_comes_before_its_porcelain_report() {
        let report = "To ../r.git\n*\tHEAD:refs/heads/main\t[new branch]\nDone\n";
        for (stdout, hook) in [
            (
                format!("from pre-push\nTo be done\n{report}"),
                "from pre-push\nTo be done\n",
            ),
            (report.to_string(), ""),
            ("from pre-push\n".to_string(), "from pre-push\n"),
            (String::new(), ""),
            // A hook's own `To ` line, with no report after it (a refused
            // push prints no `Done`): everything is the hook's.
            (
                "To fix this rejection, contact the maintainer\nIMPORTANT: detail\n".to_string(),
                "To fix this rejection, contact the maintainer\nIMPORTANT: detail\n",
            ),
            // A `To ` line the report does not follow, before the report.
            (
                format!("To fix this, see below\nIMPORTANT: detail\n{report}"),
                "To fix this, see below\nIMPORTANT: detail\n",
            ),
            // A report cut short (no `Done`) is not taken for one.
            (
                "To ../r.git\n!\tHEAD:refs/heads/main\t[rejected] (fetch first)\n".to_string(),
                "To ../r.git\n!\tHEAD:refs/heads/main\t[rejected] (fetch first)\n",
            ),
        ] {
            assert_eq!(
                String::from_utf8_lossy(before_porcelain(stdout.as_bytes())),
                hook,
                "{stdout:?}"
            );
        }
    }

    /// git's last report is the one at the very end; a hook's line joined
    /// to its `To ` (a hook that printed no newline) leaves none.
    #[test]
    fn only_the_report_at_the_end_is_gits_last() {
        let ok = "To /a.git\n \trefs/heads/main:refs/heads/main\t1111111..2222222\nDone\n";
        let rejected = "To /b.git\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\n";
        assert_eq!(last_report(ok).as_deref(), Some(&ok[..ok.len() - 6]));
        assert_eq!(
            last_report(&format!("{rejected}hook says\n{ok}")).as_deref(),
            Some(&ok[..ok.len() - 6])
        );
        assert_eq!(
            last_report(&format!("{ok}{rejected}")).as_deref(),
            Some(rejected.trim_end())
        );
        assert_eq!(last_report(&format!("gate check: {ok}")), None);
        assert_eq!(last_report("hook output\n"), None);
        assert_eq!(last_report(""), None);
        assert_eq!(
            without_reports(&format!("first\r\n{ok}To fix this, ask\nlast\n")),
            "first\r\nTo fix this, ask\nlast\n"
        );
        let stderr = "HOOK-NO\nerror: failed to push some refs to '/b.git'\n";
        assert_eq!(hook_output("HOOK-OUT\n", stderr), "HOOK-OUT\nHOOK-NO\n");
    }

    /// The push's own `pre-push` runs from git's trace2 events: a nested
    /// git's events (a longer `sid`) and other children are not the push's.
    #[test]
    fn the_pushs_own_pre_push_runs_come_from_its_trace2_events() {
        let events = [
            r#"{"event":"version","sid":"outer/push"}"#,
            r#"{"event":"child_start","sid":"outer/push","child_id":0,"child_class":"transport/ssh","argv":["ssh"]}"#,
            r#"{"event":"child_start","sid":"outer/push","child_id":1,"child_class":"hook","hook_name":"pre-push","argv":[".git/hooks/pre-push","origin","/r.git"]}"#,
            r#"{"event":"child_start","sid":"outer/push/nested","child_id":1,"child_class":"hook","hook_name":"pre-push","argv":["x"]}"#,
            r#"{"event":"child_exit","sid":"outer/push/nested","child_id":1,"code":0}"#,
            "torn line",
            r#"{"event":"child_exit","sid":"outer/push","child_id":0,"code":255}"#,
            r#"{"event":"child_exit","sid":"outer/push","child_id":1,"code":7}"#,
            r#"{"event":"child_start","sid":"outer/push","child_id":2,"child_class":"hook","hook_name":"pre-push","argv":[".git/hooks/pre-push","origin","/s.git"]}"#,
        ]
        .join("\n");
        let run = |url: &str, code| PrePushRun {
            argv: vec![".git/hooks/pre-push".into(), "origin".into(), url.into()],
            code,
        };
        assert_eq!(
            pre_push_runs(&events),
            Some(vec![run("/r.git", Some(7)), run("/s.git", None)])
        );
        assert_eq!(
            pre_push_runs(r#"{"event":"version","sid":"push"}"#),
            Some(Vec::new())
        );
        assert_eq!(pre_push_runs(""), None);
        assert_eq!(pre_push_runs("not json\n"), None);
    }

    #[test]
    fn stderr_goes_on_one_line() {
        assert_eq!(
            one_line(
                "Permission denied (publickey).\r\nfatal: Could not read from remote repository.\n\nPlease make sure you have the correct access rights\nand the repository exists.\n"
            ),
            "Permission denied (publickey). fatal: Could not read from remote repository. Please make sure you have the correct access rights and the repository exists"
        );
        assert_eq!(one_line("  \n"), "");
    }
}
