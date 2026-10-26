//! Fresh reads: before reading, `wake-up` and `search` fetch the brain and
//! fast-forward its branch, at most once a minute, within 3 s, never failing
//! the read and never changing anything but a clean fast-forward (SPEC Phase
//! 7, TEST-K3-49). Real git: a bare remote and two clones in a temp dir.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Output;
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

/// A bare `remote.git`, the teammate's clone and mine (registered as the
/// brain `team`), with a hermetic git config and HOME.
struct World {
    tmp: TempDir,
}

impl World {
    fn new() -> World {
        let tmp = TempDir::new().unwrap();
        let world = World { tmp };
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
    w.set_stamp_age(59);
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
