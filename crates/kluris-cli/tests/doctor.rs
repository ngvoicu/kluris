//! `kluris doctor [--no-refresh] [--json]` (research-03 §1.13, SPEC
//! acceptance criteria, interview-01 #8): v2's checks without the Python one,
//! a real git check, the PATH-shadow warning, the config and per-brain
//! checks, and the refresh (companions, then the skill install with its v2
//! migration). Every run sets PATH itself.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::Value;

// --- the sandbox ----------------------------------------------------------------

struct Sandbox {
    _tmp: TempDir,
    root: PathBuf,
    home: PathBuf,
    /// An empty folder for fake programs, first on PATH when used.
    fake: PathBuf,
    brains: Vec<(String, String)>,
}

fn kluris_bin() -> PathBuf {
    PathBuf::from(assert_cmd::cargo::cargo_bin!("kluris"))
}

/// The folder of the `kluris` under test.
fn bin_dir() -> PathBuf {
    kluris_bin().parent().expect("bin dir").to_path_buf()
}

/// The folder of the real `git` on the test's own PATH.
fn git_dir() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&path)
        .find(|dir| dir.join(name).is_file())
        .expect("a git on PATH")
}

fn join_path(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs).expect("join PATH")
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().canonicalize().expect("canonical");
        let home = root.join("home");
        let fake = root.join("fake");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&fake).expect("fake");
        Sandbox {
            _tmp: tmp,
            root,
            home,
            fake,
            brains: Vec::new(),
        }
    }

    fn config(&self) -> PathBuf {
        self.home.join(".kluris").join("config.yml")
    }

    fn h(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    /// Register `<root>/brains/<name>` with a kluris.yml holding `yml`
    /// (`None`: none written; the folder is created unless `create` is false).
    fn brain_with(&mut self, name: &str, yml: Option<&str>, create: bool) -> PathBuf {
        let path = self.root.join("brains").join(name);
        if create {
            fs::create_dir_all(&path).expect("brain");
            fs::write(path.join("brain.md"), format!("# {name}\n")).expect("brain.md");
        }
        if let Some(yml) = yml {
            fs::write(path.join("kluris.yml"), yml).expect("kluris.yml");
        }
        self.brains
            .push((name.to_string(), path.to_string_lossy().into_owned()));
        self.write_config();
        path
    }

    /// A brain whose kluris.yml selects `agents` and opts into `companions`
    /// (as v2's `yaml.dump` writes it).
    fn brain(&mut self, name: &str, agents: &[&str], companions: &[&str]) -> PathBuf {
        let list = |items: &[&str], indent: &str| -> String {
            if items.is_empty() {
                " []\n".to_string()
            } else {
                let lines: String = items.iter().map(|i| format!("{indent}- {i}\n")).collect();
                format!("\n{lines}")
            }
        };
        let yml = format!(
            "name: {name}\ndescription: ''\nagents:\n  commands_for:{}companions:{}",
            list(agents, "  "),
            list(companions, "")
        );
        self.brain_with(name, Some(&yml), true)
    }

    fn write_config(&self) {
        let mut text = String::from("brains:\n");
        for (name, path) in &self.brains {
            text.push_str(&format!(
                "  {name}:\n    path: '{}'\n    description: ''\n",
                path.replace('\'', "''")
            ));
        }
        put(&self.config(), &text);
    }

    /// `kluris` with HOME, the config and PATH (`dirs`, in order) set.
    fn kluris(&self, dirs: &[&Path]) -> Command {
        let mut cmd = Command::new(kluris_bin());
        cmd.env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("KLURIS_CONFIG", self.config())
            .env("KLURIS_NO_PROMPT", "1")
            .env("NO_COLOR", "1")
            .env("PATH", join_path(dirs))
            .env_remove("COPILOT_HOME");
        cmd
    }

    /// The usual PATH: this kluris, then git.
    fn doctor(&self, args: &[&str]) -> Run {
        run(self
            .kluris(&[&bin_dir(), &git_dir()])
            .arg("doctor")
            .args(args))
    }

    /// Every file and folder under the sandbox root, with file bytes.
    fn tree(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir") {
                let entry = entry.expect("entry");
                let path = entry.path();
                let rel = path.strip_prefix(&self.root).expect("rel");
                let rel = rel.to_string_lossy().replace('\\', "/");
                if entry.file_type().expect("type").is_dir() {
                    out.insert(rel, None);
                    stack.push(path);
                } else {
                    out.insert(rel, Some(fs::read(&path).expect("read")));
                }
            }
        }
        out
    }
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("{e}: {:?} {:?}", self.stdout, self.stderr))
    }
}

fn run(cmd: &mut Command) -> Run {
    let out = cmd.output().expect("run kluris");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).expect("utf-8 stdout"),
        stderr: String::from_utf8(out.stderr).expect("utf-8 stderr"),
    }
}

fn check<'a>(doc: &'a Value, name: &str) -> &'a Value {
    doc["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no {name} check in {doc}"))
}

fn check_names(doc: &Value) -> Vec<String> {
    doc["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .map(|c| c["name"].as_str().expect("name").to_string())
        .collect()
}

/// A path as the JSON writes it: `/` between components.
fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// v2's own SKILL.md for brain `name` (kluris-core's fixture).
fn v2_skill(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kluris-core/tests/fixtures/skills/v2/basic.md");
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("acme", name)
}

#[cfg(unix)]
fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    put(path, text);
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

// --- the checks and their shape ----------------------------------------------------

#[test]
fn healthy_setup_passes_with_v2s_shape_and_no_python_check() {
    let mut s = Sandbox::new();
    let brain = s.brain("acme", &["claude"], &[]);
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let keys: Vec<&String> = doc.as_object().expect("object").keys().collect();
    assert_eq!(keys, ["ok", "checks", "companions", "skills"]);
    assert_eq!(doc["ok"], true);
    assert_eq!(
        check_names(&doc),
        ["git", "path", "config", "brain:acme", "skills"]
    );
    for c in doc["checks"].as_array().expect("checks") {
        let keys: Vec<&String> = c.as_object().expect("check").keys().collect();
        assert_eq!(keys, ["name", "passed", "detail"], "{c}");
        assert_eq!(c["passed"], true, "{c}");
    }
    let git = check(&doc, "git")["detail"].as_str().expect("detail");
    assert!(git.starts_with("git version "), "{git}");
    assert_eq!(
        check(&doc, "config")["detail"],
        s.config().to_string_lossy().as_ref()
    );
    assert_eq!(
        check(&doc, "brain:acme")["detail"],
        brain.to_string_lossy().as_ref()
    );
    // ~/.agents/skills and ~/.claude/skills, for the one own-folder agent.
    assert_eq!(
        check(&doc, "skills")["detail"],
        "2 files written for 1 agents across 1 brain(s)"
    );
    assert_eq!(doc["companions"], serde_json::json!([]));
    let skills = &doc["skills"];
    assert_eq!(skills["agents"], serde_json::json!(["claude"]));
    assert_eq!(skills["brain_errors"], serde_json::json!([]));
    assert_eq!(
        skills["destinations"][0],
        serde_json::json!({
            "path": slash(&s.h(".agents/skills")),
            "role": "universal",
            "alias_of": null,
            "written": ["kluris-acme"],
            "removed": [],
            "kept": [],
            "failed": [],
        })
    );
    assert!(s.h(".claude/skills/kluris-acme/SKILL.md").is_file());
}

#[test]
fn text_output_is_one_line_per_check() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    let r = s.doctor(&[]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert!(
        lines[0].starts_with("  PASS git: git version "),
        "{lines:?}"
    );
    assert!(lines.contains(&"  PASS skills: 2 files written for 1 agents across 1 brain(s)"));
    assert!(
        lines.contains(&format!("  PASS config: {}", s.config().display()).as_str()),
        "{lines:?}"
    );
    assert!(!r.stdout.contains("python"), "{}", r.stdout);
    assert!(
        lines.contains(&"    ~/.agents/skills: written kluris-acme"),
        "{lines:?}"
    );
}

#[test]
fn no_brains_and_no_config_still_pass() {
    let s = Sandbox::new();
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(check_names(&doc), ["git", "path", "config", "skills"]);
    let agents = doc["skills"]["agents"].as_array().expect("agents").len();
    assert_eq!(
        check(&doc, "skills")["detail"],
        format!("0 files written for {agents} agents across 0 brain(s)").as_str()
    );
}

// --- git ----------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_git_that_runs_but_fails_fails_the_check() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    executable(
        &s.fake.join("git"),
        "#!/bin/sh\necho 'broken git' >&2\nexit 1\n",
    );
    let r = run(s.kluris(&[&s.fake, &bin_dir()]).args(["doctor", "--json"]));
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(doc["ok"], false);
    let git = check(&doc, "git");
    assert_eq!(git["passed"], false);
    let detail = git["detail"].as_str().expect("detail");
    assert!(detail.contains("git --version"), "{detail}");
    assert!(detail.contains("exit status: 1"), "{detail}");
    assert!(detail.contains("broken git"), "{detail}");
    // The rest still ran.
    assert_eq!(check(&doc, "skills")["passed"], true);
}

#[cfg(windows)]
#[test]
fn a_git_that_runs_but_fails_fails_the_check() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    // `find --version` runs and exits non-zero.
    let system = std::env::var_os("SystemRoot").expect("SystemRoot");
    fs::copy(
        Path::new(&system).join("System32").join("find.exe"),
        s.fake.join("git.exe"),
    )
    .expect("copy find.exe");
    let r = run(s.kluris(&[&s.fake, &bin_dir()]).args(["doctor", "--json"]));
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let git = check(&r.json(), "git").clone();
    assert_eq!(git["passed"], false);
    assert!(
        git["detail"]
            .as_str()
            .expect("detail")
            .contains("git --version")
    );
}

#[test]
fn no_git_on_path_fails_with_v2s_text() {
    let s = Sandbox::new();
    let r = run(s.kluris(&[&bin_dir(), &s.fake]).args(["doctor", "--json"]));
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let git = check(&r.json(), "git").clone();
    assert_eq!(git["passed"], false);
    assert_eq!(
        git["detail"],
        "git not found. Install: https://git-scm.com/downloads"
    );
}

#[test]
fn a_failed_check_prints_its_text_line_and_exits_1() {
    let s = Sandbox::new();
    let r = run(s.kluris(&[&bin_dir(), &s.fake]).arg("doctor"));
    assert_eq!(r.code, 1);
    assert!(
        r.stdout
            .contains("  FAIL git: git not found. Install: https://git-scm.com/downloads\n"),
        "{}",
        r.stdout
    );
    assert_eq!(r.stderr, "");
}

// --- the PATH-shadow warning --------------------------------------------------------

/// A `kluris` earlier on PATH: named, never run, a warning only.
fn shadowing_kluris(s: &Sandbox) -> (PathBuf, PathBuf) {
    let marker = s.root.join("shadow-ran");
    #[cfg(unix)]
    let other = {
        let other = s.fake.join("kluris");
        executable(
            &other,
            &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        );
        other
    };
    #[cfg(windows)]
    let other = {
        let other = s.fake.join("kluris.exe");
        put(&other, "not a program");
        other
    };
    (other, marker)
}

#[test]
fn a_kluris_earlier_on_path_is_named_as_a_warning() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    let (other, marker) = shadowing_kluris(&s);
    let r = run(s
        .kluris(&[&s.fake, &bin_dir(), &git_dir()])
        .args(["doctor", "--json"]));
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(doc["ok"], true);
    let path = check(&doc, "path");
    assert_eq!(path["passed"], true);
    assert_eq!(path["warning"], true);
    let detail = path["detail"].as_str().expect("detail");
    assert!(detail.contains(&other.display().to_string()), "{detail}");
    assert!(
        detail.contains(&bin_dir().display().to_string()),
        "{detail}"
    );
    assert!(!marker.exists(), "the other kluris was run");

    let text = run(s.kluris(&[&s.fake, &bin_dir(), &git_dir()]).arg("doctor"));
    assert_eq!(text.code, 0);
    assert!(text.stdout.contains("  WARN path: "), "{}", text.stdout);
    assert!(!marker.exists(), "the other kluris was run");
}

#[test]
fn this_kluris_first_on_path_passes() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    let (_other, marker) = shadowing_kluris(&s);
    let r = run(s
        .kluris(&[&bin_dir(), &s.fake, &git_dir()])
        .args(["doctor", "--json"]));
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let path = check(&doc, "path");
    assert_eq!(path["passed"], true);
    assert!(path.get("warning").is_none(), "{path}");
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn a_kluris_that_is_not_executable_is_not_a_shadow() {
    let s = Sandbox::new();
    put(&s.fake.join("kluris"), "not executable");
    let r = run(s.kluris(&[&s.fake, &bin_dir(), &git_dir()]).args([
        "doctor",
        "--no-refresh",
        "--json",
    ]));
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert!(check(&r.json(), "path").get("warning").is_none());
}

#[test]
fn not_on_path_at_all_is_a_warning() {
    let s = Sandbox::new();
    let r = run(s
        .kluris(&[&s.fake])
        .args(["doctor", "--no-refresh", "--json"]));
    let doc = r.json();
    let path = check(&doc, "path");
    assert_eq!(path["passed"], true);
    assert_eq!(path["warning"], true);
    assert!(
        path["detail"]
            .as_str()
            .expect("detail")
            .contains("not on PATH"),
        "{path}"
    );
}

// --- config and brains ----------------------------------------------------------------

#[test]
fn a_missing_brain_path_fails_its_check() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    let ghost = s.brain_with("ghost", None, false);
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(check(&doc, "brain:acme")["passed"], true);
    let c = check(&doc, "brain:ghost");
    assert_eq!(c["passed"], false);
    assert_eq!(
        c["detail"],
        format!("brain path not found: {}", ghost.display()).as_str()
    );
    // Still refreshed: the skills of both brains.
    assert!(s.h(".agents/skills/kluris-ghost/SKILL.md").is_file());
}

#[test]
fn an_invalid_kluris_yml_fails_its_check() {
    let mut s = Sandbox::new();
    s.brain_with("bad", Some("name: [unclosed\n"), true);
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let c = check(&doc, "brain:bad");
    assert_eq!(c["passed"], false);
    let detail = c["detail"].as_str().expect("detail");
    assert!(detail.starts_with("Invalid config file "), "{detail}");
    assert!(detail.contains("kluris.yml"), "{detail}");
    assert_eq!(doc["skills"]["brain_errors"][0]["brain"], "bad");
    // Reported once, by the brain check: the skills check passes.
    assert_eq!(check(&doc, "skills")["passed"], true);
}

#[test]
fn a_missing_kluris_yml_fails_its_check() {
    let mut s = Sandbox::new();
    s.brain_with("bare", None, true);
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let c = check(&r.json(), "brain:bare").clone();
    assert_eq!(c["passed"], false);
    assert!(
        c["detail"].as_str().expect("detail").contains("kluris.yml"),
        "{c}"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_kluris_yml_fails_its_check() {
    use std::os::unix::fs::PermissionsExt;
    let mut s = Sandbox::new();
    let brain = s.brain("locked", &["claude"], &[]);
    let yml = brain.join("kluris.yml");
    fs::set_permissions(&yml, fs::Permissions::from_mode(0o000)).expect("chmod");
    let r = s.doctor(&["--json", "--no-refresh"]);
    fs::set_permissions(&yml, fs::Permissions::from_mode(0o644)).expect("chmod back");
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let c = check(&r.json(), "brain:locked").clone();
    assert_eq!(c["passed"], false);
    assert!(
        c["detail"]
            .as_str()
            .expect("detail")
            .starts_with("Cannot access config file "),
        "{c}"
    );
}

#[test]
fn a_config_that_does_not_read_fails_and_nothing_is_refreshed() {
    let s = Sandbox::new();
    put(&s.config(), "brains: [unclosed\n");
    put(
        &s.h(".agents/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    let before = s.tree();
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(check_names(&doc), ["git", "path", "config", "skills"]);
    let config = check(&doc, "config");
    assert_eq!(config["passed"], false);
    assert!(
        config["detail"]
            .as_str()
            .expect("detail")
            .starts_with("Invalid config file "),
        "{config}"
    );
    let skills = check(&doc, "skills");
    assert_eq!(skills["passed"], false);
    assert!(
        skills["detail"]
            .as_str()
            .expect("detail")
            .starts_with("refresh skipped"),
        "{skills}"
    );
    assert_eq!(doc["skills"], Value::Null);
    assert_eq!(s.tree(), before, "nothing may be written");
}

// --- --no-refresh -------------------------------------------------------------------

#[test]
fn no_refresh_writes_nothing() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &["specmint-core"]);
    put(
        &s.h(".cursor/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(&s.h(".claude/skills/kluris-acme/SKILL.md"), "OUTDATED");
    put(&s.h(".agents/skills/.kluris-gone.tmp/SKILL.md"), "half");
    let before = s.tree();
    let r = s.doctor(&["--no-refresh", "--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(check_names(&doc), ["git", "path", "config", "brain:acme"]);
    assert_eq!(doc["companions"], serde_json::json!([]));
    assert_eq!(doc["skills"], Value::Null);
    assert_eq!(s.tree(), before);
}

#[test]
fn no_refresh_on_a_fresh_home_creates_nothing() {
    let s = Sandbox::new();
    let before = s.tree();
    let r = s.doctor(&["--no-refresh", "--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert_eq!(s.tree(), before);
    assert!(!s.h(".kluris").exists());
}

// --- the refresh ----------------------------------------------------------------------

#[test]
fn referenced_companions_are_refreshed() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &["specmint-core"]);
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert_eq!(
        doc["companions"],
        serde_json::json!([{"name": "specmint-core", "refreshed": true}])
    );
    assert_eq!(
        check(&doc, "companion:specmint-core")["detail"],
        "refreshed specmint-core"
    );
    assert_eq!(
        check_names(&doc),
        [
            "git",
            "path",
            "config",
            "brain:acme",
            "companion:specmint-core",
            "skills"
        ]
    );
    assert!(s.h(".kluris/companions/specmint-core/SKILL.md").is_file());
    let skill = fs::read_to_string(s.h(".agents/skills/kluris-acme/SKILL.md")).expect("skill");
    assert!(skill.contains("specmint-core/SKILL.md"), "{skill}");
}

#[test]
fn an_installed_companion_nobody_references_is_refreshed_too() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    put(
        &s.h(".kluris/companions/specmint-tdd/SKILL.md"),
        "old single file",
    );
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert_eq!(
        r.json()["companions"],
        serde_json::json!([{"name": "specmint-tdd", "refreshed": true}])
    );
    let text = fs::read_to_string(s.h(".kluris/companions/specmint-tdd/SKILL.md")).expect("skill");
    assert_ne!(text, "old single file");
}

#[test]
fn a_v2_layout_is_migrated_and_reported() {
    let mut s = Sandbox::new();
    s.brain("acme", &[], &[]);
    for rel in [".cursor/skills", ".codex/skills", ".claude/skills"] {
        put(
            &s.h(&format!("{rel}/kluris-acme/SKILL.md")),
            &v2_skill("acme"),
        );
    }
    put(
        &s.h(".cursor/skills/kluris-notes/SKILL.md"),
        "---\nname: kluris-notes\n---\nmine\n",
    );
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let cursor = doc["skills"]["destinations"]
        .as_array()
        .expect("destinations")
        .iter()
        .find(|d| d["path"] == slash(&s.h(".cursor/skills")).as_str())
        .expect("cursor destination")
        .clone();
    assert_eq!(cursor["role"], "migration");
    assert_eq!(cursor["removed"], serde_json::json!(["kluris-acme"]));
    assert!(!s.h(".cursor/skills/kluris-acme").exists());
    assert!(!s.h(".codex/skills/kluris-acme").exists());
    assert!(s.h(".cursor/skills/kluris-notes/SKILL.md").is_file());
    let claude = fs::read_to_string(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("claude");
    assert!(claude.contains("<!-- kluris:generated"), "{claude}");

    let text = s.doctor(&[]);
    assert_eq!(text.code, 0);
    // Nothing left to migrate on the second run.
    assert!(!text.stdout.contains("removed"), "{}", text.stdout);
}

#[test]
fn removed_entries_are_listed_under_the_skills_line() {
    let mut s = Sandbox::new();
    s.brain("acme", &[], &[]);
    put(
        &s.h(".cursor/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    let r = s.doctor(&[]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("    ~/.cursor/skills: removed kluris-acme\n"),
        "{}",
        r.stdout
    );
}

/// An earlier run's staging folder holding a Kluris-written skill is
/// removed; one holding anything else is kept and listed under `kept`.
#[test]
fn stale_staging_folders_are_removed() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    let half = ".kluris-gone.4242-0000000000000001.tmp";
    let old = ".kluris-gone.4242-0000000000000002.old";
    put(
        &s.h(".agents/skills").join(half).join("SKILL.md"),
        "half written",
    );
    put(
        &s.h(".claude/skills").join(old).join("SKILL.md"),
        &v2_skill("gone"),
    );
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    assert!(s.h(".agents/skills").join(half).join("SKILL.md").is_file());
    assert!(!s.h(".claude/skills").join(old).exists());
    let universal = &doc["skills"]["destinations"][0];
    assert_eq!(universal["kept"], serde_json::json!([half]));
    assert_eq!(universal["removed"], serde_json::json!([]));
    assert_eq!(
        doc["skills"]["destinations"][1]["removed"],
        serde_json::json!([old])
    );
    let text = s.doctor(&[]);
    assert!(
        text.stdout.contains(&format!(
            "    ~/.agents/skills: written kluris-acme; kept {half}\n"
        )),
        "{}",
        text.stdout
    );
}

/// A folder that is another name for one already handled is skipped and
/// reported as its alias, in JSON and in text.
#[cfg(unix)]
#[test]
fn an_aliased_folder_is_reported_as_such() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    fs::create_dir_all(s.h(".agents/skills")).expect("agents");
    fs::create_dir_all(s.h(".codex")).expect("codex");
    std::os::unix::fs::symlink("../.agents/skills", s.h(".codex/skills")).expect("link");
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let codex = doc["skills"]["destinations"]
        .as_array()
        .expect("destinations")
        .iter()
        .find(|d| d["path"] == slash(&s.h(".codex/skills")).as_str())
        .expect("codex destination")
        .clone();
    assert_eq!(codex["role"], "migration");
    assert_eq!(codex["alias_of"], slash(&s.h(".agents/skills")).as_str());
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
    let text = s.doctor(&[]);
    assert!(
        text.stdout
            .contains("    ~/.codex/skills: same folder as ~/.agents/skills\n"),
        "{}",
        text.stdout
    );
}

/// A companion backup a failed refresh left (T-103 #8) is restored, then
/// refreshed, and the check says so.
#[test]
fn an_outstanding_companion_backup_is_restored_and_reported() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    put(
        &s.h(".kluris/companions/.specmint-core.old/SKILL.md"),
        "the copy a failed refresh kept",
    );
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 0, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let detail = check(&doc, "companion:specmint-core")["detail"]
        .as_str()
        .expect("detail")
        .to_string();
    assert!(detail.starts_with("refreshed specmint-core"), "{detail}");
    assert!(detail.contains(".specmint-core.old"), "{detail}");
    assert!(s.h(".kluris/companions/specmint-core/SKILL.md").is_file());
    assert!(!s.h(".kluris/companions/.specmint-core.old").exists());
}

#[test]
fn a_failed_destination_fails_the_skills_check() {
    let mut s = Sandbox::new();
    s.brain("acme", &["claude"], &[]);
    put(&s.h(".claude/skills"), "not a folder");
    let r = s.doctor(&["--json"]);
    assert_eq!(r.code, 1, "{} {}", r.stdout, r.stderr);
    let doc = r.json();
    let skills = check(&doc, "skills");
    assert_eq!(skills["passed"], false);
    assert_eq!(
        skills["detail"],
        "1 files written for 1 agents across 1 brain(s) (failed: ~/.claude/skills)"
    );
    let failed: Vec<&Value> = doc["skills"]["destinations"]
        .as_array()
        .expect("destinations")
        .iter()
        .filter(|d| !d["failed"].as_array().expect("failed").is_empty())
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["path"], slash(&s.h(".claude/skills")).as_str());
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());

    let text = s.doctor(&[]);
    assert!(text.stdout.contains("  FAIL skills: "), "{}", text.stdout);
    assert!(
        text.stdout.contains("    ~/.claude/skills: failed: "),
        "{}",
        text.stdout
    );
}

#[test]
fn doctor_is_listed_in_root_help() {
    let s = Sandbox::new();
    let r = run(s.kluris(&[&bin_dir()]).arg("--help"));
    assert_eq!(r.code, 0);
    // The wording is checked; the column width is clap's layout (padded to
    // the longest command name), which parity does not cover.
    use predicates::prelude::*;
    let line = predicates::str::is_match(
        r"(?m)^  doctor +Check prerequisites and refresh installed agent skills\.$",
    )
    .unwrap();
    assert!(line.eval(&r.stdout), "{}", r.stdout);
}
