//! Fresh reads: before reading, `wake-up` and `search` fetch the brain's
//! upstream, at most once a minute, within 3 s, and then read the brain as
//! it is on disk: HEAD, the index and the work tree never change, and the
//! read never fails (SPEC Phase 7, TEST-K3-49). Real git: a bare remote and
//! two clones in a temp dir.

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

impl World {
    /// My clone's view of the remote branch (moved by a fetch).
    fn tracked_remote(&self) -> String {
        self.git(&self.mine(), &["rev-parse", "refs/remotes/origin/main"])
    }

    /// The teammate rewrites `k/jwt.md` and pushes.
    fn teammate_changes_jwt(&self) {
        let t = self.teammate();
        self.write(
            &t,
            "k/jwt.md",
            "---\nparent: map.md\nupdated: 2026-03-02\n---\n# JWT tokens\n\nWe sign and rotate every JWT.\n",
        );
        self.commit(&t, "rotate");
        self.git(&t, &["push", "-q"]);
    }

    /// HEAD, the raw index, `index.lock` and every path of my clone, read
    /// without running anything that could write the index.
    fn exact(&self) -> Exact {
        let mut paths = Vec::new();
        let mut stack = vec![self.mine()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).unwrap();
                let content = if meta.file_type().is_symlink() {
                    let target = fs::read_link(&path).unwrap();
                    format!("<link {}>", target.display()).into_bytes()
                } else if meta.is_dir() {
                    stack.push(path.clone());
                    b"<dir>".to_vec()
                } else {
                    fs::read(&path).unwrap()
                };
                paths.push((path, permission_bits(&meta), content));
            }
        }
        paths.sort();
        let git_dir = self.mine().join(".git");
        Exact {
            head: self.head(),
            index: sha256_hex(&fs::read(git_dir.join("index")).unwrap()),
            index_lock: fs::symlink_metadata(git_dir.join("index.lock")).is_ok(),
            paths,
        }
    }

    /// With the teammate's push waiting upstream, `wake-up` and `search`
    /// (JSON and text) each fetch it (the remote-tracking ref moves to the
    /// teammate's commit), print what they print for the brain as it is on
    /// disk, and leave HEAD, the index's bytes, `index.lock` and every path
    /// of my clone (bytes, permission bits, link targets) exactly as they were.
    fn assert_fetched_and_left_as_it_was(&self) {
        let before = self.exact();
        let behind = self.tracked_remote();
        assert_ne!(behind, self.remote_head(), "no push waits upstream");
        for args in [
            &["wake-up", "--json"][..],
            &["search", "jwt", "--json"],
            &["wake-up"],
            &["search", "jwt"],
        ] {
            self.git(
                &self.mine(),
                &["update-ref", "refs/remotes/origin/main", &behind],
            );
            let _ = fs::remove_file(self.stamp());
            let out = self.run(args);
            assert_eq!(
                self.tracked_remote(),
                self.remote_head(),
                "{args:?} did not fetch"
            );
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&self.baseline(args).stdout),
                "{args:?}"
            );
        }
        assert_eq!(self.exact(), before);
    }
}

/// What a read must leave exactly as it was.
#[derive(Debug, PartialEq, Eq)]
struct Exact {
    head: String,
    /// The SHA-256 of `.git/index`'s raw bytes.
    index: String,
    index_lock: bool,
    /// Each path outside `.git`: its permission bits, and its bytes, link
    /// target or `<dir>`.
    paths: Vec<(PathBuf, u32, Vec<u8>)>,
}

/// The whole mode on Unix; read-only or not elsewhere.
#[cfg(unix)]
fn permission_bits(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn permission_bits(meta: &fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_teammates_push_is_read_after_kluris_sync() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let head = w.head();
    assert_eq!(w.wake_up()["total_neurons"], 1, "the brain as it is");
    assert!(w.search_files("zebras").is_empty());
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
    assert_eq!(w.head(), head, "not pulled");

    let out = w
        .kluris()
        .args(["sync", "--brain", "team", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["brains"][0]["merged"], true, "{doc}");
    assert_eq!(w.head(), w.remote_head());
    assert_eq!(w.wake_up()["total_neurons"], 2);
    assert_eq!(w.search_files("zebras"), ["k/zebra.md"]);
}

#[test]
fn a_clean_tree_is_fetched_and_left_as_it_was() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn a_new_untracked_file_at_an_incoming_path_is_kept() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    w.write(&w.mine(), "k/zebra.md", "# my own zebra draft\n");
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn a_new_ignored_file_at_an_incoming_path_is_kept() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let m = w.mine();
    fs::write(m.join(".git").join("info").join("exclude"), "k/zebra.md\n").unwrap();
    w.write(&m, "k/zebra.md", "# my ignored zebra draft\n");
    w.git(&m, &["check-ignore", "-q", "k/zebra.md"]);
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn an_uncommitted_edit_to_an_incoming_file_is_kept() {
    let w = World::new();
    w.teammate_changes_jwt();
    w.write(
        &w.mine(),
        "k/jwt.md",
        "---\nparent: map.md\nupdated: 2026-04-01\n---\n# JWT mine\n\nmy jwt edit\n",
    );
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn a_live_git_holding_index_lock_keeps_it_and_finishes() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let m = w.mine();
    let note = w.root().join("held.md");
    fs::write(&note, "# held\n").unwrap();
    let blob = w.git(&m, &["hash-object", "-w", &note.display().to_string()]);
    let mut cmd = std::process::Command::new("git");
    cmd.args(["update-index", "--index-info"])
        .current_dir(&m)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    w.hermetic(&mut cmd);
    let mut writer = cmd.spawn().unwrap();
    let lock = m.join(".git").join("index.lock");
    let until = Instant::now() + Duration::from_secs(10);
    while !lock.exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(lock.exists(), "the writer never took index.lock");

    w.assert_fetched_and_left_as_it_was();

    assert!(lock.exists(), "the live writer's lock is gone");
    let mut stdin = writer.stdin.take().unwrap();
    stdin
        .write_all(format!("100644 {blob}\tk/held.md\n").as_bytes())
        .unwrap();
    drop(stdin);
    let out = writer.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "the writer failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!lock.exists());
    assert_eq!(w.git(&m, &["ls-files", "k/held.md"]), "k/held.md");
}

#[test]
fn an_incoming_case_alias_of_a_tracked_file_is_kept() {
    // On a case-insensitive file system (macOS, Windows) the incoming
    // `notes/A.md` is the same file as the tracked `notes/a.md`.
    let w = World::new();
    let t = w.teammate();
    let m = w.mine();
    w.write(&t, "notes/a.md", "# a\n\nlower case\n");
    w.commit(&t, "a");
    w.git(&t, &["push", "-q"]);
    w.git(&m, &["pull", "-q", "--ff-only"]);
    // A case-insensitive work tree cannot hold both: the teammate's commit
    // is made with plumbing.
    let upper = w.root().join("upper.md");
    fs::write(&upper, "# A\n\nupper case, a different file\n").unwrap();
    let blob = w.git(&t, &["hash-object", "-w", &upper.display().to_string()]);
    w.git(
        &t,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},notes/A.md"),
        ],
    );
    w.git(&t, &["commit", "-q", "-m", "A"]);
    w.git(&t, &["push", "-q"]);
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn an_lf_file_under_core_autocrlf_is_kept() {
    let w = World::new();
    let m = w.mine();
    w.teammate_changes_jwt();
    w.git(&m, &["config", "core.autocrlf", "true"]);
    assert!(!fs::read(m.join("k/jwt.md")).unwrap().contains(&b'\r'));
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn a_skip_worktree_entry_is_kept() {
    let w = World::new();
    let m = w.mine();
    w.teammate_changes_jwt();
    w.git(&m, &["update-index", "--skip-worktree", "k/jwt.md"]);
    w.write(&m, "k/jwt.md", "# my jwt, hidden from git\n");
    w.assert_fetched_and_left_as_it_was();
}

#[test]
fn an_assume_unchanged_entry_is_kept() {
    let w = World::new();
    let m = w.mine();
    w.teammate_changes_jwt();
    w.git(&m, &["update-index", "--assume-unchanged", "k/jwt.md"]);
    w.assert_fetched_and_left_as_it_was();
}

#[cfg(unix)]
#[test]
fn an_owner_only_file_is_kept() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    w.teammate_changes_jwt();
    fs::set_permissions(w.mine().join("k/jwt.md"), fs::Permissions::from_mode(0o600)).unwrap();
    w.assert_fetched_and_left_as_it_was();
}

#[cfg(unix)]
#[test]
fn an_executable_file_under_core_filemode_false_is_kept() {
    let w = World::new();
    let m = w.mine();
    w.teammate_changes_jwt();
    w.git(&m, &["config", "core.filemode", "false"]);
    make_executable(&m.join("k/jwt.md"));
    w.assert_fetched_and_left_as_it_was();
}

/// Every `*.lock` file under `dir`.
#[cfg(unix)]
fn lock_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lock") {
                found.push(path);
            }
        }
    }
    found
}

/// Unix: on Windows the fetch's job is terminated, which gives git no
/// chance to remove its locks (PAR-073).
#[cfg(unix)]
#[test]
fn a_fetch_stopped_while_it_holds_a_ref_lock_leaves_no_lock() {
    // A `reference-transaction` hook that sleeps once the refs are locked
    // (`prepared`) keeps the fetch past the 3 s.
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let m = w.mine();
    let prepared = w.root().join("prepared");
    let hook = m.join(".git").join("hooks").join("reference-transaction");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nif [ \"$1\" = prepared ]; then\n  touch '{}'\n  sleep 30\nfi\n",
            prepared.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    make_executable(&hook);
    let started = Instant::now();
    w.run(&["wake-up", "--json"]);
    assert!(started.elapsed() < Duration::from_secs(6));
    assert!(
        prepared.exists(),
        "the fetch never locked a ref: the test proves nothing"
    );
    fs::remove_file(&hook).unwrap();
    assert_eq!(lock_files(&m.join(".git")), Vec::<PathBuf>::new());
    // The next fetch and sync work.
    w.git(&m, &["fetch", "-q"]);
    assert_eq!(w.tracked_remote(), w.remote_head());
    let out = w
        .kluris()
        .args(["sync", "--brain", "team", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(w.head(), w.remote_head());
}

#[cfg(unix)]
#[test]
fn every_git_a_fresh_read_runs_has_optional_locks_off() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let bin = w.root().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = w.root().join("git.log");
    let git = bin.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\nprintf '%s|%s\\n' \"${{GIT_OPTIONAL_LOCKS-unset}}\" \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_git()
        ),
    )
    .unwrap();
    make_executable(&git);
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    for args in [&["wake-up", "--json"][..], &["search", "jwt", "--json"]] {
        let _ = fs::remove_file(w.stamp());
        let out = w.kluris().args(args).env("PATH", &path).output().unwrap();
        assert_eq!(out.status.code(), Some(0));
    }
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
    let text = fs::read_to_string(&log).unwrap();
    // The fresh read's own commands (what else wake-up and search run is
    // not this step's).
    let fresh: Vec<&str> = text
        .lines()
        .filter(|line| {
            [
                "--is-inside-work-tree",
                "symbolic-ref",
                "for-each-ref",
                "core.sshCommand",
                "fetch",
            ]
            .iter()
            .any(|word| line.contains(word))
        })
        .collect();
    assert!(fresh.iter().any(|l| l.contains("fetch")), "{text}");
    assert!(fresh.len() >= 10, "{text}");
    for line in fresh {
        assert!(line.starts_with("0|"), "{line}\n{text}");
    }
}

#[cfg(unix)]
fn real_git() -> String {
    let out = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(!real.is_empty());
    real
}

#[test]
fn the_brain_is_fetched_at_most_once_a_minute() {
    let w = World::new();
    w.wake_up();
    let seen = w.tracked_remote();
    w.teammate_pushes("zebra", "zebras");
    w.wake_up();
    assert_eq!(w.tracked_remote(), seen, "fetched seconds ago");
    w.search_files("zebras");
    assert_eq!(w.tracked_remote(), seen);
    // Well inside the minute: kluris's clock runs a little after the test's.
    w.set_stamp_age(50);
    w.wake_up();
    assert_eq!(w.tracked_remote(), seen);
    w.set_stamp_age(61);
    w.wake_up();
    assert_eq!(w.tracked_remote(), w.remote_head());
    w.teammate_pushes("okapi", "okapis");
    w.set_stamp_age(-3600);
    w.search_files("okapis");
    assert_eq!(
        w.tracked_remote(),
        w.remote_head(),
        "a stamp from the future does not throttle"
    );
}

#[test]
fn turned_off_by_the_variable_and_by_kluris_yml() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    let behind = w.tracked_remote();
    let out = w
        .kluris()
        .args(["wake-up", "--json"])
        .env("KLURIS_NO_AUTO_SYNC", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(w.tracked_remote(), behind);
    assert!(!w.stamp().exists());

    w.write(
        &w.mine(),
        "kluris.yml",
        "name: team\nsync:\n  auto_fetch: false\n",
    );
    w.wake_up();
    w.search_files("zebras");
    assert_eq!(w.tracked_remote(), behind);
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
    assert_eq!(w.tracked_remote(), w.remote_head());
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
fn a_merge_in_progress_is_fetched_and_left_untouched() {
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
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
}

#[test]
fn an_open_kluris_conflict_is_fetched_and_left_untouched() {
    let w = World::new();
    let m = w.mine();
    // `kluris sync` left a conflict for `kluris resolve`.
    let pending = m.join(".git").join("kluris-sync").join("pending.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    fs::write(&pending, "{}").unwrap();
    w.teammate_pushes("zebra", "zebras");
    w.assert_untouched();
    assert!(pending.is_file());
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
}

#[test]
fn a_conflicted_merge_is_fetched_and_left_untouched() {
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
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
}

#[test]
fn a_rebase_in_progress_is_fetched_and_left_untouched() {
    let w = World::new();
    w.teammate_pushes("zebra", "zebras");
    // git's own marker of a stopped rebase (as `git rebase -i` leaves it).
    fs::create_dir_all(w.mine().join(".git").join("rebase-merge")).unwrap();
    w.assert_untouched();
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
}

#[test]
fn a_diverged_branch_is_fetched_and_left_untouched() {
    let w = World::new();
    w.write(&w.mine(), "k/mine.md", "# mine\n");
    w.commit(&w.mine(), "mine");
    w.teammate_pushes("zebra", "zebras");
    w.assert_untouched();
    let merge_base = w.git(&w.mine(), &["merge-base", "HEAD", "origin/main"]);
    assert_ne!(w.head(), merge_base, "still my own commit on top");
    assert_eq!(w.tracked_remote(), w.remote_head(), "fetched");
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
