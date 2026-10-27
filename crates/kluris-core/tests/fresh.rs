//! Fresh reads (`kluris_core::sync::fresh`): the decisions before `wake-up`
//! and `search` read a brain, against a fake git (the real git runs in
//! `crates/kluris-cli/tests/fresh_reads.rs`).

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kluris_core::git::GitError;
use kluris_core::sync::fresh::{
    Fresh, FreshError, FreshGit, FreshRequest, MAX_FAST_FORWARD_FILES, STAMP, Upstream, WorkTree,
    filter_values_run, filter_would_run, fits_on_disk, fresh_read, ignored_in_the_way, prompt_env,
    windows_path_too_long,
};
use tempfile::TempDir;

/// A change the fake makes in the brain's folder.
type Effect = Box<dyn Fn(&Path)>;

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
    /// The bytes the incoming files take.
    incoming_size: u64,
    /// `core.longpaths`.
    long_paths: bool,
    /// The paths the filter check was asked about.
    filter_paths: RefCell<Vec<String>>,
    /// What the fast-forward answers.
    fast_forward: Result<(), FreshError>,
    /// What a failed fast-forward leaves in the work tree.
    on_fast_forward: Option<Effect>,
    /// The index entries (`ls-files -s -z`) before the fast-forward, and
    /// after a failed one until a restore.
    index: Vec<u8>,
    index_after: Option<Vec<u8>>,
    /// HEAD after a failed fast-forward (else `h1` throughout).
    head_after: Option<String>,
    /// A restore puts the work tree back with this, or fails.
    on_restore: Option<Effect>,
    restore_fails: bool,
    /// Whether the fast-forward ran, and a restore since.
    ran: Cell<bool>,
    restored: Cell<bool>,
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
            incoming_size: 0,
            long_paths: false,
            filter_paths: RefCell::new(Vec::new()),
            fast_forward: Ok(()),
            on_fast_forward: None,
            index: Vec::new(),
            index_after: None,
            head_after: None,
            on_restore: None,
            restore_fails: false,
            ran: Cell::new(false),
            restored: Cell::new(false),
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

    fn uses_filters(
        &self,
        _dir: &Path,
        to: &str,
        incoming: &[String],
        deadline: Instant,
    ) -> Result<bool, FreshError> {
        self.log(&format!("uses_filters {to}"), deadline);
        *self.filter_paths.borrow_mut() = incoming.to_vec();
        Ok(self.filters)
    }

    fn incoming_size(
        &self,
        _dir: &Path,
        to: &str,
        _incoming: &[String],
        deadline: Instant,
    ) -> Result<u64, FreshError> {
        self.log(&format!("incoming_size {to}"), deadline);
        Ok(self.incoming_size)
    }

    fn long_paths(&self, _dir: &Path, deadline: Instant) -> Result<bool, FreshError> {
        self.log("long_paths", deadline);
        Ok(self.long_paths)
    }

    fn ignored(&self, _dir: &Path, deadline: Instant) -> Result<Vec<String>, FreshError> {
        self.log("ignored", deadline);
        Ok(self.ignored.clone())
    }

    /// No deadline: once started, a fast-forward runs to its end.
    fn fast_forward(&self, _dir: &Path, to: &str) -> Result<(), FreshError> {
        self.calls.borrow_mut().push(format!("fast_forward {to}"));
        self.ran.set(true);
        if self.fast_forward.is_err()
            && let Some(effect) = &self.on_fast_forward
        {
            effect(_dir);
        }
        self.fast_forward.clone()
    }

    fn head_commit(&self, _dir: &Path, deadline: Instant) -> Result<String, FreshError> {
        self.log("head_commit", deadline);
        Ok(match (&self.head_after, self.ran.get()) {
            (Some(after), true) => after.clone(),
            _ => "h1".to_string(),
        })
    }

    fn index_entries(
        &self,
        _dir: &Path,
        _paths: &[String],
        deadline: Instant,
    ) -> Result<Vec<u8>, FreshError> {
        self.log("index_entries", deadline);
        Ok(match &self.index_after {
            Some(after) if self.ran.get() && !self.restored.get() => after.clone(),
            _ => self.index.clone(),
        })
    }

    fn restore(&self, dir: &Path, in_head: &[String], all: &[String]) -> Result<(), FreshError> {
        self.calls
            .borrow_mut()
            .push(format!("restore {} | {}", in_head.join(","), all.join(",")));
        if self.restore_fails {
            return Err(FreshError::Failed(GitError {
                args: vec!["checkout-index".into()],
                code: Some(1),
                detail: "simulated".into(),
            }));
        }
        if let Some(effect) = &self.on_restore {
            effect(dir);
        }
        self.restored.set(true);
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

#[test]
fn a_filter_counts_only_when_an_attribute_names_a_configured_driver() {
    let keys = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let lfs = keys(&[
        "filter.lfs.clean",
        "filter.lfs.smudge",
        "filter.lfs.process",
        "filter.lfs.required",
    ]);
    let attrs = |text: &str| vec![text.to_string()];
    // A driver nothing names, and a name with no driver: nothing runs.
    assert!(!filter_would_run(&[], &lfs));
    assert!(!filter_would_run(&attrs("*.png binary\n"), &lfs));
    assert!(!filter_would_run(&attrs("*.bin filter=lfs\n"), &[]));
    assert!(!filter_would_run(
        &attrs("*.bin filter=lfs\n"),
        &keys(&["filter.lfs.required"])
    ));
    // A named, configured driver runs: smudge, clean or process.
    assert!(filter_would_run(
        &attrs("*.bin filter=lfs diff=lfs -text\n"),
        &lfs
    ));
    for key in ["filter.x.smudge", "filter.x.clean", "filter.x.process"] {
        assert!(
            filter_would_run(&attrs("a filter=x\n"), &keys(&[key])),
            "{key}"
        );
    }
    // Names with dots, macros, and an attribute among several sources.
    assert!(filter_would_run(
        &attrs("a filter=a.b\n"),
        &keys(&["filter.a.b.smudge"])
    ));
    assert!(filter_would_run(
        &attrs("[attr]big filter=x\n*.bin big\n"),
        &keys(&["filter.x.clean"])
    ));
    assert!(filter_would_run(
        &[String::new(), "k/* filter=x\n".into()],
        &keys(&["filter.x.clean"])
    ));
    // Unset or negated, commented out, another name: not this driver.
    for text in [
        "a -filter\n",
        "a !filter\n",
        "# a filter=x\n",
        "a filter=y\n",
        "a myfilter=x\n",
    ] {
        assert!(
            !filter_would_run(&attrs(text), &keys(&["filter.x.smudge"])),
            "{text:?}"
        );
    }
    // Config keys are case-insensitive in their section and variable.
    assert!(filter_would_run(
        &attrs("a filter=x\n"),
        &keys(&["filter.x.Smudge"])
    ));
}

#[test]
fn the_filter_check_is_asked_about_every_incoming_path() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.incoming = vec!["k/a.md".into(), "k/z.md".into(), "gone.md".into()];
    assert_eq!(
        fresh_read(&git, &request(&brain(&tmp))),
        Fresh::FastForwarded
    );
    assert_eq!(*git.filter_paths.borrow(), git.incoming);
}

#[test]
fn too_little_free_space_leaves_only_the_fetch() {
    let tmp = TempDir::new().unwrap();
    let mut git = FakeGit::behind(&tmp);
    git.incoming_size = u64::MAX / 4;
    assert_eq!(fresh_read(&git, &request(&brain(&tmp))), Fresh::NoSpace);
    assert!(git.calls().iter().any(|c| c == "fetch origin"));
    assert!(!git.calls().iter().any(|c| c.starts_with("fast_forward")));
}

#[test]
fn the_incoming_files_must_fit_with_room_to_spare() {
    const MIB: u64 = 1024 * 1024;
    // Each file takes at least one 4 KiB block; 64 MiB stay free.
    assert!(fits_on_disk(0, 0, 64 * MIB));
    assert!(fits_on_disk(1, 1, 64 * MIB + 4096));
    assert!(!fits_on_disk(1, 1, 64 * MIB + 4095));
    assert!(!fits_on_disk(10 * MIB, 3, 70 * MIB));
    assert!(fits_on_disk(10 * MIB, 3, 75 * MIB));
    // No overflow on huge sizes.
    assert!(!fits_on_disk(u64::MAX, u64::MAX, u64::MAX));
}

#[test]
fn a_windows_path_of_248_characters_or_more_is_too_long_without_core_longpaths() {
    let root = format!("C:\\{}", "r".repeat(100));
    let short = "k/a.md".to_string();
    let rel = |len: usize| format!("k/{}", "x".repeat(len - 2));
    // root + "\\" + rel: 103 + 1 + rel.
    assert!(!windows_path_too_long(Path::new(&root), &short));
    assert!(!windows_path_too_long(Path::new(&root), &rel(143)));
    assert!(windows_path_too_long(Path::new(&root), &rel(144)));
    // Counted in UTF-16 units, as Windows counts.
    assert!(windows_path_too_long(
        Path::new(&root),
        &format!("k/{}", "\u{1F600}".repeat(72))
    ));
}

#[cfg(unix)]
#[test]
fn a_folder_the_fast_forward_cannot_write_leaves_only_the_fetch() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let k = brain.join("k");
    fs::create_dir_all(&k).unwrap();
    let mut git = FakeGit::behind(&tmp);
    // An existing file in a folder that cannot be written, and a new file
    // whose nearest existing folder cannot be written.
    for (folder, incoming) in [(&k, "k/a.md"), (&brain, "new/b.md")] {
        let _ = fs::remove_file(git.stamp());
        git.incoming = vec![incoming.into()];
        fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).unwrap();
        let fresh = fresh_read(&git, &request(&brain));
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(fresh, Fresh::Unwritable, "{incoming}");
    }
    let _ = fs::remove_file(git.stamp());
    git.incoming = vec!["k/a.md".into(), "new/b.md".into()];
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::FastForwarded);
}

#[test]
fn a_fast_forward_git_ends_in_failure_is_told_apart() {
    // git ran and exited non-zero having changed nothing: nothing to undo,
    // and the command says so. git that could not start changed nothing.
    let tmp = TempDir::new().unwrap();
    let brain = brain(&tmp);
    let failure = |code| {
        Err(FreshError::Failed(GitError {
            args: vec!["merge".into()],
            code,
            detail: "error: unable to write file".into(),
        }))
    };
    let mut git = FakeGit::behind(&tmp);
    git.fast_forward = failure(Some(1));
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::RolledBack);
    assert!(!git.calls().iter().any(|c| c.starts_with("restore")));
    let mut git = FakeGit::behind(&tmp);
    let _ = fs::remove_file(git.stamp());
    git.fast_forward = failure(None);
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::Failed);
}

#[test]
fn only_a_failed_fast_forward_is_reported() {
    for fresh in [
        Fresh::Disabled,
        Fresh::NotARepo,
        Fresh::Throttled,
        Fresh::Dirty,
        Fresh::FetchFailed,
        Fresh::TimedOut,
        Fresh::InUse,
        Fresh::Failed,
        Fresh::FastForwarded,
        Fresh::Locked,
    ] {
        assert_eq!(fresh.report("team"), None, "{fresh:?}");
    }
    assert_eq!(
        Fresh::RolledBack.report("team").as_deref(),
        Some(
            "kluris: team could not be updated from its remote; nothing changed. Run `kluris sync --brain team` to see why"
        )
    );
    assert_eq!(
        Fresh::PartlyUpdated(vec!["glossary.md".into(), "k/jwt.md".into()])
            .report("team")
            .as_deref(),
        Some(
            "kluris: team was only partly updated from its remote (glossary.md, k/jwt.md); run `kluris sync --brain team` to finish"
        )
    );
    // A long list names ten paths and counts the rest.
    let many: Vec<String> = (0..12).map(|i| format!("k/{i:02}.md")).collect();
    let line = Fresh::PartlyUpdated(many).report("team").unwrap();
    assert!(line.contains("(k/00.md, k/01.md, k/02.md, k/03.md, k/04.md, k/05.md, k/06.md, k/07.md, k/08.md, k/09.md and 2 more)"), "{line}");
}

/// `ls-files -s -z` entries for `(path, blob)` pairs.
fn index_of(entries: &[(&str, &str)]) -> Vec<u8> {
    entries
        .iter()
        .flat_map(|(path, blob)| format!("100644 {blob} 0\t{path}\0").into_bytes())
        .collect()
}

/// A brain with `k/a.md` ("old") whose fast-forward to `k/a.md` ("new")
/// and a new `k/n/new.md` stops partway after writing both.
fn partial(tmp: &TempDir) -> (PathBuf, FakeGit) {
    let brain = brain(tmp);
    fs::create_dir_all(brain.join("k")).unwrap();
    fs::write(brain.join("k/a.md"), "old\n").unwrap();
    let mut git = FakeGit::behind(tmp);
    git.incoming = vec!["k/a.md".into(), "k/n/new.md".into()];
    git.index = index_of(&[("k/a.md", "aaa")]);
    git.index_after = Some(index_of(&[("k/a.md", "bbb"), ("k/n/new.md", "ccc")]));
    git.fast_forward = Err(FreshError::Failed(GitError {
        args: vec!["merge".into()],
        code: Some(128),
        detail: "error: unable to write file".into(),
    }));
    git.on_fast_forward = Some(Box::new(|dir: &Path| {
        fs::write(dir.join("k/a.md"), "new\n").unwrap();
        fs::create_dir_all(dir.join("k/n")).unwrap();
        fs::write(dir.join("k/n/new.md"), "new\n").unwrap();
    }));
    git.on_restore = Some(Box::new(|dir: &Path| {
        fs::write(dir.join("k/a.md"), "old\n").unwrap();
    }));
    (brain, git)
}

#[test]
fn a_fast_forward_that_stops_partway_is_rolled_back() {
    let tmp = TempDir::new().unwrap();
    let (brain, git) = partial(&tmp);
    assert_eq!(fresh_read(&git, &request(&brain)), Fresh::RolledBack);
    // Only what changed is restored: the index of both, the file of the one
    // HEAD has; the new file and its new folder are removed.
    assert!(
        git.calls()
            .contains(&"restore k/a.md | k/a.md,k/n/new.md".to_string()),
        "{:?}",
        git.calls()
    );
    assert_eq!(fs::read_to_string(brain.join("k/a.md")).unwrap(), "old\n");
    assert!(!brain.join("k/n").exists());
}

#[test]
fn a_rollback_that_fails_names_what_it_left_changed() {
    let tmp = TempDir::new().unwrap();
    let (brain, mut git) = partial(&tmp);
    git.restore_fails = true;
    // The index still holds both (the fake restore did not reset it).
    assert_eq!(
        fresh_read(&git, &request(&brain)),
        Fresh::PartlyUpdated(vec!["k/a.md".into(), "k/n/new.md".into()])
    );
    // The new file is still removed.
    assert!(!brain.join("k/n").exists());
}

#[test]
fn a_head_that_moved_is_not_rolled_back() {
    let tmp = TempDir::new().unwrap();
    let (brain, mut git) = partial(&tmp);
    git.head_after = Some("h2".into());
    assert!(
        matches!(fresh_read(&git, &request(&brain)), Fresh::PartlyUpdated(_)),
        "{:?}",
        git.calls()
    );
    assert!(!git.calls().iter().any(|c| c.starts_with("restore")));
}

#[test]
fn a_filter_value_counts_when_a_driver_of_that_name_is_configured() {
    let keys = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let triples = |items: &[(&str, &str)]| -> Vec<u8> {
        items
            .iter()
            .flat_map(|(path, value)| format!("{path}\0filter\0{value}\0").into_bytes())
            .collect()
    };
    // git's states mean no filter when no driver has their name...
    for state in ["set", "unset", "unspecified"] {
        let out = triples(&[("k/z.md", state)]);
        assert!(
            !filter_values_run(&out, &keys(&["filter.lfs.smudge"])),
            "{state}"
        );
        // ...and are the driver when one does.
        let key = format!("filter.{state}.clean");
        assert!(filter_values_run(&out, &keys(&[&key])), "{state}");
    }
    // A path is never read as attribute text: `#` and spaces are just bytes.
    let out = triples(&[
        ("k/a.md", "unspecified"),
        ("#Inbox/z.md", "slow"),
        ("a b.md", "x"),
    ]);
    assert!(filter_values_run(&out, &keys(&["filter.slow.smudge"])));
    assert!(filter_values_run(&out, &keys(&["filter.x.process"])));
    assert!(!filter_values_run(&out, &keys(&["filter.other.smudge"])));
    // An empty path or value keeps the triples aligned.
    let out = b"\0filter\0unspecified\0k/z.md\0filter\0slow\0".to_vec();
    assert!(filter_values_run(&out, &keys(&["filter.slow.smudge"])));
    assert!(!filter_values_run(&[], &keys(&["filter.slow.smudge"])));
}

#[cfg(target_os = "macos")]
mod flags {
    use super::*;
    use kluris_core::sync::fresh::locked_by_flags;
    use std::process::Command;

    /// Sets a `chflags` flag and clears it however the test ends.
    pub struct Flagged(PathBuf, &'static str);

    impl Flagged {
        pub fn set(path: &Path, flag: &'static str) -> Flagged {
            assert!(
                Command::new("chflags")
                    .arg(flag)
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
            Flagged(path.to_path_buf(), flag)
        }
    }

    impl Drop for Flagged {
        fn drop(&mut self) {
            let _ = Command::new("chflags")
                .arg(format!("no{}", self.1))
                .arg(&self.0)
                .status();
        }
    }

    #[test]
    fn immutable_or_append_only_files_and_folders_lock_the_fast_forward() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        fs::create_dir_all(dir.join("k")).unwrap();
        fs::write(dir.join("k/a.md"), "a\n").unwrap();
        let incoming = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!locked_by_flags(
            dir,
            &incoming(&["k/a.md", "k/new.md", "n/x.md"])
        ));
        for flag in ["uchg", "uappnd"] {
            {
                let _f = Flagged::set(&dir.join("k/a.md"), flag);
                assert!(locked_by_flags(dir, &incoming(&["k/a.md"])), "file {flag}");
                assert!(
                    !locked_by_flags(dir, &incoming(&["n/x.md"])),
                    "elsewhere {flag}"
                );
            }
            {
                // The folder a file is replaced, added or deleted in.
                let _f = Flagged::set(&dir.join("k"), flag);
                assert!(
                    locked_by_flags(dir, &incoming(&["k/new.md"])),
                    "folder {flag}"
                );
                assert!(
                    locked_by_flags(dir, &incoming(&["k/a.md"])),
                    "folder {flag}"
                );
                assert!(
                    locked_by_flags(dir, &incoming(&["k/sub/new.md"])),
                    "nearest {flag}"
                );
            }
        }
    }

    /// The system git with the flag check turned off, as it was before
    /// T-157: the rollback is then the only guard.
    struct NoFlagCheck(kluris_core::git::SystemGit);

    impl FreshGit for NoFlagCheck {
        fn git_paths(
            &self,
            dir: &Path,
            names: &[&str],
            d: Instant,
        ) -> Result<Option<WorkTree>, FreshError> {
            self.0.git_paths(dir, names, d)
        }
        fn upstream(&self, dir: &Path, d: Instant) -> Result<Option<Upstream>, FreshError> {
            self.0.upstream(dir, d)
        }
        fn has_tracked_changes(&self, dir: &Path, d: Instant) -> Result<bool, FreshError> {
            self.0.has_tracked_changes(dir, d)
        }
        fn fetch(&self, dir: &Path, remote: &str, d: Instant) -> Result<(), FreshError> {
            self.0.fetch(dir, remote, d)
        }
        fn is_ancestor(
            &self,
            dir: &Path,
            a: &str,
            b: &str,
            d: Instant,
        ) -> Result<bool, FreshError> {
            self.0.is_ancestor(dir, a, b, d)
        }
        fn incoming(&self, dir: &Path, to: &str, d: Instant) -> Result<Vec<String>, FreshError> {
            self.0.incoming(dir, to, d)
        }
        fn uses_filters(
            &self,
            dir: &Path,
            to: &str,
            inc: &[String],
            d: Instant,
        ) -> Result<bool, FreshError> {
            self.0.uses_filters(dir, to, inc, d)
        }
        fn incoming_size(
            &self,
            dir: &Path,
            to: &str,
            inc: &[String],
            d: Instant,
        ) -> Result<u64, FreshError> {
            self.0.incoming_size(dir, to, inc, d)
        }
        fn long_paths(&self, dir: &Path, d: Instant) -> Result<bool, FreshError> {
            self.0.long_paths(dir, d)
        }
        fn ignored(&self, dir: &Path, d: Instant) -> Result<Vec<String>, FreshError> {
            self.0.ignored(dir, d)
        }
        fn fast_forward(&self, dir: &Path, to: &str) -> Result<(), FreshError> {
            self.0.fast_forward(dir, to)
        }
        fn head_commit(&self, dir: &Path, d: Instant) -> Result<String, FreshError> {
            self.0.head_commit(dir, d)
        }
        fn index_entries(
            &self,
            dir: &Path,
            paths: &[String],
            d: Instant,
        ) -> Result<Vec<u8>, FreshError> {
            self.0.index_entries(dir, paths, d)
        }
        fn restore(
            &self,
            dir: &Path,
            in_head: &[String],
            all: &[String],
        ) -> Result<(), FreshError> {
            self.0.restore(dir, in_head, all)
        }
        fn locked(&self, _dir: &Path, _incoming: &[String]) -> bool {
            false
        }
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// HEAD, `ls-files -s` and every file's bytes outside `.git`.
    fn snapshot(dir: &Path) -> (String, String, Vec<(PathBuf, Vec<u8>)>) {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if path.is_dir() {
                    files.push((path.clone(), b"<dir>".to_vec()));
                    stack.push(path);
                } else {
                    let bytes = fs::read(&path).unwrap();
                    files.push((path, bytes));
                }
            }
        }
        files.sort();
        (
            git(dir, &["rev-parse", "HEAD"]),
            git(dir, &["ls-files", "-s"]),
            files,
        )
    }

    #[test]
    fn a_real_partial_checkout_on_an_immutable_file_is_rolled_back() {
        // T-155 F155-1 with the flag check off: git writes notes/a.md, then
        // cannot replace the immutable notes/z.md and exits non-zero.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        git(root, &["clone", "-q", "remote.git", "theirs"]);
        let theirs = root.join("theirs");
        fs::create_dir_all(theirs.join("notes")).unwrap();
        fs::write(theirs.join("notes/a.md"), "# a\n").unwrap();
        fs::write(theirs.join("notes/z.md"), "# z\n").unwrap();
        git(&theirs, &["add", "-A"]);
        git(&theirs, &["commit", "-q", "-m", "base"]);
        git(&theirs, &["push", "-q", "-u", "origin", "main"]);
        git(root, &["clone", "-q", "remote.git", "mine"]);
        let mine = root.join("mine");
        fs::write(theirs.join("notes/a.md"), "# INCOMING A\n").unwrap();
        fs::write(theirs.join("notes/z.md"), "# INCOMING Z\n").unwrap();
        fs::write(theirs.join("notes/new.md"), "# new\n").unwrap();
        git(&theirs, &["add", "-A"]);
        git(&theirs, &["commit", "-q", "-m", "incoming"]);
        git(&theirs, &["push", "-q"]);
        let before = snapshot(&mine);
        let fresh = {
            let _flag = Flagged::set(&mine.join("notes/z.md"), "uchg");
            let req = FreshRequest::new(&mine, false);
            fresh_read(&NoFlagCheck(kluris_core::git::SystemGit::new()), &req)
        };
        assert_eq!(fresh, Fresh::RolledBack);
        assert_eq!(snapshot(&mine), before, "not rolled back");
        assert!(!mine.join(".git/index.lock").exists());
    }
}
