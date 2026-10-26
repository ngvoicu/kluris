//! `kluris sync [--brain NAME|all] [--no-verify] [--json]` (TEST-K3-45): the
//! command surface over real git (a bare remote and a teammate's clone per
//! brain). The merge rules themselves are in `kluris-core/tests/sync.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Mutex, MutexGuard};

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::{Value, json};

const ISOLATED_VARS: [&str; 8] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "KLURIS_AGENT",
];

fn neuron(title: &str, body: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# {title}\n\n{body}"
    )
}

const BODY: &str = "One.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";

fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// One home at a time: each runs several git processes per step, and the
/// Windows runner caps a run at 64 processes.
static SERIAL: Mutex<()> = Mutex::new(());

struct Home {
    _serial: MutexGuard<'static, ()>,
    dir: TempDir,
    brains: Vec<(String, PathBuf)>,
}

impl Home {
    fn new() -> Home {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        Home {
            _serial: serial,
            dir,
            brains: Vec::new(),
        }
    }

    fn hermetic(&self, cmd: &mut StdCommand) {
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

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
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

    fn write(&self, at: &Path, rel: &str, text: &str) {
        let path = rel
            .split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, at: &Path, rel: &str) -> String {
        let path = rel
            .split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part));
        fs::read_to_string(path).unwrap()
    }

    fn register(&mut self, name: &str, root: &Path) {
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
    fn brain(&mut self, name: &str) -> PathBuf {
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

    fn configure(&self, at: &Path, name: &str, email: &str) {
        self.git(at, &["config", "user.name", name]);
        self.git(at, &["config", "user.email", email]);
        self.git(at, &["config", "core.autocrlf", "false"]);
    }

    fn teammate(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}-teammate"))
    }

    fn commit(&self, at: &Path, rel: &str, text: &str) {
        self.write(at, rel, text);
        self.git(at, &["add", "-A"]);
        self.git(at, &["commit", "-q", "-m", rel]);
    }

    fn teammate_pushes(&self, name: &str, rel: &str, text: &str) {
        let mate = self.teammate(name);
        self.commit(&mate, rel, text);
        self.git(&mate, &["push", "-q"]);
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut std = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.hermetic(&mut std);
        let out = Command::from_std(std).args(args).output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let (code, out, err) = self.run(args);
        let doc = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}{err}"));
        (code, doc)
    }

    /// The overlapping edit that cannot be auto-merged.
    fn conflict(&self, name: &str, root: &Path) {
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

const BRAIN_KEYS: [&str; 10] = [
    "name",
    "fetched",
    "merged",
    "pushed",
    "auto_merged",
    "conflicts",
    "skipped_dirty",
    "offline",
    "blocked",
    "refused",
];

fn keys(doc: &Value) -> Vec<String> {
    doc.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn json_and_text_for_one_brain() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(keys(&doc), ["ok", "brains"]);
    let entry = &doc["brains"][0];
    assert_eq!(keys(entry), BRAIN_KEYS);
    assert_eq!(
        *entry,
        json!({
            "name": "alpha", "fetched": true, "merged": true, "pushed": false,
            "auto_merged": [], "conflicts": [], "skipped_dirty": [], "offline": false,
            "blocked": [], "refused": null,
        })
    );
    assert!(h.read(&root, "notes/new.md").contains("# New"));

    let (code, out, err) = h.run(&["sync", "--brain", "alpha"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("alpha: up to date"), "{out}");
}

#[test]
fn brain_all_syncs_the_clean_brain_and_reports_the_conflicted_one() {
    let mut h = Home::new();
    let alpha = h.brain("alpha");
    h.brain("beta");
    h.conflict("alpha", &alpha);
    h.teammate_pushes("beta", "notes/new.md", &neuron("New", ""));
    let (code, doc) = h.json(&["sync", "--brain", "all", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    assert_eq!(brains.len(), 2);
    let a = &brains[0];
    assert_eq!(a["name"], "alpha");
    assert_eq!(a["merged"], false);
    let c = &a["conflicts"][0];
    assert_eq!(keys(c), ["path", "kind", "base", "ours", "theirs"]);
    assert_eq!(c["path"], "notes/first.md");
    assert_eq!(c["kind"], "content");
    let b = &brains[1];
    assert_eq!(b["name"], "beta");
    assert_eq!(b["merged"], true);
    assert_eq!(b["refused"], Value::Null);

    let (code, out, _) = h.run(&["sync", "--brain", "all"]);
    assert_eq!(code, 1);
    assert!(out.contains("kluris resolve"), "{out}");
}

#[test]
fn a_refused_brain_does_not_stop_the_others() {
    let mut h = Home::new();
    h.brain("alpha");
    h.teammate_pushes("alpha", "notes/new.md", &neuron("New", ""));
    // A brain inside a larger repo.
    let outer = h.dir.path().join("outer");
    let nested = outer.join("brains").join("nested");
    fs::create_dir_all(&nested).unwrap();
    h.write(&nested, "kluris.yml", "name: nested\ndescription: d\n");
    h.write(&nested, "notes/n.md", &neuron("N", ""));
    h.git(&outer, &["init", "-q", "-b", "main"]);
    h.register("nested", &nested);

    let (code, doc) = h.json(&["sync", "--brain", "all", "--json"]);
    assert_eq!(code, 1, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    assert_eq!(brains[0]["name"], "alpha");
    assert_eq!(brains[0]["merged"], true);
    assert_eq!(keys(&brains[1]), BRAIN_KEYS);
    let refused = brains[1]["refused"].as_str().unwrap();
    assert!(refused.contains("larger git repository"), "{refused}");
    assert_eq!(brains[1]["fetched"], false);

    let (code, doc) = h.json(&["sync", "--brain", "nested", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(keys(&doc), ["ok", "error"]);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("larger git repository")
    );
}

#[test]
fn a_brain_blocked_by_dirty_files_exits_1() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes(
        "alpha",
        "notes/first.md",
        &neuron("First", &BODY.replace("One.", "One, theirs.")),
    );
    let mine = neuron("First", &BODY.replace("Five.", "Five, mine."));
    h.write(&root, "notes/first.md", &mine);
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["brains"][0]["blocked"], json!(["notes/first.md"]));
    assert_eq!(doc["brains"][0]["skipped_dirty"], json!(["notes/first.md"]));
    assert_eq!(h.read(&root, "notes/first.md"), mine);
    let (_, out, _) = h.run(&["sync", "--brain", "alpha"]);
    assert!(out.contains("commit or stash"), "{out}");
}

#[test]
fn sync_is_in_help() {
    let h = Home::new();
    let (code, doc) = h.json(&["help", "--json"]);
    assert_eq!(code, 0);
    let names: Vec<&str> = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let at = names
        .iter()
        .position(|n| *n == "sync")
        .expect("sync in help");
    assert_eq!(names[at - 1], "commit");
    let (code, out, _) = h.run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l.starts_with("  sync ")), "{out}");
    let (code, out, _) = h.run(&["sync", "--help"]);
    assert_eq!(code, 0);
    for text in [
        "Usage: kluris sync [OPTIONS]",
        "--brain",
        "--no-verify",
        "--json",
    ] {
        assert!(out.contains(text), "{text}: {out}");
    }
}
