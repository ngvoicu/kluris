//! The background push: a read or housekeeping session closed with commits
//! waiting starts one detached `kluris sync --background --path <root>...`
//! for every such brain. The child fetches and pushes only, under the
//! brain's `background.lock`, logs to `background.log`, and records what
//! happened in `last-push.json`, which the next command reports.
//!
//! The sessions are driven through `kluris_core::sync::session`, with this
//! package's binary as the program a session starts. A command that closes
//! a read and exits is this test binary run again as a child. Every child
//! is waited for with a bound and ended by its handle on every way out,
//! and every pipe is read in a thread, waited for with a bound.

mod sync_home;

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
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

/// A child process, ended by its handle when dropped unless it ended.
struct Running(Child);

impl Running {
    fn start(mut cmd: Command) -> Running {
        Running(cmd.spawn().unwrap())
    }

    fn id(&self) -> u32 {
        self.0.id()
    }

    /// Its exit, waited for at most [`PATIENCE`].
    fn wait(&mut self) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(start.elapsed() < PATIENCE, "the child did not end");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Its stdout, read line by line in a thread.
    fn lines(&mut self) -> Lines {
        Lines::of(self.0.stdout.take().expect("a piped stdout"))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// A pipe's lines as a thread reads them; `None` once it closed.
struct Lines(Receiver<Option<String>>);

impl Lines {
    fn of(pipe: impl Read + Send + 'static) -> Lines {
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(pipe);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => {
                        let _ = send.send(None);
                        return;
                    }
                    Ok(_) => {
                        let _ = send.send(Some(line.trim_end().to_string()));
                    }
                }
            }
        });
        Lines(receive)
    }

    /// The lines up to and with `last`, waited for at most [`PATIENCE`].
    fn until(&self, last: &str) -> Vec<String> {
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            let left = PATIENCE.saturating_sub(start.elapsed());
            match self.0.recv_timeout(left) {
                Ok(Some(line)) => {
                    let done = line == last;
                    seen.push(line);
                    if done {
                        return seen;
                    }
                }
                Ok(None) => panic!("the pipe closed before {last:?}: {seen:?}"),
                Err(_) => panic!("no {last:?} within {PATIENCE:?}: {seen:?}"),
            }
        }
    }

    /// Whether the pipe closes within `bound`; what it said meanwhile.
    fn closes_within(&self, bound: Duration) -> Result<Vec<String>, Vec<String>> {
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            match self.0.recv_timeout(bound.saturating_sub(start.elapsed())) {
                Ok(Some(line)) => seen.push(line),
                Ok(None) => return Ok(seen),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    return Err(seen);
                }
            }
        }
    }

    /// Everything until the pipe closes, waited for at most [`PATIENCE`].
    fn all(&self) -> Vec<String> {
        self.closes_within(PATIENCE)
            .unwrap_or_else(|seen| panic!("the pipe stayed open: {seen:?}"))
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

/// `kluris sync --background` with `args`, run here in `cwd` and waited
/// for: its exit code, stdout and stderr.
fn push_in(home: &Home, cwd: &Path, args: &[&Path]) -> (i32, String, String) {
    let mut cmd = Command::new(kluris());
    home.hermetic(&mut cmd);
    cmd.args(["sync", "--background"]);
    for arg in args {
        cmd.arg("--path").arg(arg);
    }
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Running::start(cmd);
    let err = Lines::of(child.0.stderr.take().unwrap());
    let out = child.lines();
    let status = child.wait();
    (
        status.code().unwrap_or(-1),
        out.all().join("\n"),
        err.all().join("\n"),
    )
}

/// `kluris sync --background --path <root>...`, run here and waited for.
fn push_in_foreground(home: &Home, roots: &[&Path]) -> (i32, String, String) {
    push_in(home, home.dir.path(), roots)
}

/// HEAD, the index's bytes, the status and every file of the work tree.
type Snapshot = (String, Vec<u8>, String, Vec<(PathBuf, Vec<u8>)>);

fn snapshot(home: &Home, root: &Path) -> Snapshot {
    let index = fs::read(root.join(".git").join("index")).unwrap();
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
    // The status refreshes no index (no optional locks).
    let status = home.git(
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    );
    (local_head(home, root), index, status, files)
}

/// A script's path as git's shell takes it.
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A `receive-pack` for the brain's remote that waits until `release`
/// exists (60 s at most), then receives.
fn stall_pushes(home: &Home, root: &Path, release: &Path) {
    let script = home.dir.path().join("stalled-receive-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ni=0\nwhile [ ! -e '{}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done\nexec git receive-pack \"$@\"\n",
            sh_path(release)
        ),
    )
    .unwrap();
    let pack = format!("sh '{}'", sh_path(&script));
    home.git(root, &["config", "remote.origin.receivepack", &pack]);
}

/// Writes `release` when dropped, so a stalled child goes on however the
/// test ends.
struct Release(PathBuf);

impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, "");
    }
}

/// A hook of the brain's that prints `text` on stderr and exits `code`.
fn hook(root: &Path, name: &str, text: &str, code: i32) {
    script_hook(root, name, &format!("echo '{text}' >&2\nexit {code}"));
}

/// A hook of the brain's running `body`.
fn script_hook(root: &Path, name: &str, body: &str) {
    let path = root.join(".git").join("hooks").join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// `kluris sync --background --path <root>` started here, not waited for.
fn spawn_push(home: &Home, root: &Path) -> Running {
    let mut cmd = Command::new(kluris());
    home.hermetic(&mut cmd);
    cmd.args(["sync", "--background", "--path"])
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Running::start(cmd)
}

/// A process id that is no process now.
fn dead_pid() -> u32 {
    let mut cmd = Command::new("git");
    cmd.arg("--version").stdout(Stdio::null());
    let mut child = Running::start(cmd);
    let pid = child.id();
    child.wait();
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

fn a_record(result: PushResult, at: SystemTime, commits: usize) -> LastPush {
    LastPush {
        at: background::timestamp(at),
        head: "0".repeat(40),
        result,
        detail: (result != PushResult::Pushed).then(|| "why".to_string()),
        commits,
    }
}

fn write_record(root: &Path, result: PushResult, at: SystemTime) -> LastPush {
    let record = a_record(result, at, 1);
    fs::create_dir_all(git_file(root, "kluris-sync")).unwrap();
    record.write(&git_file(root, LAST_PUSH)).unwrap();
    record
}

fn ago(minutes: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(60 * minutes)
}

/// An ssh for the brain's remote that answers as an ssh that cannot reach
/// the host does, with no network: `Connection refused`, exit 255.
fn unreachable_ssh(home: &Home, root: &Path) {
    let script = home.dir.path().join("unreachable-ssh.sh");
    fs::write(
        &script,
        "#!/bin/sh\necho 'ssh: connect to host unreachable.example port 22: Connection refused' >&2\nexit 255\n",
    )
    .unwrap();
    let ssh = format!("sh '{}'", sh_path(&script));
    home.git(root, &["config", "core.sshCommand", &ssh]);
    home.git(
        root,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@unreachable.example/x.git",
        ],
    );
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
    assert!(
        snapshot(&home, &root) == before,
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
    assert!(snapshot(&home, &root) == before, "something landed");
}

#[test]
fn a_push_the_remote_turns_back_because_it_moved_is_needs_pull() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    // The teammate pushes between the child's fetch and its push.
    let mate = sh_path(&home.teammate("alpha"));
    script_hook(
        &root,
        "pre-push",
        &format!(
            "unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\ncd '{mate}' && echo racing >> notes/race.md && git add -A && git commit -q -m race && git push -q >&2"
        ),
    );
    let before = snapshot(&home, &root);
    let (code, out, err) = push_in_foreground(&home, &[&root]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        result_of(&root).as_deref(),
        Some("needs_pull"),
        "{:?}",
        record(&root)
    );
    assert!(snapshot(&home, &root) == before, "something landed");
}

#[test]
fn each_failure_is_recorded_with_its_detail() {
    // Offline: the remote's host cannot be reached.
    {
        let mut home = Home::new();
        let root = waiting(&mut home, "alpha");
        unreachable_ssh(&home, &root);
        let (code, ..) = push_in_foreground(&home, &[&root]);
        assert_eq!(code, 0);
        assert_eq!(
            result_of(&root).as_deref(),
            Some("offline"),
            "{:?}",
            record(&root)
        );
        let doc = record(&root).unwrap();
        assert_eq!(doc["commits"], 1);
        // ssh's own line says why.
        assert_eq!(
            doc["detail"], "ssh: connect to host unreachable.example port 22: Connection refused",
            "{doc}"
        );
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
        script_hook(
            &root,
            "reference-transaction",
            "[ \"$1\" = prepared ] && grep -q refs/remotes/ && exit 1\nexit 0",
        );
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
    let release = Release(home.dir.path().join("release"));
    stall_pushes(&home, &root, &release.0);
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

    drop(release);
    assert!(first.wait().success());
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
    for _ in 0..3 {
        let report = read(&home, &root).expect("a report");
        assert_eq!(report.last_push.as_ref(), Some(&failure));
        assert_eq!(
            background::LastPush::read(&git_file(&root, LAST_PUSH)).as_ref(),
            Some(&failure)
        );
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
}

/// The lock every write and claim of the brain's record takes, held by
/// this test until the file is dropped.
fn hold_record_lock(root: &Path) -> fs::File {
    fs::create_dir_all(git_file(root, "kluris-sync")).unwrap();
    let gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(git_file(root, background::LAST_PUSH_CLAIM))
        .unwrap();
    gate.lock().unwrap();
    gate
}

/// No file of a claim is left beside the record.
fn no_claimed_files(root: &Path) {
    let left: Vec<String> = fs::read_dir(git_file(root, "kluris-sync"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("last-push.json") && name != "last-push.json")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

/// `jobs` run on threads that start together, each result waited for at
/// most [`PATIENCE`]: a job that never ends fails the test, never hangs it.
fn together<T: Send + 'static>(jobs: Vec<Box<dyn FnOnce() -> T + Send>>) -> Vec<T> {
    let go = Arc::new(AtomicBool::new(false));
    let (send, receive) = mpsc::channel();
    let count = jobs.len();
    for job in jobs {
        let (go, send) = (Arc::clone(&go), send.clone());
        thread::spawn(move || {
            let start = Instant::now();
            while !go.load(Ordering::Acquire) && start.elapsed() < PATIENCE {
                thread::yield_now();
            }
            let _ = send.send(job());
        });
    }
    go.store(true, Ordering::Release);
    (0..count)
        .map(|n| {
            receive
                .recv_timeout(PATIENCE)
                .unwrap_or_else(|_| panic!("job {n} of {count} did not end within {PATIENCE:?}"))
        })
        .collect()
}

/// Readers at once on one success: one reports it, and a failure is
/// reported by every one and stays; threads and processes alike.
#[test]
fn readers_at_once_report_one_success_once_and_every_failure() {
    let mut home = Home::new();
    let root = home.brain("alpha");
    let common = root.join(".git");
    for round in 0..40 {
        let success = write_record(&root, PushResult::Pushed, ago(1));
        let jobs = (0..8)
            .map(|_| {
                let common = common.clone();
                Box::new(move || background::report_last_push(&common))
                    as Box<dyn FnOnce() -> Option<LastPush> + Send>
            })
            .collect();
        let reported: Vec<LastPush> = together(jobs).into_iter().flatten().collect();
        assert_eq!(reported, [success], "round {round}");
        assert!(!git_file(&root, LAST_PUSH).exists(), "round {round}");
    }
    let failure = write_record(&root, PushResult::Offline, ago(1));
    let jobs = (0..8)
        .map(|_| {
            let common = common.clone();
            Box::new(move || background::report_last_push(&common))
                as Box<dyn FnOnce() -> Option<LastPush> + Send>
        })
        .collect();
    for reported in together(jobs) {
        assert_eq!(reported.as_ref(), Some(&failure));
    }
    assert_eq!(
        background::LastPush::read(&git_file(&root, LAST_PUSH)).as_ref(),
        Some(&failure)
    );

    // Read sessions at once, the same way.
    let success = write_record(&root, PushResult::Pushed, ago(1));
    let jobs = (0..4)
        .map(|_| {
            let (root, git) = (root.clone(), self::git(&home));
            Box::new(move || {
                let env = SessionEnv {
                    git,
                    no_verify: false,
                    now: SystemTime::now(),
                    program: None,
                };
                session::close(session::open(brain(&root), Intent::Read, &env))
                    .and_then(|r| r.last_push)
            }) as Box<dyn FnOnce() -> Option<LastPush> + Send>
        })
        .collect();
    let reported: Vec<LastPush> = together(jobs).into_iter().flatten().collect();
    assert_eq!(reported, [success]);

    // Commands at once, each its own process.
    let success = write_record(&root, PushResult::Pushed, ago(1));
    let mut readers: Vec<(Running, Lines)> =
        (0..4).map(|_| command(&home, "report", &[&root])).collect();
    let said: Vec<String> = readers
        .iter_mut()
        .flat_map(|(child, lines)| {
            let said = lines.all();
            assert!(child.wait().success(), "{said:?}");
            said
        })
        .filter(|line| line.starts_with("report "))
        .collect();
    let reports = said.iter().filter(|l| l.contains("Some(LastPush")).count();
    assert_eq!(reports, 1, "{said:?}");
    assert!(said.iter().any(|l| l.contains(&success.at)), "{said:?}");
    no_claimed_files(&root);
}

/// A record the background run writes while a claim holds the lock waits
/// for it, then lands, and survives: a failure is never lost.
#[test]
fn a_record_written_while_a_claim_holds_the_lock_lands_after_it() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let success = write_record(&root, PushResult::Pushed, ago(2));
    hook(&root, "pre-push", "not today", 1);
    let gate = hold_record_lock(&root);
    let mut child = spawn_push(&home, &root);
    // The push is refused: the child's next step is its record.
    eventually("the refused push", || {
        fs::read_to_string(git_file(&root, LOG)).is_ok_and(|log| log.contains("not today"))
    });
    let held = Instant::now();
    while held.elapsed() < Duration::from_millis(500) {
        assert_eq!(
            background::LastPush::read(&git_file(&root, LAST_PUSH)).as_ref(),
            Some(&success),
            "written while the lock was held"
        );
        thread::sleep(Duration::from_millis(25));
    }
    // The claim takes the success it read, then lets the lock go.
    fs::remove_file(git_file(&root, LAST_PUSH)).unwrap();
    drop(gate);
    assert!(child.wait().success());
    assert_eq!(result_of(&root).as_deref(), Some("refused"));
    for _ in 0..2 {
        let report = read(&home, &root).expect("a report");
        assert_eq!(
            report.last_push.map(|r| r.result),
            Some(PushResult::Refused)
        );
    }
    assert_eq!(result_of(&root).as_deref(), Some("refused"));
    no_claimed_files(&root);
}

/// A read that finds the lock held never waits for it: it reports a
/// failure it can read, leaves a success for another command, and ends in
/// its own time.
#[test]
fn a_held_lock_never_holds_a_read_up() {
    let mut home = Home::new();
    let root = home.brain("alpha");
    let gate = hold_record_lock(&root);
    let reading = |home: &Home| {
        let (root, git) = (root.clone(), git(home));
        let jobs = vec![Box::new(move || {
            let env = SessionEnv {
                git,
                no_verify: false,
                now: SystemTime::now(),
                program: None,
            };
            let started = Instant::now();
            let report = session::close(session::open(brain(&root), Intent::Read, &env));
            (report, started.elapsed())
        })
            as Box<dyn FnOnce() -> (Option<SyncReport>, Duration) + Send>];
        together(jobs).pop().unwrap()
    };
    let failure = write_record(&root, PushResult::Refused, ago(1));
    let (report, took) = reading(&home);
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert_eq!(report.and_then(|r| r.last_push), Some(failure));

    let success = write_record(&root, PushResult::Pushed, ago(1));
    let (report, took) = reading(&home);
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert_eq!(report.and_then(|r| r.last_push), None);
    assert_eq!(
        background::LastPush::read(&git_file(&root, LAST_PUSH)).as_ref(),
        Some(&success),
        "a success taken under another's lock"
    );

    drop(gate);
    let (report, _) = reading(&home);
    assert_eq!(report.and_then(|r| r.last_push), Some(success));
    assert!(!git_file(&root, LAST_PUSH).exists());
    no_claimed_files(&root);
}

/// Taking the record removes it only when it is the one meant, under the
/// lock; with the lock held past the bound nothing is taken.
#[test]
fn the_record_is_taken_only_when_it_is_the_one_meant() {
    let mut home = Home::new();
    let root = home.brain("alpha");
    let common = root.join(".git");
    let path = git_file(&root, LAST_PUSH);
    let seen = write_record(&root, PushResult::Offline, ago(2));
    let newer = a_record(PushResult::Refused, ago(1), 2);
    newer.write(&path).unwrap();
    let bound = Duration::from_millis(200);
    assert_eq!(
        background::take_last_push(&common, bound, |r| *r == seen),
        None
    );
    assert_eq!(background::LastPush::read(&path).as_ref(), Some(&newer));

    let gate = hold_record_lock(&root);
    let started = Instant::now();
    let (common_held, newer_held) = (common.clone(), newer.clone());
    let jobs =
        vec![
            Box::new(move || background::take_last_push(&common_held, bound, |r| *r == newer_held))
                as Box<dyn FnOnce() -> Option<LastPush> + Send>,
        ];
    assert_eq!(together(jobs).pop().unwrap(), None);
    assert!(started.elapsed() >= bound, "did not wait its bound");
    assert!(path.exists(), "taken under another's lock");
    drop(gate);

    assert_eq!(
        background::take_last_push(&common, bound, |r| *r == newer).as_ref(),
        Some(&newer)
    );
    assert!(!path.exists());
    assert_eq!(background::take_last_push(&common, bound, |_| true), None);
    no_claimed_files(&root);
}

#[test]
fn an_explicit_push_that_succeeds_removes_the_failure_record_it_read() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    write_record(&root, PushResult::Offline, ago(1));
    let open = session::open(brain(&root), Intent::Explicit, &env(&home, Some(kluris())));
    let report = session::close(open).expect("a report");
    assert!(report.pushed, "{report:?}");
    assert_eq!(report.last_push, None, "a failure the push made moot");
    assert!(!git_file(&root, LAST_PUSH).exists(), "the failure stayed");
    assert_eq!(remote_head(&home, "alpha"), local_head(&home, &root));

    // A record published while the push ran is newer than the one read
    // before it: it stays.
    home.commit(&root, "notes/more.md", &neuron("More", "M.\n"));
    write_record(&root, PushResult::Offline, ago(2));
    let newer = a_record(PushResult::Refused, ago(1), 3);
    let path = git_file(&root, LAST_PUSH);
    let text = fs::read_to_string({
        let draft = home.dir.path().join("newer.json");
        newer.write(&draft).unwrap();
        draft
    })
    .unwrap();
    script_hook(
        &root,
        "pre-push",
        &format!("printf '%s' '{}' > '{}'", text.trim(), sh_path(&path)),
    );
    let open = session::open(brain(&root), Intent::Explicit, &env(&home, None));
    let report = session::close(open).expect("a report");
    assert!(report.pushed, "{report:?}");
    assert_eq!(background::LastPush::read(&path).as_ref(), Some(&newer));
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

/// A brain `name` in `dir`, outside the home, pushed to `<dir>/<name>.git`,
/// with a commit of its own waiting.
fn decoy(home: &Home, dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    let bare = dir.join(format!("{name}.git"));
    fs::create_dir_all(&root).unwrap();
    home.git(&root, &["init", "-q", "-b", "main"]);
    home.configure(&root, "Dee Coy", "decoy@example.com");
    home.commit(&root, "decoy.md", "# Decoy\n");
    home.git(
        dir,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            &bare.to_string_lossy(),
        ],
    );
    home.git(&root, &["remote", "add", "origin", &bare.to_string_lossy()]);
    home.git(&root, &["push", "-q", "-u", "origin", "main"]);
    home.commit(&root, "waiting.md", "# Waiting\n");
    root
}

/// What would show Kluris touched the brain `name` in `dir`: its refs,
/// its remote's, every file of its work tree with its bytes, and whether
/// it has a `kluris-sync` folder. (git's own housekeeping in `.git`, a
/// fixture commit's auto-maintenance, is not Kluris's.)
type Untouched = (String, String, Vec<(PathBuf, Vec<u8>)>, bool);

fn untouched(home: &Home, dir: &Path, name: &str) -> Untouched {
    let root = dir.join(name);
    let bare = dir.join(format!("{name}.git"));
    let mut files = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).unwrap().flatten() {
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
        home.git(&root, &["for-each-ref"]),
        home.git(&bare, &["for-each-ref"]),
        files,
        root.join(".git").join("kluris-sync").exists(),
    )
}

#[test]
fn a_relative_path_is_refused_and_nothing_in_it_is_touched() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let before = untouched(&home, home.dir.path(), "alpha");
    let (code, out, err) = push_in(&home, home.dir.path(), &[Path::new("alpha")]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(record(&root), None);
    assert!(!git_file(&root, LOCK).exists());
    assert!(!git_file(&root, LOG).exists());
    assert!(untouched(&home, home.dir.path(), "alpha") == before);
}

/// The child's environment: what it does, the brains (joined as `PATH`
/// is), and the program to start. Its git runs in the home's hermetic
/// environment.
const CHILD_ACTION: &str = "KLURIS_TEST_BACKGROUND_ACTION";
const CHILD_ROOTS: &str = "KLURIS_TEST_BACKGROUND_ROOTS";
const CHILD_PROGRAM: &str = "KLURIS_TEST_BACKGROUND_PROGRAM";
/// The process whose console `console-of` looks at (Windows).
#[cfg(windows)]
const CHILD_PID: &str = "KLURIS_TEST_BACKGROUND_PID";

/// This test binary run again as a command that ends, in the home:
/// - `read` opens the brains for a read and closes them, printing each
///   `to_push`, then `done`;
/// - `report` prints what reporting the first brain's record gives;
/// - `late-descriptor` (Unix) starts the program with a descriptor opened
///   as the start forks (`pthread_atfork`), then prints `done`;
/// - `lowered-limit` (Unix) starts the program with a descriptor open
///   above a descriptor limit lowered after it opened, then prints `done`;
/// - `start-in-job` (Windows) joins a kill-on-close job that forbids
///   breakaway, then prints what a start and a read's close did;
/// - `read-in-job` (Windows) joins a job that forbids breakaway and does
///   not kill on close, nested in the one it is in, then reads as `read`
///   does;
/// - `console-of` (Windows) prints whether the process `CHILD_PID` names
///   has a console.
///
/// Without those variables it does nothing.
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
    let program = std::env::var_os(CHILD_PROGRAM).map_or_else(kluris, PathBuf::from);
    let env = SessionEnv {
        git: SystemGit::new(),
        no_verify: false,
        now: SystemTime::now(),
        program: Some(program.clone()),
    };
    let read = |env: &SessionEnv| {
        let brains: Vec<Brain> = roots.iter().map(|root| brain(root)).collect();
        let sessions = session::open_all(&brains, Intent::Read, env);
        for report in session::close_all(sessions) {
            println!("to_push {}", report.map_or(0, |r| r.to_push));
        }
        println!("done");
    };
    match action.as_str() {
        "read" => read(&env),
        "report" => {
            let common = roots[0].join(".git");
            println!("report {:?}", background::report_last_push(&common));
        }
        #[cfg(unix)]
        "late-descriptor" => {
            late::open_as_the_start_forks();
            println!("start {:?}", background::start(&program, &[]));
            println!("done");
        }
        #[cfg(unix)]
        "lowered-limit" => {
            late::open_above_a_lowered_limit();
            println!("start {:?}", background::start(&program, &[]));
            println!("done");
        }
        #[cfg(windows)]
        "start-in-job" => {
            windows_job::join_kill_on_close_job();
            match background::start(&program, &roots) {
                background::Start::Refused(_) => println!("refused"),
                other => println!("not refused: {other:?}"),
            }
            let report = session::close(session::open(brain(&roots[0]), Intent::Read, &env));
            println!("to_push {}", report.map_or(0, |r| r.to_push));
        }
        #[cfg(windows)]
        "read-in-job" => {
            windows_job::join_job_that_keeps_children();
            read(&env);
        }
        #[cfg(windows)]
        "console-of" => {
            let pid = std::env::var(CHILD_PID).unwrap().parse().unwrap();
            println!("{}", windows_job::console_of(pid));
        }
        other => panic!("no action {other}"),
    }
}

/// The child command doing `action` on `roots` in the home, its stdout
/// piped; not started yet.
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
    .current_dir(home.dir.path())
    .env(CHILD_ACTION, action)
    .env(CHILD_ROOTS, std::env::join_paths(roots).unwrap())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    cmd
}

/// The child command, started, and its stdout's lines.
fn command(home: &Home, action: &str, roots: &[&Path]) -> (Running, Lines) {
    let mut child = Running::start(command_line(home, action, roots));
    let lines = child.lines();
    (child, lines)
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
    let (mut child, lines) = command(&home, "read", &[&alpha, &beta, &gamma]);
    let out = lines.all();
    assert!(child.wait().success());
    let counts: Vec<&String> = out.iter().filter(|l| l.starts_with("to_push")).collect();
    assert_eq!(counts, ["to_push 1", "to_push 1", "to_push 0"], "{out:?}");
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
    let release = Release(home.dir.path().join("release"));
    stall_pushes(&home, &root, &release.0);
    let (mut command, lines) = command(&home, "read", &[&root]);
    lines.until("done");
    if let Err(seen) = lines.closes_within(Duration::from_secs(1)) {
        panic!("stdout still open 1 s after done: {seen:?}");
    }
    assert!(command.wait().success());

    eventually("the child's lock", || git_file(&root, LOCK).exists());
    let lock = Lock::read(&git_file(&root, LOCK)).unwrap();
    assert_eq!(record(&root), None, "the push did not wait");
    let pid = rustix::process::Pid::from_raw(i32::try_from(lock.pid).unwrap()).unwrap();
    let its = rustix::process::getsid(Some(pid)).unwrap();
    let ours = rustix::process::getsid(None).unwrap();
    assert_eq!(its, pid, "not a session leader");
    assert_ne!(its, ours);

    drop(release);
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"));
    eventually("the lock to go", || !git_file(&root, LOCK).exists());
}

/// A brain opened by a relative path is the one pushed: the child, which
/// runs in the temp folder, is handed the path the command opened, never
/// one that names a brain of the same name there.
#[cfg(unix)]
#[test]
fn a_brain_opened_by_a_relative_path_is_the_one_pushed() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let temp = home.dir.path().join("temp");
    fs::create_dir_all(&temp).unwrap();
    decoy(&home, &temp, "alpha");
    let before = untouched(&home, &temp, "alpha");
    let mut cmd = command_line(&home, "read", &[Path::new("alpha")]);
    for var in ["TMPDIR", "TMP", "TEMP"] {
        cmd.env(var, &temp);
    }
    let mut command = Running::start(cmd);
    let out = command.lines().all();
    assert!(command.wait().success());
    assert!(out.iter().any(|l| l == "to_push 1"), "{out:?}");
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"));
    assert_eq!(remote_head(&home, "alpha"), local_head(&home, &root));
    eventually("the lock to go", || !git_file(&root, LOCK).exists());
    assert!(
        untouched(&home, &temp, "alpha") == before,
        "the brain of the same name in the temp folder was touched"
    );
}

/// The command doing `action` starts a program that says whether it holds
/// descriptor `fd`: it holds none, and the command's stdout closes when it
/// ends.
#[cfg(unix)]
fn no_descriptor_reaches_the_child(action: &str, fd: i32) {
    let home = Home::new();
    let report = home.dir.path().join("descriptor");
    let release = Release(home.dir.path().join("release"));
    let program = home.dir.path().join("holder.sh");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nif [ -e /dev/fd/{fd} ]; then echo open; else echo closed; fi > '{report}'\ni=0\nwhile [ ! -e '{release}' ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done\n",
            report = sh_path(&report),
            release = sh_path(&release.0),
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut cmd = command_line(&home, action, &[]);
    cmd.env(CHILD_PROGRAM, &program);
    let mut command = Running::start(cmd);
    let lines = command.lines();
    let said = lines.until("done");
    assert!(
        said.iter().any(|l| l.starts_with("start Started")),
        "{said:?}"
    );
    if let Err(seen) = lines.closes_within(Duration::from_secs(1)) {
        panic!("stdout still open 1 s after done: {seen:?}");
    }
    assert!(command.wait().success());
    eventually("the child's look", || {
        fs::read_to_string(&report).is_ok_and(|t| t.ends_with('\n'))
    });
    assert_eq!(fs::read_to_string(&report).unwrap().trim(), "closed");
}

/// Started with a descriptor opened as it forks, the child still holds
/// nothing of this process's but what it was given.
#[cfg(unix)]
#[test]
fn a_descriptor_opened_as_the_child_forks_is_closed_in_it() {
    no_descriptor_reaches_the_child("late-descriptor", late::FD);
}

/// A descriptor opened before the descriptor limit was lowered below it is
/// still open, and is closed in the child all the same.
#[cfg(unix)]
#[test]
fn a_descriptor_above_a_lowered_limit_is_closed_in_the_child() {
    no_descriptor_reaches_the_child("lowered-limit", late::ABOVE);
}

/// Descriptors a start must keep from its child: one opened by the fork
/// itself, as another thread could open one meanwhile, and one open above
/// a lowered descriptor limit.
#[cfg(unix)]
mod late {
    /// The descriptor opened as a copy of stdout.
    pub const FD: i32 = 200;

    unsafe extern "C" {
        fn pthread_atfork(
            prepare: Option<unsafe extern "C" fn()>,
            parent: Option<unsafe extern "C" fn()>,
            child: Option<unsafe extern "C" fn()>,
        ) -> i32;
        fn dup2(from: i32, to: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    unsafe extern "C" fn prepare() {
        // SAFETY: a copy of this process's stdout at a free number.
        unsafe {
            dup2(1, FD);
        }
    }

    unsafe extern "C" fn parent() {
        // SAFETY: the copy `prepare` made.
        unsafe {
            close(FD);
        }
    }

    /// The descriptor left open above the lowered limit.
    pub const ABOVE: i32 = 201;

    /// Open [`ABOVE`] as a copy of stdout, then lower this process's
    /// descriptor limit to 128, below it.
    pub fn open_above_a_lowered_limit() {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        // SAFETY: a copy of this process's stdout at a free number.
        assert_eq!(unsafe { dup2(1, ABOVE) }, ABOVE);
        let limit = getrlimit(Resource::Nofile);
        setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(128),
                maximum: limit.maximum,
            },
        )
        .unwrap();
    }

    /// From now on every fork of this process opens [`FD`] as it forks.
    pub fn open_as_the_start_forks() {
        // SAFETY: registers two handlers that only copy and close one
        // descriptor.
        assert_eq!(
            unsafe { pthread_atfork(Some(prepare), Some(parent), None) },
            0
        );
    }
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
    let release = Release(home.dir.path().join("release"));
    stall_pushes(&home, &root, &release.0);
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
    assert_eq!(console_of(&home, pid), "no console", "{job}");
    drop(release);
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"), "{job}");
}

/// What the `console-of` child says of `pid`'s console.
#[cfg(windows)]
fn console_of(home: &Home, pid: u32) -> String {
    let mut cmd = command_line(home, "console-of", &[]);
    cmd.env(CHILD_PID, pid.to_string());
    let mut probe = Running::start(cmd);
    let out = probe.lines().all();
    assert!(probe.wait().success(), "{out:?}");
    out.into_iter()
        .find(|l| l == "no console" || l == "attached" || l.starts_with("not attached"))
        .unwrap_or_else(|| "nothing said".into())
}

/// In a kill-on-close job that forbids breakaway, nothing starts and the
/// commits stay counted.
#[cfg(windows)]
#[test]
fn in_a_kill_on_close_job_nothing_starts_and_the_commits_stay_counted() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let (mut child, lines) = command(&home, "start-in-job", &[&root]);
    let out = lines.all();
    assert!(child.wait().success(), "{out:?}");
    let said: Vec<&String> = out
        .iter()
        .filter(|l| *l == "refused" || l.starts_with("to_push") || l.starts_with("not refused"))
        .collect();
    assert_eq!(said, ["refused", "to_push 1"], "{out:?}");
    assert!(!git_file(&root, LOCK).exists());
    assert_eq!(record(&root), None);
}

/// From a job that forbids breakaway and does not end its processes when
/// it closes (nested in the one this process is in), the start runs the
/// child in that job, from a program in a folder with a space, for a
/// brain the command opened by a relative path: the child has no console,
/// the command's stdout closes as it ends, and the push is recorded.
#[cfg(windows)]
#[test]
fn on_windows_a_start_the_job_keeps_runs_the_child_in_it() {
    let mut home = Home::new();
    let root = waiting(&mut home, "alpha");
    let release = Release(home.dir.path().join("release"));
    stall_pushes(&home, &root, &release.0);
    let folder = home.dir.path().join("with space");
    fs::create_dir_all(&folder).unwrap();
    let program = folder.join("kluris.exe");
    fs::copy(kluris(), &program).unwrap();
    let temp = home.dir.path().join("temp");
    fs::create_dir_all(&temp).unwrap();
    decoy(&home, &temp, "alpha");
    let before = untouched(&home, &temp, "alpha");

    let mut cmd = command_line(&home, "read-in-job", &[Path::new("alpha")]);
    cmd.env(CHILD_PROGRAM, &program);
    for var in ["TMPDIR", "TMP", "TEMP"] {
        cmd.env(var, &temp);
    }
    let mut command = Running::start(cmd);
    let lines = command.lines();
    let said = lines.until("done");
    assert!(said.iter().any(|l| l == "to_push 1"), "{said:?}");
    if let Err(seen) = lines.closes_within(Duration::from_secs(1)) {
        panic!("stdout still open 1 s after done: {seen:?}");
    }
    assert!(command.wait().success());

    eventually("the child's lock", || git_file(&root, LOCK).exists());
    let pid = Lock::read(&git_file(&root, LOCK)).unwrap().pid;
    assert_eq!(console_of(&home, pid), "no console");
    drop(release);
    eventually("the push", || result_of(&root).is_some());
    assert_eq!(result_of(&root).as_deref(), Some("pushed"));
    assert_eq!(remote_head(&home, "alpha"), local_head(&home, &root));
    eventually("the lock to go", || !git_file(&root, LOCK).exists());
    assert!(
        untouched(&home, &temp, "alpha") == before,
        "the brain of the same name in the temp folder was touched"
    );
}

/// The command line the start builds reads back, argument by argument, as
/// Windows reads it: a program and brains whose paths hold spaces and
/// quotes, and backslashes before them.
#[cfg(windows)]
#[test]
fn on_windows_the_command_line_reads_back_as_its_arguments() {
    use std::ffi::OsString;
    let program = OsString::from(r"C:\Program Files\kluris here\kluris.exe");
    let args: Vec<OsString> = [
        "sync",
        "--background",
        "--path",
        r"C:\brains\my brain",
        "--path",
        r#"C:\brains\a "quoted" brain\"#,
        "--path",
        r"C:\trailing\\",
        "--path",
        "",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let line = background::command_line(&program, &args);
    let read = windows_job::split_command_line(&line);
    let mut expected = vec![program];
    expected.extend(args);
    assert_eq!(read, expected);
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_HANDLE, FALSE, GetLastError, LocalFree,
    };
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
    use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

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

    /// Join a new job with no limits, nested in the one this process is in:
    /// it forbids breakaway and does not end its processes when it closes,
    /// so the children it keeps outlive this process.
    pub fn join_job_that_keeps_children() {
        // SAFETY: a job handle made here and this process's pseudo handle;
        // the handle is left open on purpose.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            assert!(!job.is_null());
            assert_ne!(AssignProcessToJobObject(job, GetCurrentProcess()), 0);
        }
    }

    /// `line` (NUL-terminated) split into arguments as Windows reads a
    /// command line (`CommandLineToArgvW`).
    pub fn split_command_line(line: &[u16]) -> Vec<OsString> {
        assert_eq!(line.last(), Some(&0), "not NUL-terminated");
        // SAFETY: `line` is NUL-terminated; the array Windows gives back is
        // read for its `count` entries, each NUL-terminated, then freed.
        unsafe {
            let mut count = 0i32;
            let argv = CommandLineToArgvW(line.as_ptr(), &raw mut count);
            assert!(!argv.is_null());
            let args = (0..usize::try_from(count).unwrap())
                .map(|i| {
                    let arg = *argv.add(i);
                    let len = (0..).take_while(|&n| *arg.add(n) != 0).count();
                    OsString::from_wide(std::slice::from_raw_parts(arg, len))
                })
                .collect();
            LocalFree(argv.cast());
            args
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
