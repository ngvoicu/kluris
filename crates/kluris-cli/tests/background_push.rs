//! The background push: a read or housekeeping session closed with commits
//! waiting starts one detached `kluris sync --background --path <root>...`
//! for every such brain. The child fetches and pushes only, under the
//! brain's `background.lock`, logs to `background.log`, and records what
//! happened in `last-push.json`, which the next command reports.
//!
//! The sessions are driven through `kluris_core::sync::session`, with this
//! package's binary as the program a session starts. A command that closes
//! a read and exits is this test binary run again as a child.

mod sync_home;

use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::sync::SyncReport;
use kluris_core::sync::background::{self, LAST_PUSH, LOCK, LOG, LastPush, Lock, PushResult};
use kluris_core::sync::session::{self, Brain, Intent, SessionEnv};
use kluris_core::yaml::Date;
use serde_json::Value;
use sync_home::*;

/// The longest a test waits for a process to do something.
const PATIENCE: Duration = Duration::from_secs(60);

fn kluris() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kluris"))
}

/// Polls `done` until it holds, at most [`PATIENCE`].
fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < PATIENCE,
            "{what}: not within {PATIENCE:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// A file of the brain's (main worktree's) git directory.
fn git_file(root: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(root.join(".git"), |path, part| path.join(part))
}

/// The brain's `last-push.json`, read as JSON.
fn record(root: &Path) -> Option<Value> {
    let text = fs::read_to_string(git_file(root, LAST_PUSH)).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}")))
}

fn result_of(root: &Path) -> Option<String> {
    record(root).and_then(|r| r["result"].as_str().map(str::to_string))
}

/// The git a session in this process runs: the home's, hermetic.
fn git(home: &Home) -> SystemGit {
    let mut git = SystemGit::new()
        .env("HOME", home.dir.path())
        .env("USERPROFILE", home.dir.path())
        .env("GIT_CONFIG_GLOBAL", home.dir.path().join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1");
    for var in ISOLATED_VARS {
        git = git.env_remove(var);
    }
    git
}

fn env(home: &Home, program: Option<PathBuf>) -> SessionEnv {
    SessionEnv {
        git: git(home),
        no_verify: false,
        now: SystemTime::now(),
        program,
    }
}

fn maps() -> DreamOptions<'static> {
    DreamOptions {
        name: "demo",
        description: "d",
        today: Date::new(2026, 1, 1).unwrap(),
    }
}

fn brain(root: &Path) -> Brain<'_> {
    Brain { root, maps: maps() }
}

/// A brain with `name`, and one commit of ours the remote does not have.
fn waiting(home: &mut Home, name: &str) -> PathBuf {
    let root = home.brain(name);
    home.commit(&root, "notes/waiting.md", &neuron("Waiting", "W.\n"));
    root
}

fn local_head(home: &Home, root: &Path) -> String {
    home.git(root, &["rev-parse", "HEAD"]).trim().to_string()
}

fn remote_head(home: &Home, name: &str) -> String {
    let bare = home.dir.path().join(format!("{name}.git"));
    home.git(&bare, &["rev-parse", "main"]).trim().to_string()
}

/// `kluris sync --background --path <root>...`, run here and waited for.
fn push_in_foreground(home: &Home, roots: &[&Path]) -> (i32, String, String) {
    let mut args = vec!["sync".to_string(), "--background".to_string()];
    for root in roots {
        args.push("--path".into());
        args.push(root.to_string_lossy().into_owned());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    home.run(&args)
}

/// HEAD, the index, the status and every file of the work tree.
fn snapshot(home: &Home, root: &Path) -> (String, String, String, Vec<(PathBuf, Vec<u8>)>) {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if entry.file_name() == ".git" {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    (
        local_head(home, root),
        home.git(root, &["ls-files", "--stage"]),
        home.git(root, &["status", "--porcelain", "--untracked-files=all"]),
        files,
    )
}

/// A `receive-pack` for the brain's remote that waits until `release`
/// exists (60 s at most), then receives.
fn stall_pushes(home: &Home, root: &Path, release: &Path) {
    let script = home.dir.path().join("stalled-receive-pack.sh");
    let release = release.display().to_string().replace('\\', "/");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ni=0\nwhile [ ! -e '{release}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done\nexec git receive-pack \"$@\"\n"
        ),
    )
    .unwrap();
    let pack = format!("sh '{}'", script.display().to_string().replace('\\', "/"));
    home.git(root, &["config", "remote.origin.receivepack", &pack]);
}

/// A hook of the brain's that prints `text` on stderr and exits `code`.
fn hook(root: &Path, name: &str, text: &str, code: i32) {
    let path = root.join(".git").join("hooks").join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\necho '{text}' >&2\nexit {code}\n"),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// `kluris sync --background --path <root>` started here, not waited for.
fn spawn_push(home: &Home, root: &Path) -> Child {
    let mut cmd = Command::new(kluris());
    home.hermetic(&mut cmd);
    cmd.args(["sync", "--background", "--path"])
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().unwrap()
}

/// A process id that is no process now.
fn dead_pid() -> u32 {
    let mut child = Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

fn write_lock(root: &Path, pid: u32, host: &str, started: SystemTime) {
    let text = format!(
        "{{\"pid\": {pid}, \"host\": \"{host}\", \"started\": \"{}\"}}\n",
        background::timestamp(started)
    );
    fs::create_dir_all(git_file(root, "kluris-sync")).unwrap();
    fs::write(git_file(root, LOCK), text).unwrap();
}

fn write_record(root: &Path, result: PushResult, at: SystemTime) -> LastPush {
    let record = LastPush {
        at: background::timestamp(at),
        head: "0".repeat(40),
        result,
        detail: (result != PushResult::Pushed).then(|| "why".to_string()),
        commits: 1,
    };
    fs::create_dir_all(git_file(root, "kluris-sync")).unwrap();
    record.write(&git_file(root, LAST_PUSH)).unwrap();
    record
}

fn ago(minutes: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(60 * minutes)
}

#[test]
fn the_push_sends_the_commits_and_records_it_leaving_the_work_tree_alone() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let before = snapshot(&home, &root);
    let (code, out, err) = push_in_foreground(&home, &[&root]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(out, "", "nothing on stdout");
    assert_eq!(remote_head(&home, "alpha"), local_head(&home, &root));
    assert_eq!(
        snapshot(&home, &root),
        before,
        "the work tree, index or HEAD changed"
    );
    let doc = record(&root).expect("a record");
    let keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["at", "head", "result", "detail", "commits"], "{doc}");
    assert_eq!(doc["result"], "pushed", "{doc}");
    assert_eq!(doc["head"], local_head(&home, &root).as_str(), "{doc}");
    assert_eq!(doc["commits"], 1, "{doc}");
    assert_eq!(doc["detail"], Value::Null, "{doc}");
    assert!(
        background::seconds(doc["at"].as_str().unwrap()).is_some(),
        "{doc}"
    );
    assert!(!git_file(&root, LOCK).exists(), "the lock stayed");
    // Written whole: no temp file of the writer is left beside it.
    for entry in fs::read_dir(git_file(&root, "kluris-sync"))
        .unwrap()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.contains("kluris-tmp"), "{name}");
    }
}

#[test]
fn a_remote_that_moved_is_needs_pull_and_nothing_lands() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    home.teammate_pushes("alpha", "notes/theirs.md", &neuron("Theirs", "T.\n"));
    let theirs = remote_head(&home, "alpha");
    let before = snapshot(&home, &root);
    let (code, out, err) = push_in_foreground(&home, &[&root]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(result_of(&root).as_deref(), Some("needs_pull"));
    assert_eq!(
        remote_head(&home, "alpha"),
        theirs,
        "pushed over the teammate"
    );
    assert_eq!(snapshot(&home, &root), before, "something landed");
}

#[test]
fn a_push_the_remote_turns_back_because_it_moved_is_needs_pull() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    // The teammate pushes between the child's fetch and its push.
    let script = home.dir.path().join("racing-pre-push.sh");
    let mate = home
        .teammate("alpha")
        .display()
        .to_string()
        .replace('\\', "/");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nunset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\ncd '{mate}' && echo racing >> notes/race.md && git add -A && git commit -q -m race && git push -q >&2\n"
        ),
    )
    .unwrap();
    hook(&root, "pre-push", "", 0);
    let path = root.join(".git").join("hooks").join("pre-push");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nsh '{}'\n",
            script.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let before = snapshot(&home, &root);
    let (code, out, err) = push_in_foreground(&home, &[&root]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        result_of(&root).as_deref(),
        Some("needs_pull"),
        "{:?}",
        record(&root)
    );
    assert_eq!(snapshot(&home, &root), before);
}

#[test]
fn each_failure_is_recorded_with_its_detail() {
    // Offline: the remote cannot be reached.
    {
        let mut home = Home::new();
        let root = waiting(&mut home, "alpha");
        home.git(
            &root,
            &[
                "remote",
                "set-url",
                "origin",
                "ssh://git@nonexistent.invalid/x.git",
            ],
        );
        let (code, ..) = push_in_foreground(&home, &[&root]);
        assert_eq!(code, 0);
        assert_eq!(
            result_of(&root).as_deref(),
            Some("offline"),
            "{:?}",
            record(&root)
        );
        assert_eq!(record(&root).unwrap()["commits"], 1);
    }

    // Refused: a pre-push hook turns it away.
    {
        let mut home = Home::new();
        let root = waiting(&mut home, "beta");
        hook(&root, "pre-push", "not today", 1);
        let (code, ..) = push_in_foreground(&home, &[&root]);
        assert_eq!(code, 0);
        let doc = record(&root).unwrap();
        assert_eq!(doc["result"], "refused", "{doc}");
        assert!(
            doc["detail"].as_str().unwrap().contains("not today"),
            "{doc}"
        );
    }

    // Failed: a local step fails (a hook stops the tracking ref's move).
    {
        let mut home = Home::new();
        let root = waiting(&mut home, "gamma");
        home.teammate_pushes("gamma", "notes/theirs.md", &neuron("Theirs", "T.\n"));
        let path = root
            .join(".git")
            .join("hooks")
            .join("reference-transaction");
        hook(&root, "reference-transaction", "", 0);
        fs::write(
            &path,
            "#!/bin/sh\n[ \"$1\" = prepared ] && grep -q refs/remotes/ && exit 1\nexit 0\n",
        )
        .unwrap();
        let (code, ..) = push_in_foreground(&home, &[&root]);
        assert_eq!(code, 0);
        assert_eq!(
            result_of(&root).as_deref(),
            Some("failed"),
            "{:?}",
            record(&root)
        );
    }
}

#[test]
fn the_log_holds_what_git_and_the_hooks_printed_and_each_run_replaces_it() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    hook(&root, "pre-push", "first run hook", 0);
    push_in_foreground(&home, &[&root]);
    let log = fs::read_to_string(git_file(&root, LOG)).unwrap();
    assert!(log.contains("first run hook"), "{log}");
    assert!(log.contains("kluris sync --background"), "{log}");

    home.commit(&root, "notes/more.md", &neuron("More", "M.\n"));
    hook(&root, "pre-push", "second run hook", 0);
    let (_, out, err) = push_in_foreground(&home, &[&root]);
    let log = fs::read_to_string(git_file(&root, LOG)).unwrap();
    assert!(log.contains("second run hook"), "{log}");
    assert!(!log.contains("first run hook"), "{log}");
    assert_eq!(out, "");
    assert!(
        !err.contains("second run hook"),
        "the hook's output went to stderr: {err}"
    );
}

#[test]
fn the_lock_holds_pid_host_and_start_and_a_live_one_stops_a_second_run() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let release = home.dir.path().join("release");
    stall_pushes(&home, &root, &release);
    let mut first = spawn_push(&home, &root);
    eventually("the lock", || git_file(&root, LOCK).exists());
    let lock = Lock::read(&git_file(&root, LOCK)).expect("a lock that reads");
    assert_eq!(lock.pid, first.id());
    assert_eq!(lock.host, background::this_host());
    let started = background::seconds(&lock.started).expect("an RFC 3339 start");
    let now = background::seconds(&background::timestamp(SystemTime::now())).unwrap();
    assert!((now - started).abs() < 60, "{lock:?}");
    assert!(lock.live(SystemTime::now()));
    assert!(!background::wanted(&root.join(".git"), SystemTime::now()));

    // A second run leaves the brain to the first.
    let (code, ..) = push_in_foreground(&home, &[&root]);
    assert_eq!(code, 0);
    assert_eq!(Lock::read(&git_file(&root, LOCK)).unwrap().pid, first.id());
    assert_eq!(record(&root), None, "the second run recorded");

    fs::write(&release, "").unwrap();
    assert!(first.wait().unwrap().success());
    assert!(!git_file(&root, LOCK).exists(), "the lock stayed");
    assert_eq!(result_of(&root).as_deref(), Some("pushed"));
    assert!(background::wanted(&root.join(".git"), SystemTime::now()));
}

#[test]
fn a_stale_lock_is_replaced_and_another_hosts_young_lock_is_not() {
    let host = background::this_host();
    for (case, pid, host, started, replaced) in [
        (
            "a dead pid",
            dead_pid(),
            host.as_str(),
            SystemTime::now(),
            true,
        ),
        (
            "started 11 min ago",
            std::process::id(),
            host.as_str(),
            ago(11),
            true,
        ),
        (
            "another host's",
            std::process::id(),
            "elsewhere",
            SystemTime::now(),
            false,
        ),
    ] {
        let mut home = Home::new();
        let root = waiting(&mut home, "alpha");
        write_lock(&root, pid, host, started);
        assert_eq!(
            background::wanted(&root.join(".git"), SystemTime::now()),
            replaced,
            "{case}"
        );
        let (code, ..) = push_in_foreground(&home, &[&root]);
        assert_eq!(code, 0, "{case}");
        if replaced {
            assert_eq!(result_of(&root).as_deref(), Some("pushed"), "{case}");
            assert!(!git_file(&root, LOCK).exists(), "{case}: the lock stayed");
        } else {
            assert_eq!(record(&root), None, "{case}");
            assert_eq!(
                Lock::read(&git_file(&root, LOCK)).unwrap().host,
                "elsewhere"
            );
        }
    }
}

#[test]
fn a_failure_holds_off_automatic_starts_for_ten_minutes() {
    let mut home = Home::new();
    let root = home.brain("alpha");
    let common = root.join(".git");
    for (result, minutes, wanted) in [
        (PushResult::Offline, 1, false),
        (PushResult::Refused, 1, false),
        (PushResult::Failed, 1, false),
        (PushResult::Pushed, 1, true),
        (PushResult::NeedsPull, 1, true),
        (PushResult::Offline, 11, true),
        (PushResult::Refused, 11, true),
        (PushResult::Failed, 11, true),
    ] {
        write_record(&root, result, ago(minutes));
        assert_eq!(
            background::wanted(&common, SystemTime::now()),
            wanted,
            "{result:?} {minutes} min ago"
        );
    }
}

/// What a read of the brain at `root` reports, starting nothing.
fn read(home: &Home, root: &Path) -> Option<SyncReport> {
    session::close(session::open(brain(root), Intent::Read, &env(home, None)))
}

#[test]
fn last_push_is_reported_until_a_push_succeeds_and_a_success_once() {
    let mut home = Home::new();
    let root = home.brain("alpha");
    // A failure, every time, and the record stays.
    let failure = write_record(&root, PushResult::Refused, ago(1));
    for _ in 0..2 {
        let report = read(&home, &root).expect("a report");
        assert_eq!(report.last_push.as_ref(), Some(&failure));
        assert!(git_file(&root, LAST_PUSH).exists());
    }
    let json = read(&home, &root).unwrap().to_json();
    let text = kluris_core::json::to_string(&json);
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        doc["last_push"],
        serde_json::json!({"at": failure.at, "result": "refused", "detail": "why", "commits": 1})
    );

    // A success, once; then the record goes.
    let success = write_record(&root, PushResult::Pushed, ago(1));
    let report = read(&home, &root).expect("a report");
    assert_eq!(report.last_push.as_ref(), Some(&success));
    assert!(!git_file(&root, LAST_PUSH).exists(), "the success stayed");
    assert_eq!(read(&home, &root), None, "nothing more to say");

    // Not while a live run may be replacing it.
    let success = write_record(&root, PushResult::Pushed, ago(1));
    write_lock(
        &root,
        std::process::id(),
        &background::this_host(),
        SystemTime::now(),
    );
    let report = read(&home, &root).expect("a report");
    assert_eq!(report.last_push.as_ref(), Some(&success));
    assert!(
        git_file(&root, LAST_PUSH).exists(),
        "removed under a live run"
    );
}

#[test]
fn an_explicit_push_that_succeeds_removes_a_failure_record() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    write_record(&root, PushResult::Offline, ago(1));
    let open = session::open(brain(&root), Intent::Explicit, &env(&home, Some(kluris())));
    let report = session::close(open).expect("a report");
    assert!(report.pushed, "{report:?}");
    assert!(!git_file(&root, LAST_PUSH).exists(), "the failure stayed");
    assert_eq!(remote_head(&home, "alpha"), local_head(&home, &root));
}

#[test]
fn the_hidden_flags() {
    let home = Home::new();
    for args in [
        &["sync", "--background", "--brain", "alpha"][..],
        &["sync", "--path", "x"],
    ] {
        let (code, out, err) = home.run(args);
        assert_eq!(code, 2, "{args:?}: {out}{err}");
    }
    // Under --json a usage error is the JSON error, exit 1, as for every
    // command.
    let (code, out, _) = home.run(&["sync", "--background", "--json"]);
    assert_eq!(code, 1, "{out}");
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["ok"], false, "{doc}");
    assert!(
        doc["error"].as_str().unwrap().contains("--background"),
        "{doc}"
    );
    let (code, out, _) = home.run(&["sync", "--help"]);
    assert_eq!(code, 0);
    assert!(
        !out.contains("--background") && !out.contains("--path"),
        "{out}"
    );
}

/// The child's environment: what it does, and the brains (joined as
/// `PATH` is). Its git runs in the home's hermetic environment.
const CHILD_ACTION: &str = "KLURIS_TEST_BACKGROUND_ACTION";
const CHILD_ROOTS: &str = "KLURIS_TEST_BACKGROUND_ROOTS";
/// The process whose console `console-of` looks at (Windows).
#[cfg(windows)]
const CHILD_PID: &str = "KLURIS_TEST_BACKGROUND_PID";

/// This test binary run again as a command that ends: `read` opens the
/// brains for a read, closes them, prints `done` and exits; `start-in-job`
/// (Windows) joins a kill-on-close job that forbids breakaway, then
/// prints what a start and a read's close did; `console-of` (Windows)
/// prints whether the process [`CHILD_PID`] names has a console. Without those variables it
/// does nothing.
#[test]
#[ignore = "run only as the child of the background tests"]
fn child_command() {
    let (Some(action), Some(roots)) = (
        std::env::var(CHILD_ACTION).ok(),
        std::env::var_os(CHILD_ROOTS),
    ) else {
        return;
    };
    let roots: Vec<PathBuf> = std::env::split_paths(&roots).collect();
    let env = SessionEnv {
        git: SystemGit::new(),
        no_verify: false,
        now: SystemTime::now(),
        program: Some(kluris()),
    };
    match action.as_str() {
        "read" => {
            let brains: Vec<Brain> = roots.iter().map(|root| brain(root)).collect();
            let sessions = session::open_all(&brains, Intent::Read, &env);
            let reports = session::close_all(sessions);
            for report in reports {
                println!("to_push {}", report.map_or(0, |r| r.to_push));
            }
            println!("done");
        }
        #[cfg(windows)]
        "start-in-job" => {
            windows_job::join_kill_on_close_job();
            match background::start(&kluris(), &roots) {
                background::Start::Refused(_) => println!("refused"),
                other => println!("not refused: {other:?}"),
            }
            let report = session::close(session::open(brain(&roots[0]), Intent::Read, &env));
            println!("to_push {}", report.map_or(0, |r| r.to_push));
        }
        #[cfg(windows)]
        "console-of" => {
            let pid = std::env::var(CHILD_PID).unwrap().parse().unwrap();
            println!("{}", windows_job::console_of(pid));
        }
        other => panic!("no action {other}"),
    }
}

/// The child command doing `action` on `roots`, its stdout piped.
fn command(home: &Home, action: &str, roots: &[&Path]) -> Child {
    command_line(home, action, roots).spawn().unwrap()
}

/// [`command`], not started yet.
fn command_line(home: &Home, action: &str, roots: &[&Path]) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    home.hermetic(&mut cmd);
    cmd.args([
        "--ignored",
        "--exact",
        "child_command",
        "--quiet",
        "--nocapture",
    ])
    .env(CHILD_ACTION, action)
    .env(CHILD_ROOTS, std::env::join_paths(roots).unwrap())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    cmd
}

/// A read closed with commits waiting in two brains starts one child for
/// both; a brain with nothing waiting is not handed to it.
#[cfg(unix)]
#[test]
fn one_child_pushes_every_brain_with_commits_waiting() {
    let mut home = Home::new();
    let alpha = waiting(&mut home, "alpha");
    let beta = waiting(&mut home, "beta");
    let gamma = home.brain("gamma");
    let mut child = command(&home, "read", &[&alpha, &beta, &gamma]);
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(child.wait().unwrap().success());
    let counts: Vec<&str> = out.lines().filter(|l| l.starts_with("to_push")).collect();
    assert_eq!(counts, ["to_push 1", "to_push 1", "to_push 0"], "{out}");
    for (name, root) in [("alpha", &alpha), ("beta", &beta)] {
        eventually(&format!("{name}'s push"), || result_of(root).is_some());
        assert_eq!(result_of(root).as_deref(), Some("pushed"), "{name}");
        assert_eq!(remote_head(&home, name), local_head(&home, root), "{name}");
    }
    assert_eq!(record(&gamma), None);
    let pid = |root: &Path| {
        let log = fs::read_to_string(git_file(root, LOG)).unwrap();
        log.lines()
            .find_map(|line| {
                let rest = line.split("(pid ").nth(1)?;
                Some(rest.split(')').next()?.to_string())
            })
            .unwrap_or_else(|| panic!("no pid in {log}"))
    };
    assert_eq!(pid(&alpha), pid(&beta), "more than one child");
}

/// The command's stdout closes as it ends, while the child it started
/// still pushes, in a session of its own.
#[cfg(unix)]
#[test]
fn the_commands_stdout_closes_while_the_child_pushes_in_its_own_session() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let release = home.dir.path().join("release");
    stall_pushes(&home, &root, &release);
    let mut command = command(&home, "read", &[&root]);
    let mut out = BufReader::new(command.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "no done");
        if line.trim() == "done" {
            break;
        }
    }
    let done = Instant::now();
    let mut rest = Vec::new();
    out.read_to_end(&mut rest).unwrap();
    let closed = done.elapsed();
    assert!(
        closed < Duration::from_secs(1),
        "stdout closed after {closed:?}"
    );
    assert!(command.wait().unwrap().success());

    eventually("the child's lock", || git_file(&root, LOCK).exists());
    let lock = Lock::read(&git_file(&root, LOCK)).unwrap();
    assert_eq!(record(&root), None, "the push did not wait");
    let pid = rustix::process::Pid::from_raw(i32::try_from(lock.pid).unwrap()).unwrap();
    let its = rustix::process::getsid(Some(pid)).unwrap();
    let ours = rustix::process::getsid(None).unwrap();
    assert_eq!(its, pid, "not a session leader");
    assert_ne!(its, ours);

    fs::write(&release, "").unwrap();
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"));
    eventually("the lock to go", || !git_file(&root, LOCK).exists());
}

/// A start from this process, judged by this process's own job, read as
/// the start reads it: a job that ends its processes when it closes and
/// forbids breakaway (as `cargo xtask windows` runs tests in) starts
/// nothing, and the commits stay counted; any other place starts a child
/// with no console, outside this process's job unless the job keeps its
/// children, that pushes.
#[cfg(windows)]
#[test]
fn on_windows_the_job_this_process_is_in_decides_the_start() {
    let job = windows_job::Job::ours();
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let release = home.dir.path().join("release");
    stall_pushes(&home, &root, &release);
    let started = background::start(&kluris(), std::slice::from_ref(&root));
    if job.refuses_children() {
        let background::Start::Refused(_) = started else {
            panic!("{job}: expected nothing started, got {started:?}");
        };
        assert!(!git_file(&root, LOCK).exists(), "{job}: a child runs");
        let env = env(&home, Some(kluris()));
        let report = session::close(session::open(brain(&root), Intent::Read, &env));
        assert_eq!(report.map(|r| r.to_push), Some(1), "{job}: not counted");
        assert!(!git_file(&root, LOCK).exists(), "{job}: a read started one");
        return;
    }
    let background::Start::Started(pid) = started else {
        panic!("{job}: expected a child started, got {started:?}");
    };
    eventually("the child's lock", || git_file(&root, LOCK).exists());
    assert_eq!(
        Lock::read(&git_file(&root, LOCK)).unwrap().pid,
        pid,
        "{job}"
    );
    assert_eq!(
        windows_job::in_ours(pid),
        job.keeps_children(),
        "{job}: the child's place in this process's job"
    );
    let mut probe = command_line(&home, "console-of", &[&root])
        .env(CHILD_PID, pid.to_string())
        .spawn()
        .unwrap();
    let mut out = String::new();
    probe
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(probe.wait().unwrap().success(), "{out}");
    assert!(out.lines().any(|l| l == "no console"), "{job}: {out}");
    fs::write(&release, "").unwrap();
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"), "{job}");
}

/// In a kill-on-close job that forbids breakaway, nothing starts and the
/// commits stay counted.
#[cfg(windows)]
#[test]
fn in_a_kill_on_close_job_nothing_starts_and_the_commits_stay_counted() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let mut child = command(&home, "start-in-job", &[&root]);
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    assert!(child.wait().unwrap().success(), "{out}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| *l == "refused" || l.starts_with("to_push") || l.starts_with("not refused"))
        .collect();
    assert_eq!(lines, ["refused", "to_push 1"], "{out}");
    assert!(!git_file(&root, LOCK).exists());
    assert_eq!(record(&root), None);
}

#[cfg(windows)]
mod windows_job {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_HANDLE, FALSE, GetLastError};
    use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
        JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Join a new job that ends its processes when its last handle closes
    /// and forbids breakaway; its handle is kept for the process's life.
    pub fn join_kill_on_close_job() {
        // SAFETY: plain Win32 calls on a job handle made here and this
        // process's pseudo handle; the handle is left open on purpose.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            assert!(!job.is_null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(std::mem::size_of_val(&info)).unwrap();
            assert_ne!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size
                ),
                0
            );
            assert_ne!(AssignProcessToJobObject(job, GetCurrentProcess()), 0);
        }
    }

    /// This process's own job, as `QueryInformationJobObject` with no
    /// handle reads it.
    pub struct Job {
        /// Its limit flags; `None`: this process is in no job.
        flags: Option<u32>,
    }

    impl Job {
        pub fn ours() -> Job {
            // SAFETY: a query of this process's job into a local of its
            // size.
            unsafe {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                let size = u32::try_from(std::mem::size_of_val(&info)).unwrap();
                let read = QueryInformationJobObject(
                    std::ptr::null_mut(),
                    JobObjectExtendedLimitInformation,
                    (&raw mut info).cast(),
                    size,
                    std::ptr::null_mut(),
                );
                Job {
                    flags: (read != 0).then_some(info.BasicLimitInformation.LimitFlags),
                }
            }
        }

        fn has(&self, flag: u32) -> bool {
            self.flags.is_some_and(|flags| flags & flag != 0)
        }

        /// A child cannot leave it: in a job, with neither breakaway flag.
        pub fn keeps_children(&self) -> bool {
            self.flags.is_some()
                && !self.has(JOB_OBJECT_LIMIT_BREAKAWAY_OK)
                && !self.has(JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK)
        }

        /// A child would end with this process: the start refuses.
        pub fn refuses_children(&self) -> bool {
            self.keeps_children() && self.has(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
        }
    }

    impl std::fmt::Display for Job {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let Some(flags) = self.flags else {
                return write!(f, "this process is in no job (expected a child started)");
            };
            let named = [
                (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, "KILL_ON_JOB_CLOSE"),
                (JOB_OBJECT_LIMIT_BREAKAWAY_OK, "BREAKAWAY_OK"),
                (JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, "SILENT_BREAKAWAY_OK"),
            ]
            .iter()
            .map(|(flag, name)| format!("{}{name}", if flags & flag != 0 { "" } else { "no " }))
            .collect::<Vec<_>>()
            .join(", ");
            let expected = if self.refuses_children() {
                "nothing started"
            } else {
                "a child started"
            };
            write!(
                f,
                "this process's job has flags {flags:#x} ({named}); expected {expected}"
            )
        }
    }

    /// Whether the process `pid` is in this process's job (in any job
    /// when this process is in none, which is then none of its).
    pub fn in_ours(pid: u32) -> bool {
        if Job::ours().flags.is_none() {
            // SAFETY: a handle opened here and closed here.
            return unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
                assert!(!process.is_null(), "no process {pid}");
                let mut result = FALSE;
                let asked = IsProcessInJob(process, std::ptr::null_mut(), &raw mut result);
                CloseHandle(process);
                asked != 0 && result != FALSE
            };
        }
        let header = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
        let mut buffer = vec![0usize; 4096];
        let size = u32::try_from(std::mem::size_of_val(buffer.as_slice())).unwrap();
        // SAFETY: the list is written into a buffer of `size` bytes,
        // aligned for it; its count says how many ids follow the header.
        unsafe {
            let list = buffer
                .as_mut_ptr()
                .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let read = QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectBasicProcessIdList,
                list.cast(),
                size,
                std::ptr::null_mut(),
            );
            assert_ne!(read, 0, "the job's processes could not be read");
            let count = usize::try_from((*list).NumberOfProcessIdsInList).unwrap();
            let ids = buffer.as_ptr().cast::<u8>().add(header).cast::<usize>();
            (0..count).any(|i| *ids.add(i) == pid as usize)
        }
    }

    /// `no console` when the process `pid` has none (attaching to it is
    /// refused as an invalid handle), else what attaching did.
    pub fn console_of(pid: u32) -> String {
        // SAFETY: this probe's own console is let go, then one is asked
        // for; one that is had is let go again.
        unsafe {
            FreeConsole();
            if AttachConsole(pid) != 0 {
                FreeConsole();
                return "attached".into();
            }
            match GetLastError() {
                ERROR_INVALID_HANDLE => "no console".into(),
                error => format!("not attached: error {error}"),
            }
        }
    }
}
