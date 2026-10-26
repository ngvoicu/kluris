//! The git steps of `kluris sync` beyond [`Plumbing`] and [`FreshGit`]:
//! the current branch's upstream, merge bases, the in-memory merge, the
//! fast-forward that moves the branch and work tree, and the push.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use super::fresh::{FreshError, FreshGit, prompt_env};
use super::merge::TextMerger;
use crate::git::{GitError, Plumbing, Repo, SystemGit};

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
}

/// How a push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    /// Rejected because the remote branch moved: fetch, merge, push again.
    RemoteMoved,
    /// The remote could not be reached in time.
    Unreachable,
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
    /// descendant of `HEAD` (`git merge --ff-only`): git refuses if `HEAD`
    /// moved meanwhile or a local change would be overwritten, and runs the
    /// `post-merge` hook.
    fn fast_forward_to(&self, repo: &Repo, to: &str) -> Result<(), GitError>;

    /// Push `HEAD` to the upstream branch, never prompting, by `deadline`.
    /// Runs the `pre-push` hook.
    fn push(&self, repo: &Repo, tracking: &Tracking, deadline: Instant) -> PushOutcome;
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
        let args = [
            "merge-tree",
            "--write-tree",
            "-z",
            "--name-only",
            "--no-messages",
            ours,
            theirs,
        ];
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
        let mut names = nul_separated(&out.stdout).into_iter();
        let tree = names.next().unwrap_or_default();
        Ok(MergeTree {
            tree,
            conflicted: names.collect(),
        })
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
        let args = SystemGit::with_no_background(&["merge", "--ff-only", "--quiet", to]);
        self.run(&repo.dir, &args).map(drop)
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
                if remote_moved(&stdout) {
                    PushOutcome::RemoteMoved
                } else {
                    let stderr = String::from_utf8_lossy(&done.stderr);
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
}
