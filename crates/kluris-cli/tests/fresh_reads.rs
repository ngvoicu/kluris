//! Fresh reads: before reading, `wake-up` and `search` fetch the brain and
//! fast-forward its branch, at most once a minute, within 3 s, never failing
//! the read and never changing anything but a clean fast-forward (SPEC Phase
//! 7, TEST-K3-49). Real git: a bare remote and two clones in a temp dir.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{fs, thread};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// v2's `wake-up --json` keys, in order.
const WAKE_UP_KEYS: [&str; 13] = [
    "ok",
    "name",
    "path",
    "description",
    "brain_md",
    "lobes",
    "total_neurons",
    "total_yaml_neurons",
    "recent",
    "glossary",
    "deprecation_count",
    "deprecation",
    "parse_errors",
];

/// One world at a time: each runs several git processes per step, and the
/// Windows runner caps a run at 64 processes.
static SERIAL: Mutex<()> = Mutex::new(());

/// A bare `remote.git`, the teammate's clone and mine (registered as the
/// brain `team`), with a hermetic git config and HOME.
struct World {
    tmp: TempDir,
    _serial: MutexGuard<'static, ()>,
}

impl World {
    fn new() -> World {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        let world = World {
            tmp,
            _serial: serial,
        };
        fs::create_dir_all(world.home()).unwrap();
        fs::write(world.gitconfig(), "").unwrap();
        world.git(
            world.root(),
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
        );
        world.git(world.root(), &["clone", "-q", "remote.git", "teammate"]);
        let t = world.teammate();
        world.write(&t, "brain.md", "# team\n\n## Lobes\n\n- [k](k/map.md)\n");
        world.write(
            &t,
            "glossary.md",
            "# Glossary\n\n**JWT** - JSON Web Token\n",
        );
        world.write(&t, "k/map.md", "# k\n");
        world.write(
            &t,
            "k/jwt.md",
            "---\nparent: map.md\nupdated: 2026-02-03\n---\n# JWT tokens\n\nWe sign every JWT.\n",
        );
        world.write(&t, ".gitignore", "kluris.yml\n");
        world.commit(&t, "seed");
        world.git(&t, &["push", "-q", "-u", "origin", "main"]);
        world.git(world.root(), &["clone", "-q", "remote.git", "mine"]);
        world.write(&world.mine(), "kluris.yml", "name: team\n");
        fs::create_dir_all(world.home().join(".kluris")).unwrap();
        fs::write(
            world.home().join(".kluris").join("config.yml"),
            format!(
                "brains:\n  team:\n    path: '{}'\n    description: ''\n",
                world.mine().display().to_string().replace('\'', "''")
            ),
        )
        .unwrap();
        world
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    fn gitconfig(&self) -> PathBuf {
        self.root().join("gitconfig")
    }

    fn teammate(&self) -> PathBuf {
        self.root().join("teammate")
    }

    fn mine(&self) -> PathBuf {
        self.root().join("mine")
    }

    fn stamp(&self) -> PathBuf {
        self.mine()
            .join(".git")
            .join("kluris-sync")
            .join("last-fetch")
    }

    fn hermetic(&self, cmd: &mut std::process::Command) {
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.gitconfig())
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .env_remove("GIT_SSH_COMMAND")
            .env_remove("GIT_SSH")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("GCM_INTERACTIVE");
    }

    /// Run the test's own git; it must succeed.
    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args).current_dir(dir);
        self.hermetic(&mut cmd);
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn write(&self, clone: &Path, rel: &str, text: &str) {
        let path = clone.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn commit(&self, clone: &Path, message: &str) {
        self.git(clone, &["add", "-A"]);
        self.git(clone, &["commit", "-q", "-m", message]);
    }

    /// The teammate writes `k/<name>.md` about `word`, commits and pushes.
    fn teammate_pushes(&self, name: &str, word: &str) {
        let t = self.teammate();
        self.write(
            &t,
            &format!("k/{name}.md"),
            &format!(
                "---\nparent: map.md\nupdated: 2026-03-01\n---\n# {name}\n\nAll about {word}.\n"
            ),
        );
        self.commit(&t, &format!("add {name}"));
        self.git(&t, &["push", "-q"]);
    }

    fn head(&self) -> String {
        self.git(&self.mine(), &["rev-parse", "HEAD"])
    }

    fn remote_head(&self) -> String {
        self.git(&self.root().join("remote.git"), &["rev-parse", "main"])
    }

    fn kluris(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
        let mut std_cmd = std::process::Command::new("git");
        self.hermetic(&mut std_cmd);
        for (key, value) in std_cmd.get_envs() {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
        cmd.env(
            "KLURIS_CONFIG",
            self.home().join(".kluris").join("config.yml"),
        )
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1")
        .env_remove("KLURIS_NO_AUTO_SYNC")
        .current_dir(self.root());
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        let out = self.kluris().args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.stderr.is_empty(),
            "nothing on stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    /// The same command with auto-sync off: the local brain as it is.
    fn baseline(&self, args: &[&str]) -> Output {
        let out = self
            .kluris()
            .args(args)
            .env("KLURIS_NO_AUTO_SYNC", "1")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        out
    }

    fn wake_up(&self) -> Value {
        let out = self.run(&["wake-up", "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        let keys: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, WAKE_UP_KEYS, "no key added");
        doc
    }

    fn search_files(&self, query: &str) -> Vec<String> {
        let out = self.run(&["search", query, "--json"]);
        let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
        doc["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["file"].as_str().unwrap().to_string())
            .collect()
    }

    fn set_stamp_age(&self, age: i64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        fs::create_dir_all(self.stamp().parent().unwrap()).unwrap();
        fs::write(self.stamp(), format!("{}\n", now - age)).unwrap();
    }

    /// HEAD, the tracked status and every file of my clone outside `.git`.
    fn snapshot(&self) -> (String, String, Vec<(PathBuf, Vec<u8>)>) {
        let mut files = Vec::new();
        let mut stack = vec![self.mine()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = fs::read(&path).unwrap();
                    files.push((path, bytes));
                }
            }
        }
        files.sort();
        let status = self.git(&self.mine(), &["status", "--porcelain"]);
        (self.head(), status, files)
    }

    /// Every command's output equals the auto-sync-off run, and my clone is
    /// exactly as it was.
    fn assert_untouched(&self) {
        let before = self.snapshot();
        for args in [
            &["wake-up", "--json"][..],
            &["wake-up"],
            &["search", "jwt", "--json"],
            &["search", "jwt"],
        ] {
            let _ = fs::remove_file(self.stamp());
            let out = self.run(args);
            let base = self.baseline(args);
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&base.stdout),
                "{args:?}"
            );
        }
        assert_eq!(self.snapshot(), before);
    }
}

/// A local server that accepts connections and never answers.
fn silent_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    port
}

/// A local HTTP server that answers every request with 401 and a Basic
/// challenge, as a host asking for credentials does.
fn auth_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"kluris\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    port
}

#[test]
fn a_teammates_push_becomes_visible_to_wake_up_and_search() {
    let w = World::new();
    assert_eq!(w.wake_up()["total_neurons"], 1);
    assert!(w.stamp().is_file(), "the fetch is stamped");

    w.teammate_pushes("zebra", "zebras");
    fs::remove_file(w.stamp()).unwrap();
    let doc = w.wake_up();
    assert_eq!(doc["total_neurons"], 2);
    assert_eq!(w.head(), w.remote_head());

    w.teammate_pushes("okapi", "okapis");
    fs::remove_file(w.stamp()).unwrap();
    assert_eq!(w.search_files("okapis"), ["k/okapi.md"]);
    assert_eq!(w.head(), w.remote_head());
}

#[test]
fn a_fast_forward_prints_nothing_more() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let out = w.run(&["wake-up"]);
    let after = w.baseline(&["wake-up"]);
    assert_eq!(
        out.stdout, after.stdout,
        "text output as for the local brain"
    );
    w.teammate_pushes("okapi", "okapis");
    fs::remove_file(w.stamp()).unwrap();
    let out = w.run(&["search", "okapis"]);
    let after = w.baseline(&["search", "okapis"]);
    assert_eq!(out.stdout, after.stdout);
}

#[test]
fn the_brain_is_fetched_at_most_once_a_minute() {
    let w = World::new();
    w.wake_up();
    w.teammate_pushes("zebra", "zebras");
    assert_eq!(w.wake_up()["total_neurons"], 1, "fetched seconds ago");
    assert!(w.search_files("zebras").is_empty());
    // Well inside the minute: kluris's clock runs a little after the test's.
    w.set_stamp_age(50);
    assert_eq!(w.wake_up()["total_neurons"], 1);
    w.set_stamp_age(61);
    assert_eq!(w.wake_up()["total_neurons"], 2);
    w.teammate_pushes("okapi", "okapis");
    w.set_stamp_age(-3600);
    assert_eq!(
        w.search_files("okapis"),
        ["k/okapi.md"],
        "a stamp from the future does not throttle"
    );
}

#[test]
fn a_remote_that_hangs_is_given_up_after_3_seconds() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let port = silent_server();
    w.git(
        &w.mine(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("git://127.0.0.1:{port}/remote.git"),
        ],
    );
    let base = w.baseline(&["wake-up", "--json"]);
    let started = Instant::now();
    let out = w.run(&["wake-up", "--json"]);
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    assert!(took < Duration::from_secs(6), "{took:?}");
    assert_eq!(out.stdout, base.stdout, "the local brain as it is");
    // Stamped before the fetch: the next read does not wait again.
    let started = Instant::now();
    w.run(&["search", "jwt", "--json"]);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn a_timed_out_fetch_leaves_no_process_behind() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let pids = w.root().join("pids");
    let script = w.root().join("hang.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ >> '{0}'\nsleep 30 &\necho $! >> '{0}'\nwait\n",
            pids.display()
        ),
    )
    .unwrap();
    w.git(
        &w.mine(),
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", script.display()),
        ],
    );
    let started = Instant::now();
    w.run(&["wake-up", "--json"]);
    assert!(started.elapsed() < Duration::from_secs(6));
    let text = fs::read_to_string(&pids).unwrap();
    let pids: Vec<&str> = text.lines().collect();
    assert_eq!(pids.len(), 2, "{text}");
    let alive = |pid: &str| {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while pids.iter().any(|p| alive(p)) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    for pid in &pids {
        if alive(pid) {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid])
                .status();
            panic!("process {pid} outlived kluris");
        }
    }
}

#[test]
fn offline_the_read_goes_on_with_the_local_brain() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    // A port nothing listens on.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    w.git(
        &w.mine(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("git://127.0.0.1:{port}/remote.git"),
        ],
    );
    w.assert_untouched();
    // A remote path that is gone.
    let gone = w.root().join("gone.git").display().to_string();
    w.git(&w.mine(), &["remote", "set-url", "origin", &gone]);
    w.assert_untouched();
}

#[test]
fn without_a_remote_or_an_upstream_the_read_goes_on() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    w.git(&w.mine(), &["branch", "--unset-upstream"]);
    w.assert_untouched();
    w.git(&w.mine(), &["remote", "remove", "origin"]);
    w.assert_untouched();
}

#[test]
fn a_detached_head_is_left_alone() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    w.git(&w.mine(), &["checkout", "-q", "--detach"]);
    w.assert_untouched();
}

#[test]
fn a_dirty_tree_is_left_untouched() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    // An unstaged change to a tracked file.
    w.write(
        &w.mine(),
        "k/jwt.md",
        "---\nupdated: 2026-04-01\n---\n# JWT mine\n",
    );
    w.assert_untouched();
    // The same change staged.
    w.git(&w.mine(), &["add", "k/jwt.md"]);
    w.assert_untouched();
}

#[test]
fn an_untracked_file_does_not_block_and_is_kept() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    w.write(&w.mine(), "k/draft.md", "# my draft about jwt\n");
    w.wake_up();
    assert_eq!(w.head(), w.remote_head());
    assert_eq!(
        fs::read_to_string(w.mine().join("k/draft.md")).unwrap(),
        "# my draft about jwt\n"
    );
}

#[test]
fn a_merge_in_progress_is_left_untouched() {
    let w = World::new();
    let m = w.mine();
    w.git(&m, &["checkout", "-q", "-b", "side"]);
    w.write(&m, "k/side.md", "# side\n");
    w.commit(&m, "side");
    w.git(&m, &["checkout", "-q", "main"]);
    // A merge stopped before its commit, with a clean tree (`-s ours`).
    w.git(
        &m,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--no-commit",
            "-s",
            "ours",
            "side",
        ],
    );
    assert!(m.join(".git").join("MERGE_HEAD").is_file());
    w.teammate_pushes("zebra", "zebras");
    w.assert_untouched();
    assert!(m.join(".git").join("MERGE_HEAD").is_file());
}

#[test]
fn an_open_kluris_conflict_is_left_untouched() {
    let w = World::new();
    let m = w.mine();
    // `kluris sync` left a conflict for `kluris resolve`.
    let pending = m.join(".git").join("kluris-sync").join("pending.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    fs::write(&pending, "{}").unwrap();
    w.teammate_pushes("zebra", "zebras");
    w.assert_untouched();
    assert!(pending.is_file());
}

#[test]
fn a_conflicted_merge_is_left_untouched() {
    let w = World::new();
    let m = w.mine();
    w.write(
        &m,
        "k/jwt.md",
        "---\nupdated: 2026-04-01\n---\n# JWT mine\n",
    );
    w.commit(&m, "mine");
    w.write(
        &w.teammate(),
        "k/jwt.md",
        "---\nupdated: 2026-04-02\n---\n# JWT theirs\n",
    );
    w.commit(&w.teammate(), "theirs");
    w.git(&w.teammate(), &["push", "-q"]);
    w.git(&m, &["fetch", "-q"]);
    let mut cmd = std::process::Command::new("git");
    cmd.args(["merge", "-q", "origin/main"]).current_dir(&m);
    w.hermetic(&mut cmd);
    assert!(!cmd.output().unwrap().status.success(), "a conflict");
    w.teammate_pushes("zebra", "zebras");
    let before = w.snapshot();
    for args in [&["wake-up", "--json"][..], &["search", "jwt", "--json"]] {
        let _ = fs::remove_file(w.stamp());
        let out = w.run(args);
        assert_eq!(out.stdout, w.baseline(args).stdout, "{args:?}");
    }
    assert_eq!(w.snapshot(), before);
    assert!(m.join(".git").join("MERGE_HEAD").is_file());
}

#[test]
fn a_rebase_in_progress_is_left_untouched() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    // git's own marker of a stopped rebase (as `git rebase -i` leaves it).
    fs::create_dir_all(w.mine().join(".git").join("rebase-merge")).unwrap();
    w.assert_untouched();
}

#[test]
fn a_diverged_branch_is_left_untouched() {
    let w = World::new();
    w.write(&w.mine(), "k/mine.md", "# mine\n");
    w.commit(&w.mine(), "mine");
    w.teammate_pushes("zebra", "zebras");
    w.assert_untouched();
    let merge_base = w.git(&w.mine(), &["merge-base", "HEAD", "origin/main"]);
    assert_ne!(w.head(), merge_base, "still my own commit on top");
}

#[test]
fn turned_off_by_the_variable_and_by_kluris_yml() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let before = w.head();
    let out = w
        .kluris()
        .args(["wake-up", "--json"])
        .env("KLURIS_NO_AUTO_SYNC", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(w.head(), before);
    assert!(!w.stamp().exists());

    w.write(
        &w.mine(),
        "kluris.yml",
        "name: team\nsync:\n  auto_fetch: false\n",
    );
    assert_eq!(w.wake_up()["total_neurons"], 1);
    assert!(w.search_files("zebras").is_empty());
    assert_eq!(w.head(), before);
    assert!(!w.stamp().exists());

    // Any other value of the variable leaves it on.
    w.write(&w.mine(), "kluris.yml", "name: team\n");
    let out = w
        .kluris()
        .args(["wake-up", "--json"])
        .env("KLURIS_NO_AUTO_SYNC", "0")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(w.head(), w.remote_head());
}

#[test]
fn a_credential_prompt_never_blocks() {
    let w = World::new();
    let port = auth_server();
    w.git(
        &w.mine(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("http://127.0.0.1:{port}/remote.git"),
        ],
    );
    let base = w.baseline(&["wake-up", "--json"]);
    let started = Instant::now();
    let out = w.run(&["wake-up", "--json"]);
    let took = started.elapsed();
    assert_eq!(out.stdout, base.stdout);
    assert!(
        took < Duration::from_millis(2500),
        "refused at once, not cut off at 3 s: {took:?}"
    );
}

#[cfg(unix)]
#[test]
fn an_askpass_helper_is_never_asked() {
    // Editors set GIT_ASKPASS to a helper that shows a dialog; one that
    // hangs here must not be run.
    let w = World::new();
    let port = auth_server();
    w.git(
        &w.mine(),
        &[
            "remote",
            "set-url",
            "origin",
            &format!("http://127.0.0.1:{port}/remote.git"),
        ],
    );
    let asked = w.root().join("asked");
    let askpass = w.root().join("askpass.sh");
    fs::write(
        &askpass,
        format!("#!/bin/sh\ntouch '{}'\nsleep 30\n", asked.display()),
    )
    .unwrap();
    make_executable(&askpass);
    let started = Instant::now();
    let out = w
        .kluris()
        .args(["wake-up", "--json"])
        .env("GIT_ASKPASS", &askpass)
        .env("SSH_ASKPASS", &askpass)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert!(!asked.exists(), "the askpass helper ran");
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn ssh_runs_in_batch_mode_unless_the_user_chose_an_ssh_command() {
    let w = World::new();
    let bin = w.root().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = w.root().join("ssh.log");
    let ssh = bin.join("ssh");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nexit 255\n",
            log.display()
        ),
    )
    .unwrap();
    make_executable(&ssh);
    w.git(
        &w.mine(),
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.invalid/remote.git",
        ],
    );
    let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let run = |extra: &[(&str, String)]| {
        let _ = fs::remove_file(&log);
        let _ = fs::remove_file(w.stamp());
        let mut cmd = w.kluris();
        cmd.args(["wake-up", "--json"]).env("PATH", &path);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        fs::read_to_string(&log).unwrap_or_default()
    };
    let logged = run(&[]);
    assert!(logged.contains("BatchMode=yes"), "{logged}");
    // The user's own GIT_SSH_COMMAND is used as it is.
    let logged = run(&[("GIT_SSH_COMMAND", format!("'{}' -o User=me", ssh.display()))]);
    assert!(logged.contains("User=me"), "{logged}");
    assert!(!logged.contains("BatchMode"), "{logged}");
    // So is core.sshCommand.
    w.git(
        &w.mine(),
        &[
            "config",
            "core.sshCommand",
            &format!("'{}' -o User=cfg", ssh.display()),
        ],
    );
    let logged = run(&[]);
    assert!(logged.contains("User=cfg"), "{logged}");
    assert!(!logged.contains("BatchMode"), "{logged}");
}

#[test]
fn a_brain_nested_in_a_larger_repo_is_never_moved() {
    // A brain inside a project repo: a read never moves a repository the
    // user did not ask Kluris to manage, however clean it is.
    let w = World::new();
    let root = w.root();
    w.git(root, &["init", "-q", "--bare", "-b", "main", "outer.git"]);
    w.git(root, &["clone", "-q", "outer.git", "project"]);
    let theirs = root.join("project");
    w.write(&theirs, "README.md", "# project\n");
    w.write(&theirs, ".gitignore", "kluris.yml\n");
    w.write(
        &theirs,
        "kb/brain.md",
        "# team\n\n## Lobes\n\n- [k](k/map.md)\n",
    );
    w.write(&theirs, "kb/k/map.md", "# k\n");
    w.write(
        &theirs,
        "kb/k/jwt.md",
        "---\nparent: map.md\nupdated: 2026-02-03\n---\n# JWT tokens\n\nWe sign every JWT.\n",
    );
    w.commit(&theirs, "seed");
    w.git(&theirs, &["push", "-q", "-u", "origin", "main"]);
    w.git(root, &["clone", "-q", "outer.git", "project-mine"]);
    let mine = root.join("project-mine");
    let brain = mine.join("kb");
    w.write(&brain, "kluris.yml", "name: team\n");
    fs::write(
        w.home().join(".kluris").join("config.yml"),
        format!(
            "brains:\n  team:\n    path: '{}'\n    description: ''\n",
            brain.display().to_string().replace('\'', "''")
        ),
    )
    .unwrap();
    // The outer repo's upstream moves, in the brain and outside it.
    w.write(&theirs, "README.md", "# project, changed\n");
    w.write(
        &theirs,
        "kb/k/zebra.md",
        "---\nparent: map.md\nupdated: 2026-03-01\n---\n# zebra\n\nAll about zebras.\n",
    );
    w.commit(&theirs, "move");
    w.git(&theirs, &["push", "-q"]);

    let state = || {
        (
            w.git(&mine, &["rev-parse", "HEAD"]),
            w.git(&mine, &["rev-parse", "refs/remotes/origin/main"]),
            w.git(&mine, &["status", "--porcelain", "--untracked-files=all"]),
            fs::read(mine.join("README.md")).unwrap(),
            brain.join("k").join("zebra.md").exists(),
        )
    };
    let before = state();
    assert!(!before.4);
    for args in [
        &["wake-up", "--json"][..],
        &["wake-up"],
        &["search", "zebras", "--json"],
        &["search", "jwt"],
    ] {
        let out = w.run(args);
        assert_eq!(out.stdout, w.baseline(args).stdout, "{args:?}");
    }
    assert_eq!(state(), before, "nothing fetched, nothing moved");
    assert!(
        !mine.join(".git").join("kluris-sync").exists(),
        "no stamp in the outer repo"
    );
}

impl World {
    /// My clone's view of the remote branch (moved by a fetch).
    fn tracked_remote(&self) -> String {
        self.git(&self.mine(), &["rev-parse", "refs/remotes/origin/main"])
    }

    /// Fetched, not moved, nothing half done: HEAD where it was, the remote
    /// branch up to date, no `index.lock`, no change of any kind in the tree.
    fn assert_fetched_only(&self, head_before: &str) {
        assert_eq!(self.head(), head_before, "HEAD moved");
        assert_eq!(self.tracked_remote(), self.remote_head(), "not fetched");
        assert!(
            !self.mine().join(".git").join("index.lock").exists(),
            "a stale index.lock"
        );
        assert_eq!(
            self.git(
                &self.mine(),
                &["status", "--porcelain", "--untracked-files=all"]
            ),
            "",
            "the tree changed"
        );
    }
}

#[test]
fn a_slow_checkout_filter_never_leaves_a_half_done_fast_forward() {
    // The review's repro (T-145 probes/slow_checkout): a smudge filter that
    // takes 6 s on an incoming file. The read must not stop git halfway.
    let w = World::new();
    w.git(&w.mine(), &["config", "filter.slow.smudge", "sleep 6; cat"]);
    w.git(&w.mine(), &["config", "filter.slow.clean", "cat"]);
    let t = w.teammate();
    w.write(&t, ".gitattributes", "k/z.md filter=slow\n");
    w.write(
        &t,
        "k/jwt.md",
        "---\nparent: map.md\nupdated: 2026-03-02\n---\n# REMOTE NEW\n",
    );
    w.write(&t, "k/z.md", "# slow checkout\n");
    w.commit(&t, "new remote files");
    w.git(&t, &["push", "-q"]);
    let before = w.head();
    let jwt = fs::read(w.mine().join("k/jwt.md")).unwrap();
    let base = w.baseline(&["wake-up", "--json"]);
    let started = Instant::now();
    let out = w.run(&["wake-up", "--json"]);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(out.stdout, base.stdout);
    w.assert_fetched_only(&before);
    assert_eq!(fs::read(w.mine().join("k/jwt.md")).unwrap(), jwt);
    assert!(!w.mine().join("k/z.md").exists());
}

/// Git LFS's driver as `git lfs install` configures it.
fn configure_lfs_driver(w: &World) {
    for (key, value) in [
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.required", "true"),
    ] {
        w.git(&w.mine(), &["config", key, value]);
    }
}

/// The teammate commits a `.gitattributes` with `line` and pushes.
fn teammate_pushes_attributes(w: &World, line: &str) {
    let t = w.teammate();
    w.write(&t, ".gitattributes", line);
    w.commit(&t, "attributes");
    w.git(&t, &["push", "-q"]);
}

#[test]
fn a_configured_driver_no_attribute_names_does_not_block() {
    // Git for Windows' system config and a global `git lfs install` define
    // `filter.lfs.*` for every repository; with nothing assigning it, no
    // filter runs, and the fast-forward goes ahead.
    let w = World::new();
    configure_lfs_driver(&w);
    w.teammate_pushes("zebra", "zebras");
    assert_eq!(w.wake_up()["total_neurons"], 2);
    assert_eq!(w.head(), w.remote_head());
}

#[test]
fn an_attribute_naming_a_configured_driver_leaves_only_the_fetch() {
    let w = World::new();
    configure_lfs_driver(&w);
    teammate_pushes_attributes(&w, "*.bin filter=lfs diff=lfs merge=lfs -text\n");
    w.teammate_pushes("zebra", "zebras");
    let before = w.head();
    assert!(w.search_files("zebras").is_empty());
    w.assert_fetched_only(&before);
}

#[test]
fn an_attribute_without_a_driver_does_not_block() {
    // `filter=lfs` with no `filter.lfs.*` anywhere runs nothing.
    let w = World::new();
    teammate_pushes_attributes(&w, "*.bin filter=lfs diff=lfs merge=lfs -text\n");
    w.teammate_pushes("zebra", "zebras");
    assert_eq!(w.search_files("zebras"), ["k/zebra.md"]);
    assert_eq!(w.head(), w.remote_head());
}

#[test]
fn an_attribute_in_info_attributes_counts_when_its_driver_is_configured() {
    let w = World::new();
    let info = w.mine().join(".git").join("info").join("attributes");
    fs::write(&info, "k/*.md filter=slow\n").unwrap();
    w.teammate_pushes("zebra", "zebras");
    let before = w.head();
    // No driver: nothing would run, so it moves.
    w.wake_up();
    assert_eq!(w.head(), w.remote_head());
    // With the driver configured it would run: only the fetch.
    w.git(&w.mine(), &["config", "filter.slow.smudge", "cat"]);
    w.teammate_pushes("okapi", "okapis");
    fs::remove_file(w.stamp()).unwrap();
    let moved = w.head();
    assert_ne!(moved, before);
    w.wake_up();
    w.assert_fetched_only(&moved);
}

#[test]
fn an_attribute_in_the_core_attributes_file_counts_when_its_driver_is_configured() {
    let w = World::new();
    let file = w.root().join("my-attributes");
    fs::write(&file, "*.md filter=slow\n").unwrap();
    w.git(
        &w.mine(),
        &[
            "config",
            "core.attributesFile",
            &file.display().to_string().replace('\\', "/"),
        ],
    );
    w.git(&w.mine(), &["config", "filter.slow.clean", "cat"]);
    w.teammate_pushes("zebra", "zebras");
    let before = w.head();
    w.wake_up();
    w.assert_fetched_only(&before);
}

/// The teammate pushes `n` new neurons in one commit.
fn teammate_pushes_many(w: &World, n: usize) {
    let t = w.teammate();
    for i in 0..n {
        w.write(&t, &format!("k/many/n{i}.md"), &format!("# n{i}\n"));
    }
    w.commit(&t, "many");
    w.git(&t, &["push", "-q"]);
}

#[test]
fn up_to_200_incoming_files_are_fast_forwarded() {
    let w = World::new();
    teammate_pushes_many(&w, 200);
    w.wake_up();
    assert_eq!(w.head(), w.remote_head());
}

#[test]
fn more_than_200_incoming_files_leave_only_the_fetch() {
    let w = World::new();
    teammate_pushes_many(&w, 201);
    let before = w.head();
    w.wake_up();
    w.assert_fetched_only(&before);
}

#[test]
fn an_ignored_file_where_the_incoming_tree_writes_is_kept() {
    // The review's repro (T-145 probes/fresh_ignored): my ignored draft at a
    // path a teammate starts tracking is never replaced.
    let w = World::new();
    let m = w.mine();
    fs::write(m.join(".git").join("info").join("exclude"), "private/\n").unwrap();
    w.write(&m, "private/draft.md", "MY DRAFT\n");
    let t = w.teammate();
    w.write(&t, "private/draft.md", "TEAM_VERSION\n");
    w.write(&t, "k/other.md", "# other\n");
    w.commit(&t, "team draft");
    w.git(&t, &["push", "-q"]);
    let before = w.head();
    w.wake_up();
    assert_eq!(
        fs::read_to_string(m.join("private/draft.md")).unwrap(),
        "MY DRAFT\n"
    );
    assert_eq!(w.head(), before, "HEAD moved");
    assert_eq!(w.tracked_remote(), w.remote_head(), "not fetched");
    assert!(!m.join("k/other.md").exists());
    assert!(!m.join(".git").join("index.lock").exists());
}

#[test]
fn an_ignored_file_where_the_incoming_tree_needs_a_folder_is_kept() {
    let w = World::new();
    let m = w.mine();
    fs::write(m.join(".git").join("info").join("exclude"), "/notes\n").unwrap();
    w.write(&m, "notes", "MY NOTES FILE\n");
    let t = w.teammate();
    w.write(&t, "notes/a.md", "# a\n");
    w.commit(&t, "notes folder");
    w.git(&t, &["push", "-q"]);
    let before = w.head();
    w.wake_up();
    assert_eq!(
        fs::read_to_string(m.join("notes")).unwrap(),
        "MY NOTES FILE\n"
    );
    assert_eq!(w.head(), before);
}

#[test]
fn a_started_fast_forward_runs_to_its_end() {
    // A post-merge hook keeps git busy past the 3 s: the fast-forward is
    // never killed once it has started.
    let w = World::new();
    let m = w.mine();
    let done = w.root().join("hook-done");
    let hook = m.join(".git").join("hooks").join("post-merge");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nsleep 4\necho done > '{}'\n",
            done.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    #[cfg(unix)]
    make_executable(&hook);
    w.teammate_pushes("zebra", "zebras");
    let started = Instant::now();
    let doc = w.wake_up();
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert_eq!(doc["total_neurons"], 2);
    assert_eq!(w.head(), w.remote_head());
    assert!(done.exists(), "the merge was killed before its hook ended");
    assert!(!m.join(".git").join("index.lock").exists());
}

#[test]
fn the_machines_own_system_git_config_does_not_block_the_fast_forward() {
    // Git for Windows' system gitconfig defines `filter.lfs.*`; with no
    // attribute assigning it, a read on that machine must still move.
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let out = w
        .kluris()
        .args(["wake-up", "--json"])
        .env_remove("GIT_CONFIG_NOSYSTEM")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["total_neurons"], 2);
    assert_eq!(w.head(), w.remote_head());
}

/// HEAD, the index's bytes and every file of my clone outside `.git` (with
/// untracked and ignored files), read without running anything that could
/// refresh the index.
type RawSnapshot = (String, Vec<u8>, Vec<(PathBuf, Vec<u8>)>);

impl World {
    fn raw_snapshot(&self) -> RawSnapshot {
        let mut files = Vec::new();
        let mut stack = vec![self.mine()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
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
        let index = fs::read(self.mine().join(".git").join("index")).unwrap();
        (self.head(), index, files)
    }

    /// A failing, required smudge filter in my clone's config only.
    fn configure_failing_required_filter(&self) {
        let m = self.mine();
        self.git(&m, &["config", "filter.required.smudge", "exit 1"]);
        self.git(&m, &["config", "filter.required.clean", "cat"]);
        self.git(&m, &["config", "filter.required.required", "true"]);
    }

    /// The teammate changes `k/jwt.md` (checked out before `k/z.md`) and adds
    /// `k/z.md`, and pushes.
    fn teammate_pushes_a_change_and_z(&self) {
        let t = self.teammate();
        self.write(
            &t,
            "k/jwt.md",
            "---\nparent: map.md\nupdated: 2026-03-02\n---\n# INCOMING JWT\n",
        );
        self.write(&t, "k/z.md", "# FILTERED Z\n");
        self.commit(&t, "change and z");
        self.git(&t, &["push", "-q"]);
    }

    /// A read changes nothing at all: the raw snapshot is the same after it
    /// (taken before anything else runs git), and the fetch happened.
    fn assert_read_changes_nothing(&self) {
        let before = self.raw_snapshot();
        let base = self.baseline(&["wake-up", "--json"]);
        let out = self.run(&["wake-up", "--json"]);
        let after = self.raw_snapshot();
        assert_eq!(after.0, before.0, "HEAD moved");
        assert_eq!(after.1, before.1, "the index changed");
        assert_eq!(after.2, before.2, "the work tree changed");
        assert_eq!(out.stdout, base.stdout);
        assert_eq!(self.tracked_remote(), self.remote_head(), "not fetched");
    }
}

#[test]
fn an_untracked_gitattributes_naming_a_failing_required_filter_changes_nothing() {
    // T-150 F150-4 (probes/r_filter_required_failure): the attribute is in
    // the work tree only, where git's checkout reads it.
    let w = World::new();
    w.configure_failing_required_filter();
    w.write(&w.mine(), ".gitattributes", "k/z.md filter=required\n");
    w.teammate_pushes_a_change_and_z();
    w.assert_read_changes_nothing();
}

#[test]
fn an_ignored_gitattributes_naming_a_failing_required_filter_changes_nothing() {
    let w = World::new();
    w.configure_failing_required_filter();
    fs::write(
        w.mine().join(".git").join("info").join("exclude"),
        ".gitattributes\n",
    )
    .unwrap();
    w.write(&w.mine(), ".gitattributes", "k/z.md filter=required\n");
    w.teammate_pushes_a_change_and_z();
    w.assert_read_changes_nothing();
}

#[test]
fn a_gitattributes_in_a_subfolder_naming_a_failing_required_filter_changes_nothing() {
    let w = World::new();
    w.configure_failing_required_filter();
    w.write(&w.mine(), "k/.gitattributes", "z.md filter=required\n");
    w.teammate_pushes_a_change_and_z();
    w.assert_read_changes_nothing();
}

#[test]
fn a_failing_post_merge_hook_leaves_a_complete_fast_forward() {
    // git runs post-merge after HEAD, the index and the tree are updated,
    // and ignores its status: the fast-forward is whole.
    let w = World::new();
    let hook = w.mine().join(".git").join("hooks").join("post-merge");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    make_executable(&hook);
    w.teammate_pushes("zebra", "zebras");
    assert_eq!(w.wake_up()["total_neurons"], 2);
    assert_eq!(w.head(), w.remote_head());
    assert_eq!(
        w.git(
            &w.mine(),
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
}

#[cfg(unix)]
#[test]
fn a_folder_git_cannot_write_into_changes_nothing() {
    // git cannot replace k/jwt.md in a read-only k/, so a fast-forward
    // would stop halfway (the analogue of a locked file).
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    let t = w.teammate();
    w.write(&t, "a.md", "# a, checked out first\n");
    w.write(
        &t,
        "k/jwt.md",
        "---\nparent: map.md\nupdated: 2026-03-02\n---\n# INCOMING JWT\n",
    );
    w.commit(&t, "a and jwt");
    w.git(&t, &["push", "-q"]);
    let k = w.mine().join("k");
    fs::set_permissions(&k, fs::Permissions::from_mode(0o555)).unwrap();
    let before = w.raw_snapshot();
    let out = w.kluris().args(["wake-up", "--json"]).output().unwrap();
    let after = w.raw_snapshot();
    fs::set_permissions(&k, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(after, before);
    assert_eq!(w.tracked_remote(), w.remote_head(), "not fetched");
}

#[cfg(windows)]
#[test]
fn a_path_too_long_for_windows_changes_nothing() {
    // Without core.longpaths, git for Windows cannot check out a path of
    // 260 characters or more and stops halfway.
    let w = World::new();
    let t = w.teammate();
    let mine_len = w.mine().display().to_string().encode_utf16().count();
    let rel = format!("k/{}.md", "l".repeat(260usize.saturating_sub(mine_len)));
    w.write(&t, "a.md", "# a, checked out first\n");
    w.write(&t, &rel, "# long\n");
    w.git(&t, &["-c", "core.longpaths=true", "add", "-A"]);
    w.git(
        &t,
        &["-c", "core.longpaths=true", "commit", "-q", "-m", "long"],
    );
    w.git(&t, &["-c", "core.longpaths=true", "push", "-q"]);
    w.assert_read_changes_nothing();
}

#[cfg(unix)]
#[test]
fn a_failed_fast_forward_is_reported_on_stderr_and_stdout_is_unchanged() {
    // A `git` first on PATH that passes everything to the real git but
    // fails the fast-forward (`merge --ff-only`).
    let w = World::new();
    let real = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
    assert!(!real.is_empty());
    let bin = w.root().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let git = bin.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = --ff-only ]; then echo 'error: simulated' >&2; exit 1; fi\ndone\nexec '{real}' \"$@\"\n"
        ),
    )
    .unwrap();
    make_executable(&git);
    let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    w.teammate_pushes("zebra", "zebras");
    for args in [
        &["wake-up", "--json"][..],
        &["wake-up"],
        &["search", "jwt", "--json"],
        &["search", "jwt"],
    ] {
        let _ = fs::remove_file(w.stamp());
        let base = w.baseline(args);
        let out = w.kluris().args(args).env("PATH", &path).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        assert_eq!(out.stdout, base.stdout, "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "kluris: team was only partly updated from its remote; run `kluris sync --brain team` to finish\n",
            "{args:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn a_file_another_program_holds_open_changes_nothing() {
    // Another program has a file the fast-forward replaces open without
    // sharing: git could not replace it and would stop partway.
    use std::os::windows::fs::OpenOptionsExt;
    let w = World::new();
    let t = w.teammate();
    w.write(&t, "data.txt", "one\n");
    w.commit(&t, "data");
    w.git(&t, &["push", "-q"]);
    w.git(&w.mine(), &["pull", "-q", "--ff-only"]);
    // The index must know data.txt's stat data, so git's status does not
    // need to read it (a just-written file is "racily clean").
    thread::sleep(Duration::from_secs(2));
    w.git(&w.mine(), &["update-index", "-q", "--really-refresh"]);
    w.write(&t, "data.txt", "two\n");
    w.write(&t, "a.md", "# a, checked out first\n");
    w.commit(&t, "change data");
    w.git(&t, &["push", "-q"]);
    let before = w.raw_snapshot();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(w.mine().join("data.txt"))
        .unwrap();
    let base = w.baseline(&["wake-up", "--json"]);
    let out = w.kluris().args(["wake-up", "--json"]).output().unwrap();
    drop(held);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, base.stdout);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(w.raw_snapshot(), before, "a byte changed");
    assert_eq!(w.tracked_remote(), w.remote_head(), "not fetched");
}
