//! Homes with brains pushed to bare remotes and a teammate's clone each,
//! for the sync and resolve tests.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::Value;

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

pub fn neuron(title: &str, body: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# {title}\n\n{body}"
    )
}

pub const BODY: &str = "One.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";

pub fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// One home at a time: each runs several git processes per step, and the
/// Windows runner caps a run at 64 processes.
pub static SERIAL: Mutex<()> = Mutex::new(());

pub struct Home {
    pub _serial: MutexGuard<'static, ()>,
    pub dir: TempDir,
    pub brains: Vec<(String, PathBuf)>,
    /// Variables every `kluris` run gets on top of the hermetic ones.
    pub env: Vec<(String, String)>,
}

impl Home {
    pub fn new() -> Home {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        Home {
            _serial: serial,
            dir,
            brains: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn hermetic(&self, cmd: &mut StdCommand) {
        cmd.env("HOME", self.dir.path())
            .env("USERPROFILE", self.dir.path())
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "KLURIS_CONFIG",
                self.dir.path().join(".kluris").join("config.yml"),
            )
            .env("KLURIS_NO_PROMPT", "1")
            .env("NO_COLOR", "1");
        for var in ISOLATED_VARS {
            cmd.env_remove(var);
        }
    }

    pub fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = StdCommand::new("git");
        self.hermetic(&mut cmd);
        let out = cmd.args(args).current_dir(cwd).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn write(&self, at: &Path, rel: &str, text: &str) {
        let path = rel
            .split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn read(&self, at: &Path, rel: &str) -> String {
        let path = rel
            .split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part));
        fs::read_to_string(path).unwrap()
    }

    pub fn register(&mut self, name: &str, root: &Path) {
        self.brains.push((name.to_string(), root.to_path_buf()));
        let mut config = String::from("brains:\n");
        for (brain, path) in &self.brains {
            config.push_str(&format!(
                "  {brain}:\n    path: {}\n    description: ''\n",
                yaml_path(path)
            ));
        }
        let dir = self.dir.path().join(".kluris");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.yml"), config).unwrap();
    }

    /// A brain `name` at `<home>/<name>`, pushed to `<home>/<name>.git`, and
    /// a teammate's clone at `<home>/<name>-teammate`. Returns its root.
    pub fn brain(&mut self, name: &str) -> PathBuf {
        let root = self.dir.path().join(name);
        fs::create_dir_all(&root).unwrap();
        self.write(&root, ".gitignore", "kluris.yml\n");
        self.write(
            &root,
            "kluris.yml",
            &format!("name: {name}\ndescription: d\n"),
        );
        self.write(
            &root,
            "glossary.md",
            "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
        );
        self.write(&root, "notes/first.md", &neuron("First", BODY));
        self.register(name, &root);
        let (code, out, err) = self.run(&["dream", "--brain", name, "--json"]);
        assert_eq!(code, 0, "{out}{err}");
        self.git(&root, &["init", "-q", "-b", "main"]);
        self.configure(&root, "Ana Tester", "ana@example.com");
        self.git(&root, &["add", "-A"]);
        self.git(&root, &["commit", "-q", "-m", "init"]);
        let bare = self.dir.path().join(format!("{name}.git"));
        let bare_s = bare.to_string_lossy().into_owned();
        self.git(
            self.dir.path(),
            &["init", "-q", "--bare", "-b", "main", &bare_s],
        );
        self.git(&root, &["remote", "add", "origin", &bare_s]);
        self.git(&root, &["push", "-q", "-u", "origin", "main"]);
        let mate = self.teammate(name);
        let mate_s = mate.to_string_lossy().into_owned();
        self.git(self.dir.path(), &["clone", "-q", &bare_s, &mate_s]);
        self.configure(&mate, "Bob Teammate", "bob@example.com");
        root
    }

    pub fn configure(&self, at: &Path, name: &str, email: &str) {
        self.git(at, &["config", "user.name", name]);
        self.git(at, &["config", "user.email", email]);
        self.git(at, &["config", "core.autocrlf", "false"]);
    }

    pub fn teammate(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}-teammate"))
    }

    pub fn commit(&self, at: &Path, rel: &str, text: &str) {
        self.write(at, rel, text);
        self.git(at, &["add", "-A"]);
        self.git(at, &["commit", "-q", "-m", rel]);
    }

    pub fn teammate_pushes(&self, name: &str, rel: &str, text: &str) {
        let mate = self.teammate(name);
        self.commit(&mate, rel, text);
        self.git(&mate, &["push", "-q"]);
    }

    pub fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut std = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.hermetic(&mut std);
        for (key, value) in &self.env {
            std.env(key, value);
        }
        let out = Command::from_std(std).args(args).output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    pub fn json(&self, args: &[&str]) -> (i32, Value) {
        let (code, out, err) = self.run(args);
        let doc = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}{err}"));
        (code, doc)
    }

    /// The overlapping edit that cannot be auto-merged.
    pub fn conflict(&self, name: &str, root: &Path) {
        self.teammate_pushes(
            name,
            "notes/first.md",
            &neuron("First", &BODY.replace("Three.", "Three, theirs.")),
        );
        self.commit(
            root,
            "notes/first.md",
            &neuron("First", &BODY.replace("Three.", "Three, ours.")),
        );
    }
}

/// A fake ssh at `<home>/fake-ssh` that turns every key away as an ssh
/// server does (`Permission denied (publickey).`, exit 255), as a
/// `GIT_SSH_COMMAND` (Git for Windows' sh runs it too).
pub fn fake_ssh(home: &Path) -> String {
    let script = home.join("fake-ssh");
    fs::write(
        &script,
        "#!/bin/sh\necho 'Permission denied (publickey).' >&2\nexit 255\n",
    )
    .unwrap();
    format!("sh '{}'", script.display().to_string().replace('\\', "/"))
}

/// A `file://` URL of `path`.
pub fn file_url(path: &Path) -> String {
    let path = path.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// What git says when ssh turns the key away, on one line.
pub const DENIED: &str = "Permission denied (publickey). fatal: Could not read from remote repository. Please make sure you have the correct access rights and the repository exists";

/// The fix `refused` ends with, after git's message.
pub fn the_fix(command: &str) -> String {
    format!(
        "; check that this machine can run `git {command} origin` in this brain without a prompt"
    )
}

/// A brain's `sync` object brought commits in.
pub fn pulled(sync: &Value) -> bool {
    sync["pulled"].as_u64().is_some_and(|n| n > 0)
}

/// Why a brain's `sync` object was refused; `null` when it was not.
pub fn refused(sync: &Value) -> Value {
    let left = &sync["left_as_is"];
    if left["reason"] == "refused" {
        left["detail"].clone()
    } else {
        Value::Null
    }
}

impl Home {
    /// The id of brain `name`'s waiting merge, as `kluris status --json`
    /// reports it. A status that leaves commits waiting starts a
    /// background push; it is waited for, so the command after it finds
    /// the remote free.
    pub fn merge(&self, name: &str) -> String {
        let (_, doc) = self.json(&["status", "--brain", name, "--json"]);
        let sync = &doc["brains"][0]["sync"];
        if sync["to_push"].as_u64().is_some_and(|n| n > 0)
            && let Some((_, root)) = self.brains.iter().find(|(brain, _)| brain == name)
        {
            wait_for_background(root);
        }
        let merge = &sync["conflicts"][0]["merge"];
        merge
            .as_str()
            .unwrap_or_else(|| panic!("no merge: {doc}"))
            .to_string()
    }
}

impl Home {
    /// Brain `name` with a conflict waiting on each of `rels`: each is
    /// committed and shared first, then both sides change its third
    /// paragraph, the teammate pushes, and `kluris sync` records the
    /// conflict. Returns the root and the waiting merge's id.
    pub fn conflicts(&mut self, name: &str, rels: &[&str]) -> (PathBuf, String) {
        let root = self.brain(name);
        let mate = self.teammate(name);
        for rel in rels {
            if !root.join(rel).exists() {
                self.commit(&root, rel, &neuron(rel, BODY));
            }
        }
        self.git(&root, &["push", "-q"]);
        self.git(&mate, &["pull", "-q"]);
        for rel in rels {
            let title = title_of(&self.read(&root, rel));
            self.write(&mate, rel, &neuron(&title, &theirs_body()));
            self.write(&root, rel, &neuron(&title, &ours_body()));
        }
        self.git(&mate, &["commit", "-q", "-am", "theirs"]);
        self.git(&mate, &["push", "-q"]);
        self.git(&root, &["commit", "-q", "-am", "ours"]);
        let (code, doc) = self.json(&["sync", "--brain", name, "--json"]);
        assert_eq!(code, 1, "{doc}");
        let conflicts = &doc["brains"][0]["sync"]["conflicts"];
        assert_eq!(
            conflicts.as_array().map(Vec::len),
            Some(rels.len()),
            "{doc}"
        );
        let merge = conflicts[0]["merge"].as_str().unwrap().to_string();
        (root, merge)
    }

    /// The `kluris` command with this home's environment, to start.
    pub fn command(&self, args: &[&str]) -> StdCommand {
        let mut cmd = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.hermetic(&mut cmd);
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd.args(args);
        cmd
    }
}

/// A neuron's `# Title`.
pub fn title_of(text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or("Untitled")
        .to_string()
}

/// [`BODY`] as this brain changes it in a conflict.
pub fn ours_body() -> String {
    BODY.replace("Three.", "Three, ours.")
}

/// [`BODY`] as the teammate changes it in a conflict.
pub fn theirs_body() -> String {
    BODY.replace("Three.", "Three, theirs.")
}

/// A `git://` URL on a loopback port nothing listens on: the remote is
/// offline.
pub fn offline_url() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("git://127.0.0.1:{port}/alpha.git")
}

pub fn keys(doc: &Value) -> Vec<String> {
    doc.as_object().unwrap().keys().cloned().collect()
}

/// Waits, at most a minute, for the background push a read started for
/// the brain at `root` to end: on Unix until its record is written; on
/// Windows, where the runner's job may keep it from starting, until no run
/// holds its lock.
pub fn wait_for_background(root: &Path) {
    let dir = root.join(".git").join("kluris-sync");
    let done = || {
        !dir.join("background.lock").exists()
            && (!cfg!(unix) || dir.join("last-push.json").is_file())
    };
    let until = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < until, "the background push did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The git on this process's `PATH`, by its full path.
#[cfg(unix)]
pub fn real_git() -> PathBuf {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("git"))
        .find(|git| git.is_file())
        .expect("git on PATH")
}

#[cfg(unix)]
impl Home {
    /// Put a `git` first on the `PATH` of every `kluris` run that runs the
    /// real one, except that once a `git merge --ff-only` has succeeded,
    /// the next `rev-parse` of `refs/heads/main` fails (exit 128): a check
    /// after a land that cannot be made. Returns the file the failure
    /// leaves, there once it happened.
    pub fn fail_the_check_after_a_land(&mut self) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let bin = self.dir.path().join("failing-git");
        fs::create_dir_all(&bin).unwrap();
        let landed = self.dir.path().join("landed");
        let failed = self.dir.path().join("check-failed");
        let quote =
            |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
        let script = format!(
            "#!/bin/sh\ncase \" $* \" in\n*\" merge --ff-only \"*)\n  {git} \"$@\"; status=$?\n  [ $status = 0 ] && touch {landed}\n  exit $status ;;\n*\" rev-parse --verify --quiet refs/heads/main \"*)\n  if [ -e {landed} ] && [ ! -e {failed} ]; then\n    touch {failed}; echo 'the reference could not be read' >&2; exit 128\n  fi ;;\nesac\nexec {git} \"$@\"\n",
            git = quote(&real_git()),
            landed = quote(&landed),
            failed = quote(&failed),
        );
        let wrapper = bin.join("git");
        fs::write(&wrapper, script).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin];
        dirs.extend(std::env::split_paths(&path));
        let joined = std::env::join_paths(dirs).unwrap();
        self.env
            .push(("PATH".into(), joined.to_string_lossy().into_owned()));
        failed
    }
}
