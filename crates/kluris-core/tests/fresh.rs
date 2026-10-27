//! Fresh reads (`kluris_core::sync::fresh`): the decisions before `wake-up`
//! and `search` read a brain, against a fake git (the real git runs in
//! `crates/kluris-cli/tests/fresh_reads.rs`).

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kluris_core::git::GitError;
use kluris_core::sync::fresh::{
    BACKOFF, BACKOFF_FIRST, BACKOFF_MAX, Fresh, FreshError, FreshGit, FreshRequest, NO_HOOKS,
    STAMP, Upstream, WorkTree, fresh_read, prompt_env,
};
use tempfile::TempDir;

/// A scripted git: the brain's git directory is `<tmp>/gitdir`, and each
/// answer is set by the test. Every call is logged by name.
struct FakeGit {
    git_dir: Option<PathBuf>,
    /// The brain's folder relative to the work tree root (`sub/`).
    prefix: String,
    shallow: bool,
    upstream: Option<Upstream>,
    fetch: Result<(), FreshError>,
    /// The hooks path each git after `rev-parse` was given.
    hooks_paths: RefCell<Vec<PathBuf>>,
    calls: RefCell<Vec<String>>,
    deadlines: RefCell<Vec<Instant>>,
}

impl FakeGit {
    fn behind(tmp: &TempDir) -> FakeGit {
        let git_dir = tmp.path().join("gitdir");
        fs::create_dir_all(&git_dir).unwrap();
        FakeGit {
            git_dir: Some(git_dir),
            prefix: String::new(),
            shallow: false,
            upstream: Some(Upstream {
                remote: "origin".into(),
                refname: "refs/remotes/origin/main".into(),
                remote_ref: "refs/heads/main".into(),
            }),
            fetch: Ok(()),
            hooks_paths: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
            deadlines: RefCell::new(Vec::new()),
        }
    }

    fn log(&self, call: &str, deadline: Instant) {
        self.calls.borrow_mut().push(call.to_string());
        self.deadlines.borrow_mut().push(deadline);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn stamp(&self) -> PathBuf {
        self.git_dir.as_ref().unwrap().join(STAMP)
    }

    fn backoff(&self) -> PathBuf {
        self.git_dir.as_ref().unwrap().join(BACKOFF)
    }

    fn no_hooks(&self) -> PathBuf {
        self.git_dir.as_ref().unwrap().join(NO_HOOKS)
    }
}

impl FreshGit for FakeGit {
    fn git_paths(
        &self,
        _dir: &Path,
        names: &[&str],
        deadline: Instant,
    ) -> Result<Option<WorkTree>, FreshError> {
        self.log("git_paths", deadline);
        Ok(self.git_dir.as_ref().map(|dir| WorkTree {
            prefix: self.prefix.clone(),
            shallow: self.shallow,
            paths: names.iter().map(|n| dir.join(n)).collect(),
        }))
    }

    fn upstream(
        &self,
        _dir: &Path,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<Option<Upstream>, FreshError> {
        self.log("upstream", deadline);
        self.hooks_paths.borrow_mut().push(no_hooks.to_path_buf());
        Ok(self.upstream.clone())
    }

    fn fetch(&self, _dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError> {
        self.log(&format!("fetch {remote}"), deadline);
        self.fetch.clone()
    }

    fn fetch_objects(
        &self,
        _dir: &Path,
        upstream: &Upstream,
        no_hooks: &Path,
        deadline: Instant,
    ) -> Result<(), FreshError> {
        self.hooks_paths.borrow_mut().push(no_hooks.to_path_buf());
        self.log(
            &format!("fetch_objects {} {}", upstream.remote, upstream.remote_ref),
            deadline,
        );
        self.fetch.clone()
    }

    fn is_ancestor(
        &self,
        _dir: &Path,
        ancestor: &str,
        descendant: &str,
        deadline: Instant,
    ) -> Result<bool, FreshError> {
        self.log(&format!("is_ancestor {ancestor} {descendant}"), deadline);
        Ok(false)
    }
}

fn brain(tmp: &TempDir) -> PathBuf {
    let brain = tmp.path().join("brain");
    fs::create_dir_all(&brain).unwrap();
    brain
}

fn request(brain: &Path) -> FreshRequest<'_> {
    FreshRequest::new(brain, false)
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn the_brain_is_fetched_and_nothing_else_runs() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let git = FakeGit::behind(&tmp);
    let started = Instant::now();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Fetched);
    let finished = Instant::now();
    assert_eq!(
        git.calls(),
        [
            "git_paths",
            "upstream",
            "fetch_objects origin refs/heads/main"
        ]
    );
    assert!(git.stamp().is_file(), "stamped");
    // One deadline for the whole step, 3 s from its start.
    let deadlines = git.deadlines.borrow();
    assert!(deadlines.windows(2).all(|d| d[0] == d[1]));
    let limit = Duration::from_secs(3);
    assert!(deadlines[0] >= started + limit);
    assert!(deadlines[0] <= finished + limit);
}

#[test]
fn turned_off_by_the_environment_or_kluris_yml_without_running_git() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let git = FakeGit::behind(&tmp);
    assert_eq!(
        fresh_read(&git, &FreshRequest::new(&brain, true)),
        Fresh::Disabled
    );
    assert!(git.calls().is_empty());

    fs::write(
        brain.join("kluris.yml"),
        "name: b\nsync:\n  auto_fetch: false\n",
    )
    .unwrap();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Disabled);
    assert!(git.calls().is_empty());
    assert!(!git.stamp().exists());

    // `true`, a kluris.yml that does not read, and none at all: on.
    for text in [
        Some("name: b\nsync:\n  auto_fetch: true\n"),
        Some("name: [unclosed\n"),
        None,
    ] {
        match text {
            Some(text) => fs::write(brain.join("kluris.yml"), text).unwrap(),
            None => fs::remove_file(brain.join("kluris.yml")).unwrap(),
        }
        let _ = fs::remove_file(git.stamp());
        assert_eq!(
            fresh_read(&git, &request(&brain)),
            Fresh::Fetched,
            "{text:?}"
        );
    }
}

#[test]
fn outside_a_work_tree_nothing_runs() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.git_dir = None;
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::NotARepo);
    assert_eq!(git.calls(), ["git_paths"]);
}

#[test]
fn a_brain_below_its_work_tree_root_is_never_touched() {
    // A brain nested in a larger repo: no fetch, no stamp.
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.prefix = "docs/brain/".into();
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Nested);
    assert_eq!(git.calls(), ["git_paths"]);
    assert!(!git.stamp().exists());
}

#[test]
fn a_shallow_brain_is_never_fetched() {
    // Deepening it would write `.git/shallow` under `shallow.lock`.
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.shallow = true;
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Shallow);
    assert_eq!(git.calls(), ["git_paths"]);
    assert!(!git.stamp().exists());
}

#[test]
fn a_fetch_within_the_last_minute_is_not_repeated() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let git = FakeGit::behind(&tmp);
    let now = SystemTime::now();
    fs::create_dir_all(git.stamp().parent().unwrap()).unwrap();
    for (age, throttled) in [
        (0i64, true),
        (59, true),
        (61, false),
        // A stamp from the future (the clock went back) never throttles.
        (-3600, false),
    ] {
        let at = secs(now) as i64 - age;
        fs::write(git.stamp(), format!("{at}\n")).unwrap();
        git.calls.borrow_mut().clear();
        let mut req = request(&brain);
        req.now = now;
        let fresh = fresh_read(&git, &req);
        if throttled {
            assert_eq!(fresh, Fresh::Throttled, "age {age}");
            assert_eq!(git.calls(), ["git_paths"], "age {age}");
        } else {
            assert_eq!(fresh, Fresh::Fetched, "age {age}");
            assert_eq!(
                fs::read_to_string(git.stamp()).unwrap(),
                format!("{}\n", secs(now)),
                "age {age}"
            );
        }
    }
    // A stamp that does not read is no stamp.
    fs::write(git.stamp(), "yesterday").unwrap();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Fetched);
}

#[test]
fn an_operation_in_progress_or_an_open_conflict_does_not_stop_the_fetch() {
    // A fetch moves only the remote-tracking refs: a merge, a rebase or an
    // open `kluris sync` conflict (whose commits are recorded) is unaffected.
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    for marker in [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "kluris-sync/pending.json",
    ] {
        let git = FakeGit::behind(&tmp);
        let _ = fs::remove_file(git.stamp());
        let path = git.git_dir.as_ref().unwrap().join(marker);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if marker.starts_with("rebase") {
            fs::create_dir_all(&path).unwrap();
        } else {
            fs::write(&path, "0123\n").unwrap();
        }
        assert_eq!(
            fresh_read(&git, &request(&brain)),
            Fresh::Fetched,
            "{marker}"
        );
        assert_eq!(
            git.calls(),
            [
                "git_paths",
                "upstream",
                "fetch_objects origin refs/heads/main"
            ],
            "{marker}"
        );
        if marker.starts_with("rebase") {
            fs::remove_dir_all(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn without_an_upstream_nothing_is_fetched_and_the_check_is_throttled() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let mut git = FakeGit::behind(&tmp);
    git.upstream = None;
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::NoUpstream);
    assert!(!git.calls().iter().any(|c| c.starts_with("fetch")));
    assert!(git.stamp().exists());
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Throttled);
}

#[test]
fn a_failed_or_timed_out_fetch_ends_the_step_and_is_throttled() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    for (fetch, expected) in [
        (
            Err(FreshError::Failed(GitError {
                args: vec!["fetch".into()],
                code: Some(128),
                detail: "fatal: unable to access".into(),
            })),
            Fresh::FetchFailed,
        ),
        (Err(FreshError::TimedOut), Fresh::TimedOut),
    ] {
        let mut git = FakeGit::behind(&tmp);
        let _ = fs::remove_file(git.stamp());
        git.fetch = fetch;
        assert_eq!(fresh_read(&git, &request(&brain)), expected);
        assert_eq!(
            git.calls(),
            [
                "git_paths",
                "upstream",
                "fetch_objects origin refs/heads/main"
            ]
        );
        // Stamped before the fetch: an offline brain costs this once a minute.
        assert!(git.stamp().exists());
        assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Throttled);
    }
}

#[test]
fn prompts_are_turned_off_without_overriding_the_users_ssh_command() {
    let none = |_: &str| -> Option<OsString> { None };
    assert_eq!(
        prompt_env(none, false),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GCM_INTERACTIVE", "never"),
            ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"),
        ]
    );
    // The user's own choices stay: GCM_INTERACTIVE, and an ssh command set
    // by GIT_SSH_COMMAND, GIT_SSH or core.sshCommand.
    let set = |key: &'static str| move |k: &str| (k == key).then(|| OsString::from("mine"));
    assert_eq!(
        prompt_env(set("GCM_INTERACTIVE"), false),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"),
        ]
    );
    for key in ["GIT_SSH_COMMAND", "GIT_SSH"] {
        assert_eq!(
            prompt_env(set(key), false),
            [
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GIT_ASKPASS", ""),
                ("GCM_INTERACTIVE", "never"),
            ],
            "{key}"
        );
    }
    assert_eq!(
        prompt_env(none, true),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GCM_INTERACTIVE", "never"),
        ]
    );
    // An empty variable is not a setting.
    let empty = |k: &str| (k == "GIT_SSH_COMMAND").then(OsString::new);
    assert_eq!(prompt_env(empty, false).len(), 4);
}

#[test]
fn every_git_after_rev_parse_runs_with_hooks_off() {
    // `core.hooksPath` points at `<git dir>/kluris-sync/no-hooks`, which is
    // never made; `rev-parse`, which finds the git dir, runs no hook.
    let tmp = TempDir::new().unwrap();
    let git = FakeGit::behind(&tmp);
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Fetched);
    assert_eq!(*git.hooks_paths.borrow(), [git.no_hooks(), git.no_hooks()]);
    assert!(!git.no_hooks().exists());
}

#[test]
fn something_at_the_no_hooks_path_stops_the_fetch() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let git = FakeGit::behind(&tmp);
    fs::create_dir_all(git.no_hooks()).unwrap();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::HooksPathTaken);
    assert_eq!(git.calls(), ["git_paths"]);
    assert!(!git.stamp().exists());
    fs::remove_dir(git.no_hooks()).unwrap();
    fs::write(git.no_hooks(), "").unwrap();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::HooksPathTaken);
}

/// A read of `brain` at `at` (Unix seconds).
fn read_at(git: &FakeGit, brain: &Path, at: u64) -> Fresh {
    git.calls.borrow_mut().clear();
    let mut req = request(brain);
    req.now = UNIX_EPOCH + Duration::from_secs(at);
    fresh_read(git, &req)
}

#[test]
fn a_fetch_stopped_at_the_deadline_backs_off_an_hour_doubling_to_a_day() {
    assert_eq!(BACKOFF_FIRST, Duration::from_secs(3600));
    assert_eq!(BACKOFF_MAX, Duration::from_secs(24 * 3600));
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let mut git = FakeGit::behind(&tmp);
    git.fetch = Err(FreshError::TimedOut);
    let mut at = 1_000_000_000u64;
    for window in [3600u64, 7200, 14400, 28800, 57600, 86400, 86400] {
        assert_eq!(read_at(&git, &brain, at), Fresh::TimedOut, "at {at}");
        assert_eq!(
            fs::read_to_string(git.backoff()).unwrap(),
            format!("{} {window}\n", at + window)
        );
        // Every read inside the window (past the throttle) fetches nothing.
        for later in [61, window / 2, window - 1] {
            assert_eq!(
                read_at(&git, &brain, at + later),
                Fresh::BackedOff,
                "{later} s into {window}"
            );
            assert_eq!(git.calls(), ["git_paths"]);
        }
        at += window;
    }
}

#[test]
fn a_fetch_that_succeeds_ends_the_back_off() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let mut git = FakeGit::behind(&tmp);
    let at = 1_000_000_000u64;
    fs::create_dir_all(git.backoff().parent().unwrap()).unwrap();
    // A window of two hours that has run out.
    fs::write(git.backoff(), format!("{} 7200\n", at - 1)).unwrap();
    assert_eq!(read_at(&git, &brain, at), Fresh::Fetched);
    assert!(!git.backoff().exists());
    // The next stop starts over at one hour.
    git.fetch = Err(FreshError::TimedOut);
    assert_eq!(read_at(&git, &brain, at + 61), Fresh::TimedOut);
    assert_eq!(
        fs::read_to_string(git.backoff()).unwrap(),
        format!("{} 3600\n", at + 61 + 3600)
    );
    // A failure that is not the deadline (offline) leaves it as it is.
    git.fetch = Err(FreshError::Failed(GitError {
        args: vec!["fetch".into()],
        code: Some(128),
        detail: "fatal: unable to access".into(),
    }));
    fs::remove_file(git.backoff()).unwrap();
    assert_eq!(read_at(&git, &brain, at + 200), Fresh::FetchFailed);
    assert!(!git.backoff().exists());
}

#[test]
fn a_back_off_that_does_not_read_or_lies_far_ahead_never_blocks_longer() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let git = FakeGit::behind(&tmp);
    let at = 1_000_000_000u64;
    fs::create_dir_all(git.backoff().parent().unwrap()).unwrap();
    for (text, blocked) in [
        ("yesterday\n".to_string(), false),
        (String::new(), false),
        (format!("{}\n", at + 100), false),
        // Further ahead than its own window (the clock went back).
        (format!("{} 3600\n", at + 3601), false),
        // A window past the cap counts as the cap.
        (format!("{} 999999999\n", at + 86_400), true),
        (format!("{} 999999999\n", at + 86_401), false),
        (format!("{} 3600\n", at + 3600), true),
    ] {
        fs::write(git.backoff(), &text).unwrap();
        let _ = fs::remove_file(git.stamp());
        let fresh = read_at(&git, &brain, at);
        if blocked {
            assert_eq!(fresh, Fresh::BackedOff, "{text:?}");
        } else {
            assert_eq!(fresh, Fresh::Fetched, "{text:?}");
        }
    }
}
