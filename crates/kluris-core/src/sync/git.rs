//! The git layer of the sync engine: [`SyncGit`] (the steps beyond
//! [`Plumbing`]) and its [`SystemGit`].
//!
//! - **One upstream query:** [`SyncGit::upstream`], the current branch's
//!   remote, remote-tracking ref and branch on the remote ([`Upstream`]).
//! - **One remote environment:** [`SyncGit::for_remote`] gives a git whose
//!   every run has [`remote_env`] (no terminal prompt, no askpass, no
//!   interactive credential manager, ssh in batch mode unless the user chose
//!   an ssh command) and [`NO_BACKGROUND`]. The engine takes it once per
//!   brain, so no git it starts can prompt: not a fetch or a push, and not a
//!   merge or a checkout whose lazy fetch (a partial clone) talks to the
//!   remote. The steps that talk to a remote take it themselves when given
//!   a git without it.
//! - **stdin closed** for every git ([`SystemGit`]'s runs), so a hook that
//!   reads it never waits.
//! - **The push names no configured remote:** it goes to `kluris-<remote>`,
//!   an alias defined for that one git (`-c`) with every
//!   `remote.<remote>.*` value but `fetch` and `mirror`. git applies
//!   `insteadOf`, `pushInsteadOf`, the push URLs, `receivepack`, the proxy
//!   and the rest exactly as for the remote itself, but the alias has no
//!   fetch refspec, so git moves no remote-tracking ref after the push (one
//!   it would move through a symbolic ref, onto another branch): the caller
//!   moves it ([`super::fetch::advance`]). The `pre-push` hook gets
//!   `kluris-<remote>` and the push URL, and reads the alias's settings
//!   (`git config remote.$1.url`) from `GIT_CONFIG_PARAMETERS`.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::merge::TextMerger;
use crate::git::{GitError, Plumbing, Repo, SystemGit, TreeEntry};
use crate::process::{Finished, RunError, run_until};

/// Where the current branch pulls from and pushes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
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
    /// Refused for another reason (a hook, the remote's rules): git's words.
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
pub trait SyncGit: Plumbing + TextMerger + Sized {
    /// This git with the remote environment for `repo`: [`remote_env`]
    /// (`core.sshCommand` read once, here) and [`NO_BACKGROUND`] on every
    /// run. A git that has it already is given back as it is.
    fn for_remote(&self, repo: &Repo) -> Result<Self, GitError>;

    /// The current branch's upstream on a remote; `None` on a detached HEAD,
    /// without one, or with a local branch as upstream.
    fn upstream(&self, repo: &Repo) -> Result<Option<Upstream>, GitError>;

    /// The worktree's own git directory and the repository's common one.
    fn git_dirs(&self, repo: &Repo) -> Result<GitDirs, GitError>;

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

    /// The URLs git goes to for `remote`, after `insteadOf`, one per line
    /// of git's answer: the fetch's (`git ls-remote --get-url`), or with
    /// `push` every one a push goes to (`git remote get-url --push --all`,
    /// after `pushInsteadOf`). Read from config alone: no network.
    fn remote_urls(&self, repo: &Repo, remote: &str, push: bool) -> Result<Vec<String>, GitError>;

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

    /// The commit `rev` names.
    fn rev_parse(&self, repo: &Repo, rev: &str) -> Result<String, GitError>;

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
    fn fast_forward_to(&self, repo: &Repo, to: &str) -> Result<(), GitError>;

    /// Push `commit` to the upstream's branch through the `kluris-<remote>`
    /// alias (see the module docs), never prompting, by `deadline`. Runs
    /// the `pre-push` hook; what git printed, the hook's output included,
    /// goes to this process's stderr. Moves no remote-tracking ref.
    fn push(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        commit: &str,
        deadline: Instant,
    ) -> PushOutcome;
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

/// What `git push --porcelain` printed on stdout before its own report:
/// the `pre-push` hook's output, which git sends there, ahead of the
/// report's `To <destination>` line, its ref lines and `Done`.
fn before_porcelain(stdout: &[u8]) -> &[u8] {
    let mut starts = std::iter::once(0).chain(
        stdout
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i + 1),
    );
    match starts.rfind(|i| stdout[*i..].starts_with(b"To ")) {
        Some(report) => &stdout[..report],
        None => stdout,
    }
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

/// `git push --porcelain` refused a ref because the remote has commits we
/// do not: a `!` line with a plain rejection, or the remote's refusal when
/// its branch moved while the push ran (`incorrect old value provided`),
/// never a remote's own refusal (a hook, a protected branch).
fn remote_moved(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        line.starts_with('!')
            && ((line.contains("[rejected]")
                && ["fetch first", "non-fast-forward", "stale info"]
                    .iter()
                    .any(|why| line.contains(why)))
                || (line.contains("[remote rejected]")
                    && line.contains("incorrect old value provided")))
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

/// The `remote.<remote>.*` settings a push alias named `alias` takes
/// (`git config --null --list`: `<key>\n<value>` entries, or `<key>` for a
/// key with no value), as `-c` values in their order: every one but
/// `fetch` and `mirror`. A remote already configured under the alias's
/// name refuses the push, since its settings would add to the alias's.
fn alias_settings(list: &[u8], remote: &str, alias: &str) -> Result<Vec<String>, String> {
    let (own, taken) = (format!("remote.{remote}."), format!("remote.{alias}."));
    let mut settings = Vec::new();
    for entry in String::from_utf8_lossy(list).split('\0') {
        let (key, value) = match entry.split_once('\n') {
            Some((key, value)) => (key, Some(value)),
            None => (entry, None),
        };
        if key.starts_with(&taken) {
            return Err(format!(
                "a remote named {alias} is configured; Kluris pushes to {remote} through a remote of that name, so rename it (git remote rename {alias} <another name>) and run kluris sync again"
            ));
        }
        let Some(name) = key.strip_prefix(&own) else {
            continue;
        };
        if name.contains('.') || name == "fetch" || name == "mirror" {
            continue;
        }
        settings.push(match value {
            Some(value) => format!("remote.{alias}.{name}={value}"),
            None => format!("remote.{alias}.{name}"),
        });
    }
    Ok(settings)
}

impl SystemGit {
    /// git with `args` in `dir`, stopped with every process it started at
    /// `deadline`.
    pub(crate) fn bounded(
        &self,
        dir: &Path,
        args: &[&str],
        deadline: Instant,
    ) -> Result<Finished, RemoteError> {
        let error = |code, detail: String| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code,
            detail,
        };
        match run_until(self.command(Some(dir), args), deadline) {
            Ok(done) => Ok(done),
            Err(RunError::TimedOut) => Err(RemoteError::TimedOut),
            Err(RunError::Spawn(e)) => Err(RemoteError::Failed(error(None, e.to_string()))),
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
            Ok(done.stdout)
        } else {
            Err(RemoteError::Failed(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: done.status.code(),
                detail: String::from_utf8_lossy(&done.stderr).trim().to_string(),
            }))
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
            return Err(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: None,
                detail: format!("unexpected output: {text}"),
            });
        };
        // A relative common dir is relative to where git ran.
        Ok(GitDirs {
            own: PathBuf::from(own),
            common: repo.dir.join(common),
        })
    }

    fn count(&self, repo: &Repo, from: Option<&str>, to: &str) -> Result<usize, GitError> {
        let range = match from {
            Some(from) => format!("{from}..{to}"),
            None => to.to_string(),
        };
        let args = ["rev-list", "--count", &range];
        let out = self.run(&repo.dir, &args)?;
        first_line(&out).parse().map_err(|_| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code: None,
            detail: format!("not a count: {}", String::from_utf8_lossy(&out)),
        })
    }

    fn upstream(&self, repo: &Repo) -> Result<Option<Upstream>, GitError> {
        let Ok(branch) = self.run(&repo.dir, &["symbolic-ref", "--quiet", "HEAD"]) else {
            return Ok(None);
        };
        let branch = first_line(&branch);
        let format = "--format=%(upstream:remotename)%00%(upstream)%00%(upstream:remoteref)";
        let out = self.run(&repo.dir, &["for-each-ref", format, &branch])?;
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

    fn remote_urls(&self, repo: &Repo, remote: &str, push: bool) -> Result<Vec<String>, GitError> {
        let args: &[&str] = if push {
            &["remote", "get-url", "--push", "--all", remote]
        } else {
            &["ls-remote", "--get-url", remote]
        };
        let out = self.run(&repo.dir, args)?;
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

    fn rev_parse(&self, repo: &Repo, rev: &str) -> Result<String, GitError> {
        let spec = format!("{rev}^{{commit}}");
        let out = self.run(&repo.dir, &["rev-parse", "--verify", &spec])?;
        Ok(first_line(&out))
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
        let out = self
            .command(Some(&repo.dir), &args)
            .output()
            .map_err(|e| GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: None,
                detail: e.to_string(),
            })?;
        // 0: clean; 1: conflicts, listed after the tree; anything else fails.
        if !matches!(out.status.code(), Some(0 | 1)) {
            return Err(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: out.status.code(),
                detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
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
        // it is kept for the error.
        let out = self
            .command(Some(&repo.dir), &args)
            .output()
            .map_err(|e| GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: None,
                detail: e.to_string(),
            })?;
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(&out.stdout);
        let _ = stderr.write_all(&out.stderr);
        if out.status.success() {
            Ok(())
        } else {
            Err(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: out.status.code(),
                detail: one_line(&String::from_utf8_lossy(&out.stderr)),
            })
        }
    }

    fn push(
        &self,
        repo: &Repo,
        upstream: &Upstream,
        commit: &str,
        deadline: Instant,
    ) -> PushOutcome {
        let git = match self.remote(repo) {
            Ok(git) => git,
            Err(RemoteError::TimedOut) => return PushOutcome::Unreachable,
            Err(RemoteError::Failed(e)) => return PushOutcome::Failed(e.to_string()),
        };
        let list = match git.run(&repo.dir, &["config", "--null", "--list"]) {
            Ok(list) => list,
            Err(e) => return PushOutcome::Failed(e.to_string()),
        };
        let alias = format!("kluris-{}", upstream.remote);
        let settings = match alias_settings(&list, &upstream.remote, &alias) {
            Ok(settings) => settings,
            Err(why) => return PushOutcome::Failed(why),
        };
        let refspec = format!("{commit}:{}", upstream.remote_ref);
        let mut args: Vec<&str> = settings.iter().flat_map(|s| ["-c", s.as_str()]).collect();
        args.extend([
            "push",
            "--porcelain",
            "--no-recurse-submodules",
            &alias,
            &refspec,
        ]);
        let done = git.bounded(&repo.dir, &args, deadline);
        // The `pre-push` hook's output and git's stderr reach the user, on
        // stderr so `--json` stays one document; both are kept for the
        // outcome.
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
                let stdout = String::from_utf8_lossy(&done.stdout);
                let stderr = String::from_utf8_lossy(&done.stderr);
                if remote_moved(&stdout) {
                    PushOutcome::RemoteMoved
                } else if done.status.code() == Some(128) {
                    // git died (`die()`, 128) before any ref was sent: it
                    // could not talk to the remote. A hook or a ref the
                    // remote rejected ends with 1 and `failed to push some
                    // refs` (`builtin/push.c`).
                    let urls = git.remote_urls(repo, &upstream.remote, true);
                    if network_failure(&urls.unwrap_or_default(), &stderr) {
                        PushOutcome::Unreachable
                    } else {
                        PushOutcome::Refused(stderr.into_owned())
                    }
                } else {
                    let detail = [stdout.trim(), stderr.trim()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join("\n");
                    PushOutcome::Failed(detail)
                }
            }
        }
    }
}

impl TextMerger for SystemGit {
    fn merge_text(&self, base: &str, ours: &str, theirs: &str) -> Result<Option<String>, GitError> {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("kluris-merge-{}-{n}", std::process::id()));
        let error = |detail: String| GitError {
            args: vec!["merge-file".into()],
            code: None,
            detail,
        };
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
    let out = git
        .command(Some(dir), &args)
        .output()
        .map_err(|e| error(e.to_string()))?;
    match out.status.code() {
        Some(0) => String::from_utf8(out.stdout)
            .map(Some)
            .map_err(|_| error("the merged text is not UTF-8".into())),
        // The number of conflicts (git caps it at 127).
        Some(1..=127) => Ok(None),
        code => Err(GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code,
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        }),
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
        let moved = "To /r.git\n!\tHEAD:refs/heads/main\t[rejected] (fetch first)\nDone\n";
        assert!(remote_moved(moved));
        assert!(remote_moved(
            "!\tHEAD:refs/heads/main\t[rejected] (non-fast-forward)\n"
        ));
        assert!(remote_moved(
            "!\tHEAD:refs/heads/main\t[remote rejected] (incorrect old value provided)\n"
        ));
        assert!(!remote_moved(
            "!\tHEAD:refs/heads/main\t[remote rejected] (pre-receive hook declined)\n"
        ));
        assert!(!remote_moved("=\tHEAD:refs/heads/main\t[up to date]\n"));
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
        // Git for Windows after a terminated fetch (T-159's probe).
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
    fn a_push_alias_takes_every_remote_setting_but_fetch_and_mirror() {
        let list = b"core.bare\nfalse\0remote.origin.url\n/r.git\0remote.origin.fetch\n+refs/heads/*:refs/remotes/origin/*\0remote.origin.pushurl\na\0remote.origin.pushurl\nb\0remote.origin.mirror\nfalse\0remote.origin.lfsurl\nhttps://l/x\0remote.origin.skipfetchall\0remote.origin.x.y\n1\0remote.originx.url\n/x.git\0remote.other.url\n/o.git\0";
        assert_eq!(
            alias_settings(list, "origin", "kluris-origin").unwrap(),
            [
                "remote.kluris-origin.url=/r.git",
                "remote.kluris-origin.pushurl=a",
                "remote.kluris-origin.pushurl=b",
                "remote.kluris-origin.lfsurl=https://l/x",
                "remote.kluris-origin.skipfetchall",
            ]
        );
        // A remote whose name has a dot: only its own keys.
        let dotted = b"remote.a.b.url\n/ab.git\0remote.a.url\n/a.git\0";
        assert_eq!(
            alias_settings(dotted, "a.b", "kluris-a.b").unwrap(),
            ["remote.kluris-a.b.url=/ab.git"]
        );
        let taken = b"remote.origin.url\n/r.git\0remote.kluris-origin.url\n/k.git\0";
        let why = alias_settings(taken, "origin", "kluris-origin").unwrap_err();
        assert!(
            why.starts_with("a remote named kluris-origin is configured"),
            "{why}"
        );
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
        ] {
            assert_eq!(
                String::from_utf8_lossy(before_porcelain(stdout.as_bytes())),
                hook,
                "{stdout:?}"
            );
        }
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
