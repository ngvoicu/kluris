//! What the commands that refresh the agent skills (`create`, `register`,
//! `remove`, `companion add`, `companion remove`) say when the install could
//! not write a skill (T-106: the report used to be dropped). The clean case
//! keeps v2's output; a blocked destination is named on stderr with how to
//! fix it, `--json` gains a `skills` object, and the exit code stays 0
//! (`parity/allowlist.md`, SPEC Decision Log 2026-10-26 "Skill ownership
//! (T-105)").

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::Value;

const FIX: &str = "move or delete it, then run `kluris doctor`";
const FOREIGN: &str = "---\nname: kluris-demo\ndescription: mine\n---\n\n# Mine\n";

/// `path` without Windows' `\\?\` prefix, as Kluris prints paths.
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

struct Env {
    _tmp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", self.stdout))
    }
}

impl Env {
    fn new() -> Env {
        let tmp = TempDir::new().unwrap();
        let root = plain(fs::canonicalize(tmp.path()).unwrap());
        let home = root.join("home");
        let work = root.join("work");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&work).unwrap();
        Env {
            _tmp: tmp,
            home,
            work,
        }
    }

    fn run(&self, args: &[&str]) -> Run {
        let out = Command::new(assert_cmd::cargo::cargo_bin!("kluris"))
            .current_dir(&self.work)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env(
                "KLURIS_CONFIG",
                self.home.join(".kluris").join("config.yml"),
            )
            .env("NO_COLOR", "1")
            .env_remove("KLURIS_NO_PROMPT")
            .env_remove("COPILOT_HOME")
            .args(args)
            .write_stdin("")
            .output()
            .unwrap();
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    fn universal(&self) -> PathBuf {
        self.home.join(".agents").join("skills")
    }

    /// A user's own folder where Kluris writes brain `name`'s skill.
    fn block(&self, name: &str) -> PathBuf {
        let dir = self.universal().join(format!("kluris-{name}"));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        let text = FOREIGN.replace("kluris-demo", &format!("kluris-{name}"));
        fs::write(dir.join("SKILL.md"), text).unwrap();
        dir
    }

    fn create(&self, name: &str) {
        let r = self.run(&["create", name, "--no-git", "--type", "blank", "--json"]);
        assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    }
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The blocked entry is kept as it was, stderr names it with the fix, and
/// the exit code is 0.
fn assert_blocked_text(env: &Env, r: &Run, blocked: &Path) {
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains(&blocked.display().to_string()),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains(FIX), "{}", r.stderr);
    assert!(
        fs::read_to_string(blocked.join("SKILL.md"))
            .unwrap()
            .starts_with("---\nname: kluris-"),
        "kept"
    );
    assert!(env.universal().is_dir());
}

/// `--json`: v2's keys, then `skills`: `{"failed": [...], "kept": [...]}`,
/// doctor's destination items, naming the universal folder.
fn assert_blocked_json(env: &Env, r: &Run, keys: &[&str], name: &str) {
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let got: Vec<&String> = doc.as_object().unwrap().keys().collect();
    let mut want: Vec<&str> = keys.to_vec();
    want.push("skills");
    assert_eq!(got, want, "{doc}");
    assert_eq!(doc["ok"], true);
    let skills = &doc["skills"];
    let item_keys: Vec<&String> = skills["failed"][0].as_object().unwrap().keys().collect();
    assert_eq!(
        item_keys,
        [
            "path", "role", "alias_of", "written", "removed", "kept", "failed"
        ]
    );
    let failed = skills["failed"].as_array().unwrap();
    let universal = failed
        .iter()
        .find(|d| d["path"] == slashed(&env.universal()).as_str())
        .unwrap_or_else(|| panic!("{skills}"));
    assert_eq!(universal["role"], "universal");
    assert!(
        universal["failed"][0].as_str().unwrap().contains(FIX),
        "{universal}"
    );
    let kept = skills["kept"].as_array().unwrap();
    assert!(
        kept.iter()
            .any(|d| d["path"] == slashed(&env.universal()).as_str()
                && d["kept"] == serde_json::json!([format!("kluris-{name}")])),
        "{skills}"
    );
}

// --- create -------------------------------------------------------------------

#[test]
fn create_clean_keeps_v2s_output() {
    let env = Env::new();
    let r = env.run(&["create", "demo", "--no-git", "--type", "blank", "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    let doc = r.json();
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["ok", "name", "path", "type", "lobes", "companions"]);

    let env = Env::new();
    let r = env.run(&["create", "demo", "--no-git", "--type", "blank"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    assert!(
        r.stdout.contains("Agent skill installed: kluris-demo\n"),
        "{}",
        r.stdout
    );
}

#[test]
fn create_with_a_blocked_skill_says_it_was_not_installed() {
    let env = Env::new();
    let blocked = env.block("demo");
    let r = env.run(&["create", "demo", "--no-git", "--type", "blank"]);
    assert_blocked_text(&env, &r, &blocked);
    assert!(!r.stdout.contains("Agent skill installed"), "{}", r.stdout);
    assert!(
        r.stdout.contains("Agent skill not installed: kluris-demo"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains(FIX), "the why: {}", r.stdout);
    assert!(r.stdout.contains("Brain created: demo"), "{}", r.stdout);
}

#[test]
fn create_json_with_a_blocked_skill_reports_it() {
    let env = Env::new();
    env.block("demo");
    let r = env.run(&["create", "demo", "--no-git", "--type", "blank", "--json"]);
    assert_blocked_json(
        &env,
        &r,
        &["ok", "name", "path", "type", "lobes", "companions"],
        "demo",
    );
}

// --- register -----------------------------------------------------------------

/// A brain on disk that is not registered: created, then removed.
fn unregistered(env: &Env) -> String {
    env.create("demo");
    let r = env.run(&["remove", "demo", "--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    env.work.join("demo").to_string_lossy().into_owned()
}

#[test]
fn register_with_a_blocked_skill_says_it_was_not_installed() {
    let env = Env::new();
    let path = unregistered(&env);
    let blocked = env.block("demo");
    let r = env.run(&["register", &path]);
    assert_blocked_text(&env, &r, &blocked);
    assert!(!r.stdout.contains("Agent skill installed"), "{}", r.stdout);
    assert!(
        r.stdout.contains("Agent skill not installed: kluris-demo"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("Brain registered: demo"), "{}", r.stdout);
}

#[test]
fn register_json_with_a_blocked_skill_reports_it() {
    let env = Env::new();
    let path = unregistered(&env);
    env.block("demo");
    let r = env.run(&["register", &path, "--json"]);
    assert_blocked_json(&env, &r, &["ok", "name", "path", "companions"], "demo");
}

#[test]
fn register_clean_keeps_v2s_output() {
    let env = Env::new();
    let path = unregistered(&env);
    let r = env.run(&["register", &path, "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    let doc = r.json();
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["ok", "name", "path", "companions"]);
}

// --- remove -------------------------------------------------------------------

#[test]
fn remove_with_another_brains_skill_blocked_reports_it() {
    let env = Env::new();
    env.create("demo");
    env.create("other");
    let blocked = env.block("other");
    let r = env.run(&["remove", "demo"]);
    assert_blocked_text(&env, &r, &blocked);
    assert!(
        r.stdout.contains("Unregistered: demo (files preserved)"),
        "{}",
        r.stdout
    );

    let env = Env::new();
    env.create("demo");
    env.create("other");
    env.block("other");
    let r = env.run(&["remove", "demo", "--json"]);
    assert_blocked_json(&env, &r, &["ok", "name"], "other");
}

#[test]
fn remove_clean_keeps_v2s_output() {
    let env = Env::new();
    env.create("demo");
    let r = env.run(&["remove", "demo", "--json"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    assert_eq!(r.stdout, "{\"ok\": true, \"name\": \"demo\"}\n");
}

// --- companion add / remove -----------------------------------------------------

#[test]
fn companion_add_with_a_blocked_skill_reports_it() {
    let env = Env::new();
    env.create("demo");
    let blocked = env.block("demo");
    let r = env.run(&["companion", "add", "specmint-core", "--brain", "demo"]);
    assert_blocked_text(&env, &r, &blocked);

    let env = Env::new();
    env.create("demo");
    env.block("demo");
    let r = env.run(&[
        "companion",
        "add",
        "specmint-core",
        "--brain",
        "demo",
        "--json",
    ]);
    let keys: Vec<String> = clean_companion_keys(&env, "add");
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    assert_blocked_json(&env, &r, &keys, "demo");
}

#[test]
fn companion_remove_with_a_blocked_skill_reports_it() {
    let env = Env::new();
    env.create("demo");
    let r = env.run(&[
        "companion",
        "add",
        "specmint-core",
        "--brain",
        "demo",
        "--json",
    ]);
    assert_eq!(r.code, 0);
    let blocked = env.block("demo");
    let r = env.run(&["companion", "remove", "specmint-core", "--brain", "demo"]);
    assert_blocked_text(&env, &r, &blocked);

    let env = Env::new();
    env.create("demo");
    let r = env.run(&[
        "companion",
        "add",
        "specmint-core",
        "--brain",
        "demo",
        "--json",
    ]);
    assert_eq!(r.code, 0);
    env.block("demo");
    let r = env.run(&[
        "companion",
        "remove",
        "specmint-core",
        "--brain",
        "demo",
        "--json",
    ]);
    let keys: Vec<String> = clean_companion_keys(&env, "remove");
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    assert_blocked_json(&env, &r, &keys, "demo");
}

/// The keys of a clean `companion add|remove --json` (v2's), from a run in a
/// fresh sandbox; that run must have no `skills` key and say nothing on
/// stderr.
fn clean_companion_keys(_blocked: &Env, action: &str) -> Vec<String> {
    let env = Env::new();
    env.create("demo");
    if action == "remove" {
        let r = env.run(&[
            "companion",
            "add",
            "specmint-core",
            "--brain",
            "demo",
            "--json",
        ]);
        assert_eq!(r.code, 0);
    }
    let r = env.run(&[
        "companion",
        action,
        "specmint-core",
        "--brain",
        "demo",
        "--json",
    ]);
    assert_eq!(r.code, 0);
    assert_eq!(r.stderr, "");
    let keys: Vec<String> = r.json().as_object().unwrap().keys().cloned().collect();
    assert!(!keys.contains(&"skills".to_string()), "{keys:?}");
    keys
}
