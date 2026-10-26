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
    Fresh, FreshError, FreshGit, FreshRequest, STAMP, Upstream, WorkTree, fresh_read, prompt_env,
};
use tempfile::TempDir;

/// A scripted git: the brain's git directory is `<tmp>/gitdir`, and each
/// answer is set by the test. Every call is logged by name.
struct FakeGit {
    git_dir: Option<PathBuf>,
    /// The brain's folder relative to the work tree root (`sub/`).
    prefix: String,
    upstream: Option<Upstream>,
    dirty: bool,
    fetch: Result<(), FreshError>,
    fetch_takes: Duration,
    /// `(ancestor, descendant)` pairs that are ancestors.
    ancestors: Vec<(&'static str, &'static str)>,
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
            upstream: Some(Upstream {
                remote: "origin".into(),
                refname: "refs/remotes/origin/main".into(),
            }),
            dirty: false,
            fetch: Ok(()),
            fetch_takes: Duration::ZERO,
            ancestors: vec![("HEAD", "refs/remotes/origin/main")],
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
            paths: names.iter().map(|n| dir.join(n)).collect(),
        }))
    }

    fn upstream(&self, _dir: &Path, deadline: Instant) -> Result<Option<Upstream>, FreshError> {
        self.log("upstream", deadline);
        Ok(self.upstream.clone())
    }

    fn has_tracked_changes(&self, _dir: &Path, deadline: Instant) -> Result<bool, FreshError> {
        self.log("has_tracked_changes", deadline);
        Ok(self.dirty)
    }

    fn fetch(&self, _dir: &Path, remote: &str, deadline: Instant) -> Result<(), FreshError> {
        self.log(&format!("fetch {remote}"), deadline);
        std::thread::sleep(self.fetch_takes);
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
        Ok(self.ancestors.contains(&(ancestor, descendant)))
    }

    fn fast_forward(&self, _dir: &Path, to: &str, deadline: Instant) -> Result<(), FreshError> {
        self.log(&format!("fast_forward {to}"), deadline);
        Ok(())
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
fn behind_its_upstream_the_brain_is_fetched_and_fast_forwarded() {
    let tmp = TempDir::new().unwrap();
    let git = FakeGit::behind(&tmp);
    let brain = brain(&tmp);
    let started = Instant::now();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::FastForwarded);
    let calls = git.calls();
    assert_eq!(calls.iter().filter(|c| *c == "fetch origin").count(), 1);
    assert_eq!(
        calls.last().map(String::as_str),
        Some("fast_forward refs/remotes/origin/main")
    );
    // One deadline for the whole step, 3 s from its start.
    let deadlines = git.deadlines.borrow();
    assert!(deadlines.windows(2).all(|w| w[0] == w[1]), "{deadlines:?}");
    // Set between the call's start and its first git step.
    assert!(deadlines[0] >= started + Duration::from_secs(3));
    assert!(deadlines[0] <= started + Duration::from_millis(3100));
    // The stamp holds the time of the fetch, in Unix seconds.
    let stamp = fs::read_to_string(git.stamp()).unwrap();
    let written: u64 = stamp.trim().parse().unwrap();
    assert!(written.abs_diff(secs(SystemTime::now())) <= 2, "{stamp:?}");
}

#[test]
fn up_to_date_or_ahead_nothing_moves() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    for ancestors in [
        // equal: each is the other's ancestor
        vec![
            ("HEAD", "refs/remotes/origin/main"),
            ("refs/remotes/origin/main", "HEAD"),
        ],
        // ahead
        vec![("refs/remotes/origin/main", "HEAD")],
    ] {
        let mut git = FakeGit::behind(&tmp);
        git.ancestors = ancestors;
        let _ = fs::remove_file(git.stamp());
        assert_eq!(fresh_read(&git, &request(&brain)), Fresh::UpToDate);
        assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
    }
}

#[test]
fn a_diverged_branch_is_left_alone() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.ancestors.clear();
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Diverged);
    assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
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
            Fresh::FastForwarded,
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
    // A brain nested in a larger repo: no fetch, no fast-forward, no stamp.
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.prefix = "docs/brain/".into();
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Nested);
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
            assert_eq!(fresh, Fresh::FastForwarded, "age {age}");
        }
    }
    // A stamp that does not read is no stamp.
    fs::write(git.stamp(), "yesterday").unwrap();
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::FastForwarded);
}

#[test]
fn a_dirty_tree_is_never_fetched_into_and_not_stamped() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.dirty = true;
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Dirty);
    assert!(!git.calls().iter().any(|c| c.starts_with("fetch")));
    assert!(!git.stamp().exists());
}

#[test]
fn an_operation_in_progress_is_never_fetched_into() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    for marker in [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        let git = FakeGit::behind(&tmp);
        let path = git.git_dir.as_ref().unwrap().join(marker);
        if marker.starts_with("rebase") {
            fs::create_dir_all(&path).unwrap();
        } else {
            fs::write(&path, "0123\n").unwrap();
        }
        assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Busy, "{marker}");
        assert!(!git.calls().iter().any(|c| c.starts_with("fetch")));
        assert!(!git.stamp().exists(), "{marker}");
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
fn a_failed_or_timed_out_fetch_moves_nothing_and_is_throttled() {
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
        assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
        // Stamped before the fetch: an offline brain costs this once a minute.
        assert!(git.stamp().exists());
        assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Throttled);
    }
}

#[test]
fn no_fast_forward_starts_without_a_second_left() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.fetch_takes = Duration::from_millis(600);
    let brain = brain(&tmp);
    let mut req = request(&brain);
    req.time_limit = Duration::from_millis(1500);
    assert_eq!(fresh_read(&git, &req), Fresh::TimedOut);
    assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
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
