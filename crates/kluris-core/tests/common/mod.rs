//! A bare remote and two clones in temp dirs, "ours" (the brain being
//! synced) and "theirs" (a teammate), for the sync and resolve tests; the
//! frozen format corpora in [`corpus`].

#![allow(dead_code)]

pub mod corpus;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use kluris_core::brain::store::LocalGitStore;
use kluris_core::dream::{DreamOptions, dream};
use kluris_core::git::SystemGit;
use kluris_core::sync::claim::{COMMAND_BOUND, ClaimError, TreeClaim};
use kluris_core::sync::commit::{CommitRequest, commit};
use kluris_core::sync::engine::{self, Context};
use kluris_core::sync::land::failed;
use kluris_core::sync::pending::{Found, Pending};
use kluris_core::sync::resolve::{Choice, ResolveError, ResolveReport, ResolveRequest, resolve_in};
use kluris_core::sync::session::{self, Brain, Intent, SessionEnv};
use kluris_core::sync::{LeftAsIs, LeftReason, PENDING, SyncReport, SyncRequest};
use kluris_core::yaml::Date;
use tempfile::TempDir;

/// One `kluris resolve` call: the choice, the request of the engine it is
/// made with, and how long it waits for the tree claim another command
/// holds.
#[derive(Debug, Clone)]
pub struct Resolve<'a> {
    pub path: &'a str,
    pub merge: &'a str,
    pub choice: Choice,
    pub sync: SyncRequest<'a>,
    pub tree_bound: Duration,
}

/// What a [`resolve`] call did: the choice's report, then, once the merge
/// landed, what the sync after it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: String,
    pub choice: &'static str,
    /// The paths still without a choice; after the merge, those of a new
    /// conflict the sync met.
    pub remaining: Vec<String>,
    pub committed: bool,
    pub commit: Option<String>,
    pub pushed: bool,
    /// Why the sync after the merge did not push, unless offline.
    pub sync_error: Option<String>,
}

/// The choice recorded under the exclusive tree claim ([`resolve_in`]), as
/// the command's explicit session holds it; once the merge landed, the
/// claim let go and the brain synced by an explicit session
/// ([`explicit`]).
pub fn resolve(root: &Path, git: &SystemGit, call: &Resolve) -> Result<Resolved, ResolveError> {
    let ctx = engine::open(root, git, &call.sync)?;
    let claim = TreeClaim::exclusive(&ctx.dirs().own, call.tree_bound).map_err(|e| {
        ResolveError::Left(match e {
            ClaimError::Busy => LeftAsIs::because(
                LeftReason::Busy,
                "another kluris command is changing this brain's files; run again once it is done",
            ),
            ClaimError::Io(e) => failed(format!("cannot claim the work tree: {e}")),
        })
    })?;
    let request = ResolveRequest {
        path: call.path,
        merge: call.merge,
        choice: call.choice.clone(),
    };
    let ResolveReport {
        path,
        choice,
        remaining,
        committed,
        commit,
        warning: _,
    } = resolve_in(&ctx, &claim, &request)?;
    let mut done = Resolved {
        path,
        choice,
        remaining,
        committed,
        commit,
        pushed: false,
        sync_error: None,
    };
    if committed {
        drop(claim);
        let synced = explicit(root, &env(git, call.sync.no_verify));
        done.pushed = synced.pushed;
        done.remaining = synced.conflicts.into_iter().map(|c| c.path).collect();
        done.sync_error = synced
            .left_as_is
            .filter(|left| left.reason != LeftReason::Offline)
            .map(|left| left.to_string());
    }
    Ok(done)
}

/// The environment a command's sessions share, with `git` and no
/// background push.
pub fn env(git: &SystemGit, no_verify: bool) -> SessionEnv {
    SessionEnv {
        git: git.clone(),
        no_verify,
        now: SystemTime::now(),
        program: None,
    }
}

fn brain(root: &Path) -> Brain<'_> {
    Brain {
        root,
        maps: options(),
    }
}

/// What `kluris sync` reports for the brain at `root`: an explicit session
/// opened (its pull), then closed (its push).
pub fn explicit(root: &Path, env: &SessionEnv) -> SyncReport {
    session::close(session::open(brain(root), Intent::Explicit, env))
        .expect("an explicit session reports")
}

/// What a read reports for the brain at `root` an hour from now, past the
/// throttle of every session the test ran before: it never waits for the
/// remote claim, and waits [`kluris_core::sync::claim::READ_BOUND`] for the
/// tree claim; `None` when it has nothing to say.
pub fn read_session(root: &Path, git: &SystemGit) -> Option<SyncReport> {
    let env = SessionEnv {
        now: SystemTime::now() + Duration::from_secs(3600),
        ..env(git, false)
    };
    session::close(session::open(brain(root), Intent::Read, &env))
}

/// The waiting merge recorded in the brain at `root`, whatever commit it
/// was found for; `None` when none is (or it does not read).
pub fn waiting_merge(root: &Path, _git: &SystemGit) -> Option<Pending> {
    match Pending::read(&root.join(".git").join(PENDING)) {
        Found::Waiting(pending) => Some(pending),
        _ => None,
    }
}

pub const ISOLATED_VARS: [&str; 8] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "KLURIS_AGENT",
];

/// How `child`, a process the test started, ended, waited for up to
/// `patience`; past it the child is killed (by its handle) and the test
/// fails.
pub fn wait_bounded(child: &mut Child, what: &str, patience: Duration) -> ExitStatus {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if start.elapsed() < patience => thread::sleep(Duration::from_millis(10)),
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{what}: did not end within {patience:?} ({outcome:?})");
            }
        }
    }
}

/// A child the test started, ended (killed, then reaped) whenever the
/// guard goes, however the test ends; waited for only within a bound
/// ([`Owned::wait`]).
pub struct Owned(pub Child);

impl Owned {
    /// Start `cmd`, owned.
    pub fn spawn(cmd: &mut Command) -> Owned {
        Owned(cmd.spawn().unwrap())
    }

    /// How the child ended, within `patience` ([`wait_bounded`]).
    pub fn wait(&mut self, what: &str, patience: Duration) -> ExitStatus {
        wait_bounded(&mut self.0, what, patience)
    }
}

impl std::ops::Deref for Owned {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Owned {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub fn today() -> Date {
    Date::new(2026, 1, 1).unwrap()
}

pub fn options() -> DreamOptions<'static> {
    DreamOptions {
        name: "demo",
        description: "A demo brain",
        today: today(),
    }
}

/// A neuron with the given frontmatter lines (between `created` and
/// `updated`, sorted as the writer sorts them) and body.
pub fn neuron(extra: &str, body: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\n{extra}parent: ./map.md\nupdated: '2026-01-01'\n---\n\n{body}"
    )
}

/// One world at a time: each runs several git processes per step, and the
/// Windows runner caps a run at 64 processes.
pub static SERIAL: Mutex<()> = Mutex::new(());

pub struct World {
    /// `None` for one of [`World::several`], whose caller holds it.
    pub _serial: Option<MutexGuard<'static, ()>>,
    pub dir: TempDir,
    pub remote: PathBuf,
    pub ours: PathBuf,
    pub theirs: PathBuf,
}

impl World {
    pub fn new() -> World {
        World::build(Some(SERIAL.lock().unwrap_or_else(|e| e.into_inner())))
    }

    /// `count` worlds at once, for a test that runs them side by side, and
    /// the lock that keeps every other world out meanwhile.
    pub fn several(count: usize) -> (MutexGuard<'static, ()>, Vec<World>) {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        (serial, (0..count).map(|_| World::build(None)).collect())
    }

    fn build(serial: Option<MutexGuard<'static, ()>>) -> World {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        let w = World {
            _serial: serial,
            remote: dir.path().join("remote.git"),
            ours: dir.path().join("ours"),
            theirs: dir.path().join("theirs"),
            dir,
        };
        w.run(
            w.dir.path(),
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
        );
        fs::create_dir_all(&w.ours).unwrap();
        w.run(&w.ours, &["init", "-q", "-b", "main"]);
        w.configure(&w.ours, "Ana Tester", "ana@example.com");
        w.write(&w.ours, ".gitignore", "kluris.yml\n");
        w.write(
            &w.ours,
            "kluris.yml",
            "name: demo\ndescription: A demo brain\n",
        );
        w.write(
            &w.ours,
            "glossary.md",
            "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
        );
        w.write(
            &w.ours,
            "notes/first.md",
            &neuron("", "# First\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n"),
        );
        w.write(&w.ours, "notes/keep.md", &neuron("", "# Keep\n"));
        w.write(&w.ours, "ideas/idea.md", &neuron("", "# Idea\n"));
        let mut store = LocalGitStore::open(&w.ours);
        dream(&mut store, &w.git(), &options());
        w.run(&w.ours, &["add", "-A"]);
        w.run(&w.ours, &["commit", "-q", "-m", "init"]);
        let url = w.remote.to_string_lossy().into_owned();
        w.run(&w.ours, &["remote", "add", "origin", &url]);
        w.run(&w.ours, &["push", "-q", "-u", "origin", "main"]);
        w.clone_as(&w.theirs, "Bob Teammate", "bob@example.com");
        w
    }

    pub fn clone_as(&self, at: &Path, name: &str, email: &str) {
        let url = self.remote.to_string_lossy().into_owned();
        let to = at.to_string_lossy().into_owned();
        self.run(self.dir.path(), &["clone", "-q", &url, &to]);
        self.configure(at, name, email);
        self.write(at, "kluris.yml", "name: demo\ndescription: A demo brain\n");
    }

    pub fn configure(&self, at: &Path, name: &str, email: &str) {
        self.run(at, &["config", "user.name", name]);
        self.run(at, &["config", "user.email", email]);
        self.run(at, &["config", "core.autocrlf", "false"]);
    }

    /// The git Kluris runs, with no user or system config.
    pub fn git(&self) -> SystemGit {
        let mut git = SystemGit::new()
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path());
        for var in ISOLATED_VARS {
            git = git.env_remove(var);
        }
        git
    }

    pub fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path());
        for var in ISOLATED_VARS {
            cmd.env_remove(var);
        }
        cmd
    }

    pub fn run(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.command(cwd, args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn path(at: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part))
    }

    pub fn write(&self, at: &Path, rel: &str, text: &str) {
        let path = World::path(at, rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn read(&self, at: &Path, rel: &str) -> String {
        fs::read_to_string(World::path(at, rel)).unwrap()
    }

    /// Commit `rels` in a clone as `kluris commit` does (maps regenerated).
    pub fn commit(&self, at: &Path, rels: &[&str]) {
        let paths: Vec<String> = rels.iter().map(|r| r.to_string()).collect();
        let report = commit(
            at,
            &self.git(),
            &CommitRequest {
                brain: "demo",
                paths: &paths,
                message: None,
                agent: None,
                no_verify: false,
                maps: options(),
            },
        )
        .unwrap();
        assert!(report.commit.is_some(), "nothing committed for {rels:?}");
    }

    /// The teammate writes, commits and pushes.
    pub fn theirs_push(&self, files: &[(&str, &str)], deleted: &[&str]) {
        for (rel, text) in files {
            self.write(&self.theirs, rel, text);
        }
        for rel in deleted {
            let path = World::path(&self.theirs, rel);
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
        let mut rels: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
        rels.extend_from_slice(deleted);
        self.commit(&self.theirs, &rels);
        self.run(&self.theirs, &["push", "-q"]);
    }

    /// We write and commit (not pushed).
    pub fn ours_commit(&self, files: &[(&str, &str)], deleted: &[&str]) {
        for (rel, text) in files {
            self.write(&self.ours, rel, text);
        }
        for rel in deleted {
            fs::remove_file(World::path(&self.ours, rel)).unwrap();
        }
        let mut rels: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
        rels.extend_from_slice(deleted);
        self.commit(&self.ours, &rels);
    }

    pub fn request(&self) -> SyncRequest<'static> {
        SyncRequest {
            no_verify: false,
            maps: options(),
            network_time_limit: Duration::from_secs(60),
        }
    }

    /// A `kluris resolve` call in "ours" with the engine's request and the
    /// command's wait for the tree claim.
    pub fn resolve_call<'a>(&self, path: &'a str, merge: &'a str, choice: Choice) -> Resolve<'a> {
        Resolve {
            path,
            merge,
            choice,
            sync: self.request(),
            tree_bound: COMMAND_BOUND,
        }
    }

    /// The engine's context for "ours".
    pub fn open(&self) -> Context<'static, SystemGit> {
        engine::open(&self.ours, &self.git(), &self.request()).unwrap()
    }

    /// Pull "ours" as an explicit command's session opens it: the open's
    /// report, the session ended before its close would push.
    pub fn pull(&self) -> SyncReport {
        self.pull_at(&self.ours, &self.git())
    }

    /// [`World::pull`] of the brain at `root` with `git`.
    pub fn pull_at(&self, root: &Path, git: &SystemGit) -> SyncReport {
        let opened = session::open(brain(root), Intent::Explicit, &env(git, false));
        opened
            .report()
            .cloned()
            .expect("an explicit session reports")
    }

    /// Pull and push "ours", as `kluris sync` does ([`explicit`]).
    pub fn sync(&self) -> SyncReport {
        self.sync_at(&self.ours, &self.git())
    }

    /// [`World::sync`] of the brain at `root` with `git`.
    pub fn sync_at(&self, root: &Path, git: &SystemGit) -> SyncReport {
        explicit(root, &env(git, false))
    }

    /// [`World::sync`] with `--no-verify`.
    pub fn sync_no_verify(&self) -> SyncReport {
        explicit(&self.ours, &env(&self.git(), true))
    }

    /// What a read of "ours" reports ([`read_session`]).
    pub fn read_session(&self) -> Option<SyncReport> {
        read_session(&self.ours, &self.git())
    }

    /// The id of "ours"' waiting merge; empty when none is open.
    pub fn merge_id(&self) -> String {
        waiting_merge(&self.ours, &self.git())
            .map(|waiting| waiting.id())
            .unwrap_or_default()
    }

    pub fn head(&self, at: &Path) -> String {
        self.run(at, &["rev-parse", "HEAD"]).trim().to_string()
    }

    pub fn remote_head(&self) -> String {
        self.run(&self.remote, &["rev-parse", "main"])
            .trim()
            .to_string()
    }

    pub fn show(&self, at: &Path, rev: &str, rel: &str) -> String {
        self.run(at, &["show", &format!("{rev}:{rel}")])
    }

    pub fn status(&self, at: &Path) -> String {
        self.run(at, &["status", "--porcelain", "--untracked-files=all"])
    }

    /// Every file in a clone (not `.git`), with its bytes.
    pub fn files(&self, at: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut pending = vec![at.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if entry.file_name() == ".git" {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let rel = path
                        .strip_prefix(at)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    /// HEAD, the index, the status and every file of "ours", and the remote.
    pub fn snapshot(&self) -> (String, String, String, BTreeMap<String, Vec<u8>>, String) {
        (
            self.head(&self.ours),
            self.run(&self.ours, &["ls-files", "--stage"]),
            self.status(&self.ours),
            self.files(&self.ours),
            self.remote_head(),
        )
    }

    pub fn parents(&self, at: &Path, rev: &str) -> usize {
        self.run(at, &["rev-list", "--parents", "-n", "1", rev])
            .split_whitespace()
            .count()
            - 1
    }

    pub fn hook(&self, at: &Path, name: &str, body: &str) {
        let dir = at.join(".git").join("hooks");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// Why the brain was left as it is, as JSON spells it; `None` when nothing
/// was left.
pub fn reason(r: &SyncReport) -> Option<&'static str> {
    r.left_as_is.as_ref().map(|left| left.reason.as_str())
}

/// The detail of why the brain was left as it is.
pub fn detail(r: &SyncReport) -> &str {
    r.left_as_is
        .as_ref()
        .and_then(|left| left.detail.as_deref())
        .unwrap_or_default()
}

/// Nothing was left: pulled, merged or up to date.
pub fn clean(r: &SyncReport) -> bool {
    r.left_as_is.is_none() && r.conflicts.is_empty()
}

/// Left as it is for `reason`.
pub fn left(r: &SyncReport, reason: LeftReason) -> bool {
    r.left_as_is.as_ref().is_some_and(|l| l.reason == reason)
}

pub fn no_markers(text: &str) {
    for marker in ["<<<<<<<", ">>>>>>>", "|||||||"] {
        assert!(
            !text.contains(marker),
            "conflict marker {marker} in:\n{text}"
        );
    }
    assert!(
        !text.lines().any(|l| l == "======="),
        "conflict marker ======= in:\n{text}"
    );
}

/// No file of a clone holds a conflict marker.
pub fn no_markers_anywhere(w: &World, at: &Path) {
    for (rel, bytes) in w.files(at) {
        if let Ok(text) = String::from_utf8(bytes) {
            assert!(
                !text.contains("<<<<<<<") && !text.contains(">>>>>>>"),
                "conflict marker in {rel}:\n{text}"
            );
        }
    }
}
