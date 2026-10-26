//! The decided fixes in `kluris create` and `kluris register` (TEST-K3-25;
//! each one is a `parity/allowlist.md` entry):
//! - `create --json` without a name is a JSON error (v2: a traceback);
//! - the wizard's brain type is checked like `--type`;
//! - nothing is asked when stdin is not a terminal or `KLURIS_NO_PROMPT=1`:
//!   a missing name (create) or path (register) is an error naming it, the
//!   other values take their defaults;
//! - `--from-config` refuses lobe names that are paths and nested values;
//! - new brain names follow `^[a-z][a-z0-9]*(-[a-z0-9]+)*$` (at most 48,
//!   not `all`), while names v2 allowed keep loading;
//! - a git failure is a clean error naming the command and what is left on
//!   disk, and the brain is not registered;
//! - a target path that is a file is an error (v2: a traceback).

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_fs::TempDir;
use kluris_core::create::{CreateInput, wizard};
use serde_json::Value;

const NAME_REQUIRED: &str =
    "Brain name is required in non-interactive mode. Run 'kluris create <name>'.";
const PATH_REQUIRED: &str =
    "Brain path is required in non-interactive mode. Run 'kluris register <path>'.";
const TYPE_PROMPT: &str =
    "  Brain type (product-group, personal, product, research, blank) [product-group]: ";
const NOT_A_TYPE: &str =
    "is not one of 'product-group', 'personal', 'product', 'research', 'blank'.";

/// `path` without Windows' `\\?\` prefix, as git and Kluris print paths
/// (and as git needs `GIT_CONFIG_GLOBAL`).
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

/// A sandbox: HOME, the config and a working directory, with git pinned to
/// an empty global config and a fixed identity.
struct Env {
    _tmp: TempDir,
    root: PathBuf,
    home: PathBuf,
    work: PathBuf,
}

impl Env {
    fn new() -> Env {
        let tmp = TempDir::new().unwrap();
        let root = plain(fs::canonicalize(tmp.path()).unwrap());
        let home = root.join("home");
        let work = root.join("work");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        Env {
            _tmp: tmp,
            root,
            home,
            work,
        }
    }

    fn config(&self) -> PathBuf {
        self.home.join(".kluris").join("config.yml")
    }

    fn kluris(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
        cmd.current_dir(&self.work)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("KLURIS_CONFIG", self.config())
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("NO_COLOR", "1")
            .env_remove("KLURIS_NO_PROMPT")
            .env_remove("COPILOT_HOME")
            .write_stdin("");
        cmd
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let out = self.kluris().args(args).output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let doc = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text:?}"));
        (out.status.code().unwrap_or(-1), doc)
    }

    fn error(&self, args: &[&str]) -> String {
        let (code, doc) = self.json(args);
        assert_eq!(code, 1, "{doc}");
        assert_eq!(doc["ok"], Value::Bool(false), "{doc}");
        doc["error"].as_str().unwrap().to_string()
    }

    fn config_text(&self) -> String {
        fs::read_to_string(self.config()).unwrap_or_default()
    }

    /// Every file and directory under the working directory and HOME.
    fn listing(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                out.push(path.strip_prefix(base).unwrap().display().to_string());
                if path.is_dir() {
                    walk(&path, base, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
}

// --- create --json without a name ------------------------------------------------

#[test]
fn create_json_without_a_name_is_a_json_error() {
    let env = Env::new();
    let before = env.listing();
    let out = env.kluris().args(["create", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{{\"ok\": false, \"error\": \"{NAME_REQUIRED}\"}}\n")
    );
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "");
    assert_eq!(env.listing(), before);
}

// --- the wizard ---------------------------------------------------------------------

fn ask(given: CreateInput, typed: &str) -> (Result<CreateInput, String>, String) {
    let mut out = Vec::new();
    let answers = wizard(given, Path::new("/brains"), &mut typed.as_bytes(), &mut out)
        .map_err(|e| format!("{e:?}"));
    (answers, String::from_utf8(out).unwrap())
}

#[test]
fn wizard_brain_type_is_checked_like_the_option() {
    let given = CreateInput {
        name: Some("wiz".into()),
        description: Some("d".into()),
        path: Some(".".into()),
        no_git: true,
        ..CreateInput::default()
    };
    let (answers, out) = ask(given, "foo\nPersonal\npersonal\n");
    assert_eq!(answers.unwrap().brain_type.as_deref(), Some("personal"));
    assert_eq!(
        out,
        format!(
            "{TYPE_PROMPT}Error: 'foo' {NOT_A_TYPE}\n{TYPE_PROMPT}Error: 'Personal' {NOT_A_TYPE}\n{TYPE_PROMPT}\n"
        )
    );
}

#[test]
fn wizard_empty_type_answer_is_the_default() {
    let given = CreateInput {
        name: Some("wiz".into()),
        description: Some("d".into()),
        path: Some(".".into()),
        no_git: true,
        ..CreateInput::default()
    };
    let (answers, out) = ask(given, "\n");
    assert_eq!(
        answers.unwrap().brain_type.as_deref(),
        Some("product-group")
    );
    assert_eq!(out, format!("{TYPE_PROMPT}\n"));
}

#[test]
fn wizard_asks_every_missing_value_like_click() {
    let (answers, out) = ask(
        CreateInput::default(),
        "\nwiz\n\n my desc \n\nblank\n2\n\ngit@x:y.git\ndev\n",
    );
    let answers = answers.unwrap();
    assert_eq!(answers.name.as_deref(), Some("wiz"));
    assert_eq!(answers.description.as_deref(), Some(" my desc "));
    assert_eq!(answers.path.as_deref(), Some("/brains"));
    assert_eq!(answers.brain_type.as_deref(), Some("blank"));
    assert_eq!(answers.remote.as_deref(), Some("git@x:y.git"));
    assert_eq!(answers.branch.as_deref(), Some("dev"));
    assert!(!answers.no_git);
    assert_eq!(
        out,
        "\nCreate a new brain\n\n  Brain name (lowercase, hyphens ok):   Brain name (lowercase, hyphens ok): \
         \u{20} What does this brain cover? (one sentence):   What does this brain cover? (one sentence): \
         \u{20} Location [/brains]: \
         \u{20} Brain type (product-group, personal, product, research, blank) [product-group]: \
         \u{20} Git setup (1=local, 2=with remote, 3=no git) [1]: \
         \u{20} Git remote URL:   Git remote URL:   Default branch [main]: \n"
    );
}

#[test]
fn wizard_end_of_input_aborts() {
    let (answers, _) = ask(CreateInput::default(), "wiz\n");
    assert!(answers.is_err());
}

#[test]
fn without_a_terminal_create_asks_nothing_and_needs_a_name() {
    let env = Env::new();
    let out = env.kluris().args(["create"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!("Error: {NAME_REQUIRED}\n")
    );
    assert!(
        !String::from_utf8(out.stdout)
            .unwrap()
            .contains("Brain name")
    );

    // With a name, every other value takes its default.
    let out = env
        .kluris()
        .args(["create", "demo", "--no-git"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Brain created: demo (product-group)"),
        "{stdout}"
    );
    assert!(!stdout.contains("What does this brain cover"), "{stdout}");
    let brain_md = fs::read_to_string(env.work.join("demo").join("brain.md")).unwrap();
    assert!(brain_md.contains("\ndemo knowledge base\n"), "{brain_md}");
    assert!(
        env.work
            .join("demo")
            .join("projects")
            .join("map.md")
            .is_file()
    );
}

#[test]
fn no_prompt_variable_asks_nothing() {
    let env = Env::new();
    let out = env
        .kluris()
        .env("KLURIS_NO_PROMPT", "1")
        .args(["create"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!("Error: {NAME_REQUIRED}\n")
    );
}

#[test]
fn without_a_terminal_register_needs_a_path() {
    let env = Env::new();
    assert_eq!(env.error(&["register", "--json"]), PATH_REQUIRED);
    let out = env.kluris().args(["register"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!("Error: {PATH_REQUIRED}\n")
    );
    assert!(
        !String::from_utf8(out.stdout)
            .unwrap()
            .contains("Path to brain")
    );
}

// --- --from-config ------------------------------------------------------------------------

#[test]
fn from_config_refuses_paths_and_nested_values() {
    let cases = [
        ("structure:\n  ../escape: x\n", "lobe name '../escape'"),
        ("structure:\n  a/b: x\n", "lobe name 'a/b'"),
        ("structure:\n  a\\b: x\n", "lobe name 'a\\\\b'"),
        ("structure:\n  /abs: x\n", "lobe name '/abs'"),
        ("structure:\n  C:/abs: x\n", "lobe name 'C:/abs'"),
        ("structure:\n  ..: x\n", "lobe name '..'"),
        ("structure:\n  .: x\n", "lobe name '.'"),
        ("structure:\n  '': x\n", "lobe name ''"),
        // The YAML reader refuses a key that is not a string as it loads.
        ("structure:\n  1: x\n", "mapping keys must be strings"),
        ("structure:\n  lobe:\n    sub: x\n", "lobe 'lobe'"),
        ("structure:\n  lobe: [a, b]\n", "lobe 'lobe'"),
        ("structure: [a, b]\n", "structure"),
        ("structure: null\n", "structure"),
        ("- a\n- b\n", "mapping"),
        ("structure: {a: [\n", "structure.yml"),
    ];
    for (text, needle) in cases {
        let env = Env::new();
        fs::write(env.work.join("structure.yml"), text).unwrap();
        let before = env.listing();
        let error = env.error(&["create", "demo", "--from-config", "structure.yml", "--json"]);
        assert!(error.starts_with("--from-config: "), "{text:?}: {error}");
        assert!(error.contains(needle), "{text:?}: {error}");
        assert_eq!(env.listing(), before, "{text:?} wrote something");
        assert!(!env.root.join("escape").exists());
    }
}

#[test]
fn from_config_keeps_scalar_values_as_python_text() {
    let env = Env::new();
    fs::write(
        env.work.join("structure.yml"),
        "structure:\n  stats: 42\n  flags: true\n",
    )
    .unwrap();
    let (code, doc) = env.json(&[
        "create",
        "demo",
        "--from-config",
        "structure.yml",
        "--no-git",
        "--json",
    ]);
    assert_eq!(code, 0, "{doc}");
    let brain_md = fs::read_to_string(env.work.join("demo").join("brain.md")).unwrap();
    assert!(
        brain_md.contains("[stats/](./stats/map.md) — 42"),
        "{brain_md}"
    );
    assert!(
        brain_md.contains("[flags/](./flags/map.md) — True"),
        "{brain_md}"
    );
}

// --- names ----------------------------------------------------------------------------------

#[test]
fn new_names_follow_the_stricter_rule() {
    for name in ["a--b", "a-", "ab-", "a-b--c", "x\n"] {
        let env = Env::new();
        let before = env.listing();
        let error = env.error(&["create", name, "--no-git", "--json"]);
        assert!(
            error.starts_with(&format!("Brain name '{name}' is invalid.")),
            "{name:?}: {error}"
        );
        assert_eq!(env.listing(), before, "{name:?}");
    }
    for name in ["a", "a-b", "a1-2b-c3", &"a".repeat(48)] {
        let env = Env::new();
        let (code, doc) = env.json(&["create", name, "--no-git", "--json"]);
        assert_eq!(code, 0, "{name}: {doc}");
    }
}

#[test]
fn v2_names_keep_loading() {
    let env = Env::new();
    let brain = env.work.join("old");
    fs::create_dir_all(brain.join("projects")).unwrap();
    fs::write(
        brain.join("brain.md"),
        "---\nauto_generated: true\n---\n\n# a--b\n\nOld brain.\n",
    )
    .unwrap();
    fs::write(
        brain.join("kluris.yml"),
        "name: a--b\ndescription: Old brain.\n",
    )
    .unwrap();
    fs::create_dir_all(env.config().parent().unwrap()).unwrap();
    fs::write(
        env.config(),
        format!(
            "brains:\n  a--b:\n    path: '{}'\n    description: Old brain.\n",
            brain.display()
        ),
    )
    .unwrap();
    let (code, doc) = env.json(&["wake-up", "--brain", "a--b", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["name"], "a--b");
    let (code, doc) = env.json(&["remove", "a--b", "--json"]);
    assert_eq!(code, 0, "{doc}");
    // register keeps v2's rule: an existing brain whose brain.md names it
    // `a--b` registers under that name.
    let (code, doc) = env.json(&["register", "old", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["name"], "a--b");
}

// --- git failures -------------------------------------------------------------------------------

#[test]
fn a_failing_git_command_leaves_the_brain_unregistered() {
    let env = Env::new();
    let error = env.error(&["create", "demo", "--branch", "bad..name", "--json"]);
    let brain = env.work.join("demo");
    assert!(
        error.starts_with("`git checkout -b bad..name` failed (exit "),
        "{error}"
    );
    assert!(error.contains(&brain.display().to_string()), "{error}");
    assert!(error.contains("not registered"), "{error}");
    assert!(!env.config_text().contains("demo"), "{}", env.config_text());
    assert!(brain.join("brain.md").is_file(), "the scaffold is left");
    assert!(!env.home.join(".agents").exists(), "no skill installed");
}

#[test]
fn a_failing_commit_leaves_the_brain_unregistered() {
    let env = Env::new();
    let hooks = env.root.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        env.root.join("gitconfig"),
        format!(
            "[core]\n\thooksPath = {}\n",
            hooks.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let error = env.error(&["create", "demo", "--json"]);
    assert!(
        error.starts_with("`git commit -m brain: initialize demo` failed (exit "),
        "{error}"
    );
    assert!(error.contains("not registered"), "{error}");
    assert!(!env.config_text().contains("demo"));
    let brain_md = fs::read_to_string(env.work.join("demo").join("brain.md")).unwrap();
    assert!(
        brain_md.contains("updated: '"),
        "dream ran before the commit: {brain_md}"
    );
}

#[test]
fn a_target_that_is_a_file_is_an_error() {
    let env = Env::new();
    fs::write(env.work.join("demo"), "x").unwrap();
    let before = env.listing();
    let error = env.error(&["create", "demo", "--json"]);
    assert_eq!(
        error,
        format!(
            "{} already exists and is not a directory. Choose a different name or use --path to specify a location.",
            env.work.join("demo").display()
        )
    );
    assert_eq!(env.listing(), before);
}
