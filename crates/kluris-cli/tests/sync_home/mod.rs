//! Homes with brains pushed to bare remotes and a teammate's clone each,
//! for the sync and resolve tests.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Mutex, MutexGuard};

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
            .env("KLURIS_NO_AUTO_SYNC", "1")
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

pub fn keys(doc: &Value) -> Vec<String> {
    doc.as_object().unwrap().keys().cloned().collect()
}
