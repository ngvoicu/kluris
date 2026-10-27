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
    Fresh, FreshError, FreshGit, FreshRequest, MAX_FAST_FORWARD_FILES, STAMP, Upstream, WorkTree,
    fresh_read, ignored_in_the_way, prompt_env,
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
    /// The paths the fast-forward would write.
    incoming: Vec<String>,
    /// Whether a checkout filter is configured or set for a path.
    filters: bool,
    /// Ignored files and folders (`dir/`) of the work tree.
    ignored: Vec<String>,
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
            incoming: vec!["k/new.md".into()],
            filters: false,
            ignored: Vec::new(),
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

    fn incoming(
        &self,
        _dir: &Path,
        to: &str,
        deadline: Instant,
    ) -> Result<Vec<String>, FreshError> {
        self.log(&format!("incoming {to}"), deadline);
        Ok(self.incoming.clone())
    }

    fn uses_filters(&self, _dir: &Path, to: &str, deadline: Instant) -> Result<bool, FreshError> {
        self.log(&format!("uses_filters {to}"), deadline);
        Ok(self.filters)
    }

    fn ignored(&self, _dir: &Path, deadline: Instant) -> Result<Vec<String>, FreshError> {
        self.log("ignored", deadline);
        Ok(self.ignored.clone())
    }

    /// No deadline: once started, a fast-forward runs to its end.
    fn fast_forward(&self, _dir: &Path, to: &str) -> Result<(), FreshError> {
        self.calls.borrow_mut().push(format!("fast_forward {to}"));
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

#[test]
fn the_ignored_check_comes_right_before_the_fast_forward() {
    let tmp = TempDir::new().unwrap();
    let git = FakeGit::behind(&tmp);
    assert_eq!(
        fresh_read(&git, &request(&brain(&tmp))),
        Fresh::FastForwarded
    );
    let calls = git.calls();
    let n = calls.len();
    assert_eq!(calls[n - 2], "ignored", "{calls:?}");
    assert_eq!(calls[n - 1], "fast_forward refs/remotes/origin/main");
}

#[test]
fn an_ignored_file_in_the_way_stops_the_fast_forward() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.incoming = vec!["k/new.md".into(), "private/draft.md".into()];
    git.ignored = vec!["private/draft.md".into(), "build/".into()];
    assert_eq!(
        fresh_read(&git, &request(&brain(&tmp))),
        Fresh::IgnoredInTheWay
    );
    assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
    // Ignored files elsewhere do not.
    let mut git = FakeGit::behind(&tmp);
    git.ignored = vec!["private/draft.md".into(), "build/".into()];
    let _ = fs::remove_file(git.stamp());
    assert_eq!(
        fresh_read(&git, &request(&brain(&tmp))),
        Fresh::FastForwarded
    );
}

#[test]
fn a_path_is_in_the_way_of_itself_its_folders_and_its_contents() {
    let ignored = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let incoming = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // The same path.
    assert!(ignored_in_the_way(
        &ignored(&["a/b.md"]),
        &incoming(&["a/b.md"])
    ));
    // An ignored folder the incoming file goes into.
    assert!(ignored_in_the_way(
        &ignored(&["a/"]),
        &incoming(&["a/b.md"])
    ));
    assert!(ignored_in_the_way(
        &ignored(&["a/b/"]),
        &incoming(&["a/b/c/d.md"])
    ));
    // An ignored file where the incoming tree needs a folder.
    assert!(ignored_in_the_way(&ignored(&["a"]), &incoming(&["a/b.md"])));
    // An ignored file inside a folder the incoming tree writes as a file.
    assert!(ignored_in_the_way(&ignored(&["a/b.md"]), &incoming(&["a"])));
    assert!(ignored_in_the_way(&ignored(&["a/b/"]), &incoming(&["a"])));
    // Names that only share a prefix are not in the way.
    assert!(!ignored_in_the_way(&ignored(&["a"]), &incoming(&["ab.md"])));
    assert!(!ignored_in_the_way(
        &ignored(&["a/"]),
        &incoming(&["ab/c.md"])
    ));
    assert!(!ignored_in_the_way(&ignored(&["ab.md"]), &incoming(&["a"])));
    assert!(!ignored_in_the_way(&ignored(&[]), &incoming(&["a/b.md"])));
}

#[test]
fn a_configured_filter_leaves_only_the_fetch() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.filters = true;
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::Filtered);
    let calls = git.calls();
    assert!(calls.iter().any(|c| c == "fetch origin"), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("fast_forward")));
}

#[test]
fn too_many_incoming_files_leave_only_the_fetch() {
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let files = |n: usize| (0..n).map(|i| format!("k/n{i}.md")).collect::<Vec<_>>();
    let mut git = FakeGit::behind(&tmp);
    git.incoming = files(MAX_FAST_FORWARD_FILES + 1);
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::TooBig);
    assert!(git.calls().iter().any(|c| c == "fetch origin"));
    assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
    let mut git = FakeGit::behind(&tmp);
    let _ = fs::remove_file(git.stamp());
    git.incoming = files(MAX_FAST_FORWARD_FILES);
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::FastForwarded);
    assert_eq!(MAX_FAST_FORWARD_FILES, 200);
}
