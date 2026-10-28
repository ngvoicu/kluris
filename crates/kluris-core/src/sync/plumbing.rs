//! The git steps of `kluris sync` beyond [`Plumbing`] and [`FreshGit`]:
//! the current branch's upstream, merge bases, the in-memory merge, the
//! fast-forward that moves the branch and work tree, and the push.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;

use super::fresh::{FreshError, FreshGit, prompt_env};
use super::merge::TextMerger;
use crate::git::{GitError, Plumbing, Repo, SystemGit, TreeEntry};

/// Where the current branch pulls from and pushes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    /// The remote (`%(upstream:remotename)`, `origin`).
    pub remote: String,
    /// The remote-tracking ref (`refs/remotes/origin/main`).
    pub refname: String,
    /// The branch on the remote (`refs/heads/main`).
    pub remote_ref: String,
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

/// The git steps of a sync (see the module docs).
pub trait SyncGit: Plumbing + FreshGit + TextMerger {
    /// The current branch's upstream on a remote; `None` on a detached HEAD
    /// or without one.
    fn tracking(&self, repo: &Repo) -> Result<Option<Tracking>, GitError>;

    /// The commit `rev` names.
    fn rev_parse(&self, repo: &Repo, rev: &str) -> Result<String, GitError>;

    /// `git merge-base --all`: every best common ancestor, git's order.
    fn merge_bases(&self, repo: &Repo, a: &str, b: &str) -> Result<Vec<String>, GitError>;

    /// `git merge-tree --write-tree` (git >= 2.38): the merge, in memory.
    fn merge_tree(&self, repo: &Repo, ours: &str, theirs: &str) -> Result<MergeTree, GitError>;

    /// The paths that differ between two trees (or commits), brain-relative.
    fn diff_names(&self, repo: &Repo, from: &str, to: &str) -> Result<Vec<String>, GitError>;

    /// Move the current branch, the index and the work tree to `to`, a
    /// descendant of `HEAD` (`git merge --ff-only --no-overwrite-ignore`,
    /// autostash off): git refuses if `HEAD` moved meanwhile or a local
    /// change, an untracked or an ignored file would be overwritten, and
    /// runs the `post-merge` hook.
    fn fast_forward_to(&self, repo: &Repo, to: &str) -> Result<(), GitError>;

    /// Push `HEAD` to the upstream branch, never prompting, by `deadline`.
    /// Runs the `pre-push` hook.
    fn push(&self, repo: &Repo, tracking: &Tracking, deadline: Instant) -> PushOutcome;
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

/// What git (or the ssh or curl it runs) says when the network is in the
/// way, each from its source:
///
/// - the host does not resolve: curl's `Could not resolve host: <host>`
///   and `Could not resolve proxy: <proxy>` (`lib/hostip.c`, curl 8.11),
///   which git prints after `unable to access '<url>': ` (`remote-curl.c`);
///   OpenSSH's `ssh: Could not resolve hostname <host>: <reason>` (`ssh.c`,
///   9.9p1); git's own `unable to look up <host> (port <port>) (<reason>)`
///   for `git://` (`connect.c`, `git_tcp_connect_sock`, 2.55);
/// - the connection is refused, times out or finds no route: curl's
///   `Failed to connect to <host> port <port> after <n> ms: <reason>`
///   (`lib/connect.c`); OpenSSH's `ssh: connect to host <host> port
///   <port>: <strerror>` (`sshconnect.c`, `ssh_connect_direct`); git's
///   `unable to connect to <host>:` with `errno=<strerror>` for `git://`
///   (`connect.c`); and the C library's words for the errors themselves,
///   `Connection refused`, `Network is unreachable`, `No route to host`;
/// - a time limit: curl's `Resolving timed out after`, `Connection timed
///   out after`, `Operation timed out after` (`lib/multi.c`), `name lookup
///   timed out` (`lib/hostip.c`) and `Timeout was reached`
///   (`lib/strerror.c`), and `ETIMEDOUT`'s `Connection timed out` (glibc,
///   Windows) or `Operation timed out` (macOS).
///
/// Anything else (`Permission denied (publickey).`, `Authentication
/// failed`, `does not appear to be a git repository`, `Repository not
/// found`, and git's `Could not read from remote repository.` after any
/// of them) is not network trouble. git translates its own messages
/// (`unable to look up`, `unable to connect to`); under another language
/// those read as a refusal, shown rather than hidden.
pub(crate) fn network_failure(stderr: &str) -> bool {
    const NETWORK: [&str; 11] = [
        "Could not resolve host",
        "Could not resolve proxy",
        "unable to look up ",
        "unable to connect to ",
        "Failed to connect to ",
        "ssh: connect to host ",
        "Connection refused",
        "Network is unreachable",
        "No route to host",
        "timed out",
        "Timeout was reached",
    ];
    NETWORK.iter().any(|words| stderr.contains(words))
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

impl SyncGit for SystemGit {
    fn tracking(&self, repo: &Repo) -> Result<Option<Tracking>, GitError> {
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
        Ok(Some(Tracking {
            remote: remote.to_string(),
            refname: refname.to_string(),
            remote_ref: remote_ref.to_string(),
        }))
    }

    fn rev_parse(&self, repo: &Repo, rev: &str) -> Result<String, GitError> {
        let spec = format!("{rev}^{{commit}}");
        let out = self.run(&repo.dir, &["rev-parse", "--verify", &spec])?;
        Ok(first_line(&out))
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
        let args = SystemGit::with_no_background(&[
            "-c",
            "merge.autostash=false",
            "-c",
            "rebase.autoStash=false",
            "merge",
            "--ff-only",
            "--no-overwrite-ignore",
            "--quiet",
            to,
        ]);
        // The post-merge hook's output reaches the user, all of it on
        // stderr so `--json` stays one document; so does git's own message
        // when it refuses.
        let mut cmd = self.command(Some(&repo.dir), &args);
        cmd.stdout(Stdio::from(std::io::stderr()))
            .stderr(Stdio::inherit());
        let status = cmd.status().map_err(|e| GitError {
            args: args.iter().map(|a| a.to_string()).collect(),
            code: None,
            detail: e.to_string(),
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(GitError {
                args: args.iter().map(|a| a.to_string()).collect(),
                code: status.code(),
                detail: "git refused the fast-forward (its message is above)".into(),
            })
        }
    }

    fn push(&self, repo: &Repo, tracking: &Tracking, deadline: Instant) -> PushOutcome {
        let refspec = format!("HEAD:{}", tracking.remote_ref);
        let ssh = self.run(&repo.dir, &["config", "--get", "core.sshCommand"]);
        let configured = ssh.is_ok_and(|out| !out.trim_ascii().is_empty());
        let env = prompt_env(|key| self.env_var(key), configured);
        let args = SystemGit::with_no_background(&[
            "push",
            "--porcelain",
            "--no-recurse-submodules",
            &tracking.remote,
            &refspec,
        ]);
        match self.bounded(&repo.dir, &args, &env, deadline) {
            Err(FreshError::TimedOut) => PushOutcome::Unreachable,
            Err(FreshError::Failed(e)) => PushOutcome::Failed(e.to_string()),
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
                    if network_failure(&stderr) {
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
        ] {
            assert!(network_failure(&network), "{network}");
        }
        for refused in [
            format!("Permission denied (publickey).{read}"),
            format!("fatal: '/gone.git' does not appear to be a git repository{read}"),
            "remote: Repository not found.\nfatal: repository 'https://github.com/a/b.git/' not found".into(),
            "fatal: Authentication failed for 'https://h/x.git/'".into(),
            "fatal: unable to access 'https://h/x.git/': The requested URL returned error: 403".into(),
            format!("ERROR: Permission to a/b.git denied to deploy-key.{read}"),
            "fatal: could not read Username for 'https://h': terminal prompts disabled".into(),
            String::new(),
        ] {
            assert!(!network_failure(&refused), "{refused}");
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
