//! `kluris commit --brain X --paths a.md,b.md [-m MSG] [--json]`
//! (TEST-K3-43), with real git in temp dirs: only the given paths and the
//! `map.md`/`brain.md` files the map step regenerates because of them are
//! committed; every other dirty file stays as it was and is listed in
//! `left_dirty`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::{Value, json};

const GLOSSARY: &str = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n";

fn neuron(title: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# {title}\n"
    )
}

fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// The variables every git and kluris run gets: no user or system git
/// config, no identity or agent from the environment.
fn isolate(cmd: &mut StdCommand, dir: &Path) {
    cmd.env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("GIT_CONFIG_GLOBAL", dir.join("gitconfig-empty"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CEILING_DIRECTORIES", dir)
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("KLURIS_AGENT");
}

struct Brain {
    dir: TempDir,
    /// The git work tree.
    repo: PathBuf,
    /// The brain directory (the work tree, or a folder in it).
    root: PathBuf,
}

impl Brain {
    /// A brain at the root of its own repo, committed, with the identity
    /// `Ana Tester <ana@example.com>` in the repo's config.
    fn new() -> Brain {
        Brain::build(None, true)
    }

    /// The same brain at `brains/demo` inside a larger repo.
    fn nested() -> Brain {
        Brain::build(Some("brains/demo"), true)
    }

    /// A brain that is not a git repo.
    fn without_git() -> Brain {
        Brain::build(None, false)
    }

    fn build(sub: Option<&str>, git: bool) -> Brain {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig-empty"), "").unwrap();
        let repo = dir.path().join("work");
        let root = match sub {
            Some(sub) => sub.split('/').fold(repo.clone(), |p, part| p.join(part)),
            None => repo.clone(),
        };
        fs::create_dir_all(&root).unwrap();
        let brain = Brain { dir, repo, root };
        brain.write(".gitignore", "kluris.yml\nbrain-mri.html\n");
        brain.write("kluris.yml", "name: demo\ndescription: A demo brain\n");
        brain.write("glossary.md", GLOSSARY);
        brain.write("notes/first.md", &neuron("First"));
        brain.write("notes/keep.md", &neuron("Keep"));
        brain.write("ideas/idea.md", &neuron("Idea"));
        let config = format!(
            "brains:\n  demo:\n    path: {}\n    description: ''\n",
            yaml_path(&brain.root)
        );
        let kluris_dir = brain.dir.path().join(".kluris");
        fs::create_dir_all(&kluris_dir).unwrap();
        fs::write(kluris_dir.join("config.yml"), config).unwrap();
        let (code, out, err) = brain.run(&["dream", "--brain", "demo", "--json"], &[]);
        assert_eq!(code, 0, "{out}{err}");
        if git {
            brain.git_in(&brain.repo, &["init", "-q", "-b", "main"]);
            brain.git_in(&brain.repo, &["config", "core.autocrlf", "false"]);
            brain.git_in(&brain.repo, &["config", "user.name", "Ana Tester"]);
            brain.git_in(&brain.repo, &["config", "user.email", "ana@example.com"]);
            if sub.is_some() {
                fs::write(brain.repo.join("README.md"), "outer\n").unwrap();
            }
            brain.git_in(&brain.repo, &["add", "-A"]);
            brain.git_in(&brain.repo, &["commit", "-q", "-m", "init"]);
        }
        brain
    }

    fn path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.root.clone(), |p, part| p.join(part))
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    fn kluris(&self, env: &[(&str, &str)]) -> Command {
        let mut std = StdCommand::new(assert_cmd::cargo::cargo_bin!("kluris"));
        isolate(&mut std, self.dir.path());
        std.env(
            "KLURIS_CONFIG",
            self.dir.path().join(".kluris").join("config.yml"),
        )
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
        for (key, value) in env {
            std.env(key, value);
        }
        Command::from_std(std)
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let out = self.kluris(env).args(args).output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn json_env(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, Value) {
        let (code, stdout, stderr) = self.run(args, env);
        assert_eq!(stderr, "", "{stdout}");
        let doc = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"));
        (code, doc)
    }

    /// `kluris commit --brain demo --paths <paths> [extra] --json`.
    fn commit(&self, paths: &str, extra: &[&str]) -> (i32, Value) {
        self.commit_env(paths, extra, &[])
    }

    fn commit_env(&self, paths: &str, extra: &[&str], env: &[(&str, &str)]) -> (i32, Value) {
        let mut args = vec!["commit", "--brain", "demo", "--paths", paths];
        args.extend_from_slice(extra);
        args.push("--json");
        self.json_env(&args, env)
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = StdCommand::new("git");
        isolate(&mut cmd, self.dir.path());
        let out = cmd
            .args(["-c", "core.quotePath=false"])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// git, run in the brain directory.
    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    fn git_ok(&self, args: &[&str]) -> bool {
        let mut cmd = StdCommand::new("git");
        isolate(&mut cmd, self.dir.path());
        cmd.args(args)
            .current_dir(&self.root)
            .output()
            .unwrap()
            .status
            .success()
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The brain-relative paths a commit touched, sorted.
    fn committed(&self, rev: &str) -> Vec<String> {
        lines(&self.git(&["show", "--relative", "--name-only", "--format=", rev]))
    }

    /// `git show --name-status` of a commit, brain-relative.
    fn name_status(&self, rev: &str) -> Vec<String> {
        lines(&self.git(&["show", "--relative", "--name-status", "--format=", rev]))
    }

    /// The brain-relative paths staged in the user's index, sorted.
    fn staged(&self) -> Vec<String> {
        lines(&self.git(&["diff", "--cached", "--relative", "--name-only"]))
    }

    /// `git status --porcelain` of the whole work tree, sorted lines.
    fn status(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .git_in(
                &self.repo,
                &["status", "--porcelain", "--untracked-files=all"],
            )
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        lines
    }

    fn message(&self) -> String {
        self.git(&["log", "-1", "--format=%B"])
    }
}

fn lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    lines.sort();
    lines
}

fn keys(doc: &Value) -> Vec<String> {
    doc.as_object().unwrap().keys().cloned().collect()
}

// --- what it stages ------------------------------------------------------------------

#[test]
fn commits_only_the_given_paths_and_the_maps_they_change() {
    let b = Brain::new();
    let before = b.head();
    b.write("notes/second.md", &neuron("Second"));
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: changed\n"));
    b.write("ideas/draft.md", &neuron("Draft"));
    let ideas_map = b.read("ideas/map.md");

    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        keys(&doc),
        [
            "ok",
            "brain",
            "committed",
            "commit",
            "paths",
            "regenerated",
            "left_dirty"
        ]
    );
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["brain"], "demo");
    assert_eq!(doc["committed"], true);
    let head = b.head();
    assert_ne!(head, before);
    assert_eq!(doc["commit"], head.as_str());
    assert_eq!(doc["paths"], json!(["notes/second.md"]));
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["left_dirty"], json!(["glossary.md", "ideas/draft.md"]));

    assert_eq!(b.committed("HEAD"), ["notes/map.md", "notes/second.md"]);
    assert_eq!(b.git(&["rev-parse", "HEAD^"]).trim(), before);
    assert!(
        b.read("notes/map.md")
            .contains("- [second.md](./second.md) — Second")
    );
    // A map the paths do not change is not written.
    assert_eq!(b.read("ideas/map.md"), ideas_map);
    assert_eq!(b.status(), [" M glossary.md", "?? ideas/draft.md"]);
}

#[test]
fn a_file_staged_beforehand_stays_staged_and_out_of_the_commit() {
    let b = Brain::new();
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: staged\n"));
    b.git(&["add", "glossary.md"]);
    b.write("notes/second.md", &neuron("Second"));

    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(b.committed("HEAD"), ["notes/map.md", "notes/second.md"]);
    assert_eq!(b.staged(), ["glossary.md"]);
    assert_eq!(doc["left_dirty"], json!(["glossary.md"]));
    assert_eq!(b.status(), ["M  glossary.md"]);
}

#[test]
fn maps_a_dream_already_regenerated_are_committed() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    let (code, out, err) = b.run(&["dream", "--brain", "demo", "--json"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(b.read("notes/map.md").contains("second.md"));

    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(b.committed("HEAD"), ["notes/map.md", "notes/second.md"]);
    // The dream also refreshed the git dates of the committed neurons: not
    // given, so left as they are.
    let refreshed = ["ideas/idea.md", "notes/first.md", "notes/keep.md"];
    assert_eq!(doc["left_dirty"], json!(refreshed));
    assert_eq!(b.status(), refreshed.map(|rel| format!(" M {rel}")));
}

#[test]
fn a_changed_neuron_without_map_changes_commits_alone() {
    let b = Brain::new();
    let text = b
        .read("notes/first.md")
        .replace("# First", "# First\n\nMore.");
    b.write("notes/first.md", &text);
    let (code, doc) = b.commit("notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!([]));
    assert_eq!(b.committed("HEAD"), ["notes/first.md"]);
}

#[test]
fn paths_are_deduplicated_and_trimmed() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    b.write("notes/third.md", &neuron("Third"));
    let (code, doc) = b.commit("notes/second.md, notes/third.md,notes/second.md,", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!(["notes/second.md", "notes/third.md"]));
    assert_eq!(
        b.committed("HEAD"),
        ["notes/map.md", "notes/second.md", "notes/third.md"]
    );
}

#[test]
fn no_paths_is_an_error() {
    let b = Brain::new();
    let before = b.head();
    let (code, doc) = b.commit(" , ", &[]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["ok"], false);
    assert!(doc["error"].as_str().unwrap().contains("--paths"), "{doc}");
    assert_eq!(b.head(), before);
}

// --- paths it refuses ----------------------------------------------------------------

/// Each refused path, given beside a good one: exit 1, nothing staged,
/// committed or written.
fn assert_refused(b: &Brain, bad: &str) {
    let before = b.head();
    let map = b.read("notes/map.md");
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit(&format!("notes/second.md,{bad}"), &[]);
    assert_eq!(code, 1, "{bad}: {doc}");
    assert_eq!(keys(&doc), ["ok", "error"], "{bad}");
    assert_eq!(doc["ok"], false);
    let error = doc["error"].as_str().unwrap();
    assert!(error.contains(bad), "{bad}: {error}");
    assert_eq!(b.head(), before, "{bad}");
    assert_eq!(b.staged(), Vec::<String>::new(), "{bad}");
    assert_eq!(b.read("notes/map.md"), map, "{bad}: a map was written");
}

#[test]
fn refuses_paths_outside_the_brain() {
    let b = Brain::new();
    fs::write(b.dir.path().join("outside.md"), neuron("Outside")).unwrap();
    for bad in ["../outside.md", "notes/../../outside.md", "notes/../.."] {
        assert_refused(&b, bad);
    }
}

#[test]
fn refuses_absolute_paths() {
    let b = Brain::new();
    let outside = b.dir.path().join("outside.md");
    fs::write(&outside, neuron("Outside")).unwrap();
    assert_refused(&b, &outside.display().to_string());
    // An absolute path into the brain is refused too: paths are brain-relative.
    assert_refused(&b, &b.path("notes/first.md").display().to_string());
}

#[test]
fn refuses_git_internals_directories_and_missing_files() {
    let b = Brain::new();
    for bad in [".git/config", "notes", "notes/missing.md"] {
        assert_refused(&b, bad);
    }
}

#[cfg(unix)]
#[test]
fn refuses_symlinks_that_point_outside() {
    let b = Brain::new();
    let outside = b.dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("x.md"), neuron("X")).unwrap();
    std::os::unix::fs::symlink(outside.join("x.md"), b.path("notes/link.md")).unwrap();
    std::os::unix::fs::symlink(&outside, b.path("notes/ext")).unwrap();
    for bad in ["notes/link.md", "notes/ext/x.md"] {
        assert_refused(&b, bad);
    }
}

#[test]
fn refuses_a_path_outside_a_nested_brain_but_inside_its_repo() {
    let b = Brain::nested();
    fs::write(b.repo.join("README.md"), "changed\n").unwrap();
    assert_refused(&b, "../../README.md");
}

// --- message, trailers, author -------------------------------------------------------

#[test]
fn the_default_message_names_the_paths_and_the_approver() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.message(),
        "kluris: update notes/second.md\n\nKluris-Approved-By: Ana Tester\n\n"
    );
    assert_eq!(
        b.git(&["log", "-1", "--format=%an <%ae>|%cn <%ce>"]).trim(),
        "Ana Tester <ana@example.com>|Ana Tester <ana@example.com>"
    );
}

#[test]
fn kluris_agent_adds_a_co_author_trailer() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit_env(
        "notes/second.md",
        &[],
        &[("KLURIS_AGENT", "Claude Opus <noreply@anthropic.com>")],
    );
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.message(),
        "kluris: update notes/second.md\n\nKluris-Approved-By: Ana Tester\nCo-Authored-By: Claude Opus <noreply@anthropic.com>\n\n"
    );
    let trailers = b.git(&["log", "-1", "--format=%(trailers:only,unfold)"]);
    assert_eq!(
        trailers.trim(),
        "Kluris-Approved-By: Ana Tester\nCo-Authored-By: Claude Opus <noreply@anthropic.com>"
    );
}

#[test]
fn an_empty_kluris_agent_adds_no_trailer() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit_env("notes/second.md", &[], &[("KLURIS_AGENT", "  ")]);
    assert_eq!(code, 0, "{doc}");
    assert!(!b.message().contains("Co-Authored-By"), "{}", b.message());
}

#[test]
fn a_given_message_is_kept_and_gets_the_trailers() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &["-m", "Add the second note"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.message(),
        "Add the second note\n\nKluris-Approved-By: Ana Tester\n\n"
    );
    b.write("notes/third.md", &neuron("Third"));
    let (code, doc) = b.commit("notes/third.md", &["--message", "Third\n\nWith a body.\n"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.message(),
        "Third\n\nWith a body.\n\nKluris-Approved-By: Ana Tester\n\n"
    );
}

#[test]
fn a_long_default_message_is_shortened_and_lists_every_path_in_the_body() {
    let b = Brain::new();
    let paths: Vec<String> = (1..=8)
        .map(|n| format!("notes/neuron-number-{n:02}.md"))
        .collect();
    for path in &paths {
        b.write(path, &neuron(path));
    }
    let (code, doc) = b.commit(&paths.join(","), &[]);
    assert_eq!(code, 0, "{doc}");
    let message = b.message();
    let subject = message.lines().next().unwrap();
    assert_eq!(
        subject,
        "kluris: update notes/neuron-number-01.md and 7 more"
    );
    let mut expected = format!("{subject}\n\n");
    for path in &paths {
        expected.push_str(&format!("{path}\n"));
    }
    expected.push_str("\nKluris-Approved-By: Ana Tester\n\n");
    assert_eq!(message, expected);
}

#[test]
fn without_a_git_identity_the_fallback_authors_and_approves() {
    let b = Brain::new();
    b.git(&["config", "--unset", "user.name"]);
    b.git(&["config", "--unset", "user.email"]);
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.git(&["log", "-1", "--format=%an <%ae>|%cn <%ce>"]).trim(),
        "kluris <kluris@local>|kluris <kluris@local>"
    );
    assert!(b.message().contains("\nKluris-Approved-By: kluris\n"));
}

// --- nothing to commit, not a repo, mid-merge ----------------------------------------

#[test]
fn nothing_to_commit_exits_0_with_committed_false() {
    let b = Brain::new();
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: other\n"));
    let before = b.head();
    let (code, doc) = b.commit("notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc,
        json!({
            "ok": true,
            "brain": "demo",
            "committed": false,
            "commit": null,
            "paths": ["notes/first.md"],
            "regenerated": [],
            "left_dirty": ["glossary.md"],
        })
    );
    assert_eq!(b.head(), before);
}

#[test]
fn a_brain_that_is_not_a_git_repo_is_a_clean_error() {
    let b = Brain::without_git();
    b.write("notes/second.md", &neuron("Second"));
    let map = b.read("notes/map.md");
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("is not a git repository"),
        "{doc}"
    );
    assert_eq!(b.read("notes/map.md"), map);
    let (code, out, err) = b.run(
        &["commit", "--brain", "demo", "--paths", "notes/second.md"],
        &[],
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.starts_with("Error: ") && err.contains("is not a git repository"),
        "{err}"
    );
}

/// Two branches that change `glossary.md` differently.
fn diverged(b: &Brain) {
    b.git(&["checkout", "-q", "-b", "other"]);
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: theirs\n"));
    b.git(&["commit", "-q", "-am", "theirs"]);
    b.git(&["checkout", "-q", "main"]);
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: ours\n"));
    b.git(&["commit", "-q", "-am", "ours"]);
}

#[test]
fn refuses_to_commit_in_the_middle_of_a_merge() {
    let b = Brain::new();
    diverged(&b);
    assert!(!b.git_ok(&["merge", "-q", "other"]));
    let before = b.head();
    b.write("notes/second.md", &neuron("Second"));
    let status = b.status();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert!(doc["error"].as_str().unwrap().contains("merge"), "{doc}");
    assert_eq!(b.head(), before);
    assert!(b.git_ok(&["rev-parse", "-q", "--verify", "MERGE_HEAD"]));
    assert_eq!(b.status(), status);
}

#[test]
fn refuses_to_commit_in_the_middle_of_a_rebase() {
    let b = Brain::new();
    diverged(&b);
    assert!(!b.git_ok(&["rebase", "other"]));
    let before = b.head();
    b.write("notes/second.md", &neuron("Second"));
    let status = b.status();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert!(doc["error"].as_str().unwrap().contains("rebase"), "{doc}");
    assert_eq!(b.head(), before);
    assert_eq!(b.status(), status);
}

// --- names, nested lobes, deletions, nested brains -----------------------------------

#[test]
fn non_ascii_names_and_a_new_top_level_lobe() {
    let b = Brain::new();
    b.write("notes/café ünï.md", &neuron("Café"));
    b.write("été/naïve.md", &neuron("Naïve"));
    let (code, doc) = b.commit("notes/café ünï.md,été/naïve.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!(["notes/café ünï.md", "été/naïve.md"]));
    // A new lobe: its map, brain.md, and the sideways links of its siblings.
    assert_eq!(
        doc["regenerated"],
        json!(["brain.md", "ideas/map.md", "notes/map.md", "été/map.md"])
    );
    assert_eq!(
        b.committed("HEAD"),
        [
            "brain.md",
            "ideas/map.md",
            "notes/café ünï.md",
            "notes/map.md",
            "été/map.md",
            "été/naïve.md"
        ]
    );
    assert!(b.read("brain.md").contains("[été/](./été/map.md)"));
    assert!(b.read("ideas/map.md").contains("[été](../été/map.md)"));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_nested_lobe_regenerates_the_maps_above_it() {
    let b = Brain::new();
    let brain_md = b.read("brain.md");
    b.write("notes/deep/inner/x.md", &neuron("X"));
    let (code, doc) = b.commit("notes/deep/inner/x.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["regenerated"],
        json!([
            "notes/deep/inner/map.md",
            "notes/deep/map.md",
            "notes/map.md"
        ])
    );
    assert_eq!(
        b.committed("HEAD"),
        [
            "notes/deep/inner/map.md",
            "notes/deep/inner/x.md",
            "notes/deep/map.md",
            "notes/map.md"
        ]
    );
    assert!(b.read("notes/map.md").contains("- [deep/](./deep/map.md)"));
    assert_eq!(b.read("brain.md"), brain_md);
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_deleted_neuron_is_committed_with_its_map() {
    let b = Brain::new();
    fs::remove_file(b.path("notes/first.md")).unwrap();
    let (code, doc) = b.commit("notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(
        b.name_status("HEAD"),
        ["D\tnotes/first.md", "M\tnotes/map.md"]
    );
    assert!(!b.read("notes/map.md").contains("first.md"));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_deleted_lobe_takes_its_map_and_updates_brain_md_and_siblings() {
    let b = Brain::new();
    fs::remove_dir_all(b.path("ideas")).unwrap();
    let (code, doc) = b.commit("ideas/idea.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["regenerated"],
        json!(["brain.md", "ideas/map.md", "notes/map.md"])
    );
    assert_eq!(
        b.name_status("HEAD"),
        [
            "D\tideas/idea.md",
            "D\tideas/map.md",
            "M\tbrain.md",
            "M\tnotes/map.md"
        ]
    );
    assert!(!b.read("brain.md").contains("ideas"));
    assert!(!b.read("notes/map.md").contains("ideas"));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_deleted_lobe_takes_the_maps_of_its_empty_sub_lobes_too() {
    let b = Brain::new();
    fs::create_dir_all(b.path("ideas/sub")).unwrap();
    let (code, out, err) = b.run(&["dream", "--brain", "demo", "--json"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    b.git(&["add", "-A"]);
    b.git(&["commit", "-q", "-m", "an empty sub-lobe"]);
    assert!(
        b.committed("HEAD")
            .contains(&"ideas/sub/map.md".to_string())
    );

    fs::remove_dir_all(b.path("ideas")).unwrap();
    let (code, doc) = b.commit("ideas/idea.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        doc["regenerated"],
        json!([
            "brain.md",
            "ideas/map.md",
            "ideas/sub/map.md",
            "notes/map.md"
        ])
    );
    assert!(
        b.name_status("HEAD")
            .contains(&"D\tideas/sub/map.md".to_string()),
        "{:?}",
        b.name_status("HEAD")
    );
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn the_first_commit_of_a_repo_with_no_commit_yet() {
    let b = Brain::without_git();
    b.git(&["init", "-q", "-b", "main"]);
    b.git(&["config", "core.autocrlf", "false"]);
    b.git(&["config", "user.name", "Ana Tester"]);
    b.git(&["config", "user.email", "ana@example.com"]);
    let (code, doc) = b.commit("notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true);
    assert_eq!(doc["commit"], b.head().as_str());
    // The lobe is new: its map and brain.md. The uncommitted `ideas` lobe
    // is not in the commit, so no committed map names it.
    assert_eq!(doc["regenerated"], json!(["brain.md", "notes/map.md"]));
    assert_eq!(
        b.committed("HEAD"),
        ["brain.md", "notes/first.md", "notes/map.md"]
    );
    assert!(!show(&b, "HEAD", "brain.md").contains("ideas"));
    assert!(!show(&b, "HEAD", "notes/map.md").contains("keep.md"));
    assert!(!b.git_ok(&["rev-parse", "-q", "--verify", "HEAD^"]));
    // Untracked work-tree maps had changes of their own: left as they are.
    assert_eq!(
        doc["left_dirty"],
        json!([
            ".gitignore",
            "brain.md",
            "glossary.md",
            "ideas/idea.md",
            "ideas/map.md",
            "notes/keep.md",
            "notes/map.md"
        ])
    );
    assert_eq!(b.staged(), Vec::<String>::new());
}

#[test]
fn a_map_that_cannot_be_regenerated_stops_the_commit() {
    let b = Brain::new();
    // Committed: the maps are built from the commit, not the work tree.
    let bad = "---\ntitle: Foo: bar\n---\n\n# Notes\n";
    b.write("notes/map.md", bad);
    b.git(&["commit", "-q", "-am", "a map that does not parse"]);
    b.write("notes/second.md", &neuron("Second"));
    let before = b.head();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(error.contains("cannot regenerate notes/map.md"), "{error}");
    assert_eq!(b.head(), before);
    assert_eq!(b.staged(), Vec::<String>::new());
    assert_eq!(b.read("notes/map.md"), bad);
}

#[test]
fn a_brain_inside_a_larger_repo() {
    let b = Brain::nested();
    fs::write(b.repo.join("README.md"), "changed\n").unwrap();
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!(["notes/second.md"]));
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    // left_dirty is the brain's own; the outer repo's files are not listed.
    assert_eq!(doc["left_dirty"], json!([]));
    assert_eq!(
        lines(&b.git_in(&b.repo, &["show", "--name-only", "--format=", "HEAD"])),
        ["brains/demo/notes/map.md", "brains/demo/notes/second.md"]
    );
    assert_eq!(b.status(), [" M README.md"]);
}

#[test]
fn file_names_with_pathspec_characters_are_taken_literally() {
    let b = Brain::new();
    // As a glob, `[draft] a.md` would also match `d a.md`.
    b.write("notes/[draft] a.md", &neuron("Draft"));
    b.write("notes/d a.md", &neuron("Not me"));
    let (code, doc) = b.commit("notes/[draft] a.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(b.committed("HEAD"), ["notes/[draft] a.md", "notes/map.md"]);
    assert_eq!(doc["left_dirty"], json!(["notes/d a.md"]));
}

#[cfg(windows)]
#[test]
fn windows_paths_with_backslashes() {
    let b = Brain::new();
    b.write("notes/deep/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes\\deep\\second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["paths"], json!(["notes/deep/second.md"]));
    assert_eq!(
        doc["regenerated"],
        json!(["notes/deep/map.md", "notes/map.md"])
    );
    for bad in [
        "C:\\outside.md",
        "\\outside.md",
        "C:outside.md",
        "..\\outside.md",
    ] {
        assert_refused(&b, bad);
    }
}

// --- the command surface -------------------------------------------------------------

#[test]
fn human_output() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: other\n"));
    let (code, out, err) = b.run(
        &["commit", "--brain", "demo", "--paths", "notes/second.md"],
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(err, "");
    let short = &b.head()[..7];
    assert_eq!(
        out,
        format!(
            "Committed {short} in demo: kluris: update notes/second.md\n  notes/second.md\n  notes/map.md (regenerated)\nLeft uncommitted: glossary.md\n"
        )
    );
    let (code, out, _) = b.run(
        &["commit", "--brain", "demo", "--paths", "notes/second.md"],
        &[],
    );
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "Nothing to commit in demo.\nLeft uncommitted: glossary.md\n"
    );
}

#[test]
fn brain_all_is_refused() {
    let b = Brain::new();
    let (code, doc) = b.json_env(
        &[
            "commit",
            "--brain",
            "all",
            "--paths",
            "notes/first.md",
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 1);
    assert_eq!(doc["ok"], false);
}

#[test]
fn commit_is_in_help() {
    let b = Brain::new();
    let (code, doc) = b.json_env(&["help", "--json"], &[]);
    assert_eq!(code, 0);
    let commands = doc["commands"].as_array().unwrap();
    let at = commands
        .iter()
        .position(|c| c["name"] == "commit")
        .expect("commit in help");
    assert_eq!(commands[at - 1]["name"], "help");
    assert_eq!(
        &commands[at],
        &json!({
            "name": "commit",
            "description": "Commit approved brain files and the maps they change (--paths)"
        })
    );
    let (code, out, _) = b.run(&["--help"], &[]);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l.starts_with("  commit ")), "{out}");
    let (code, out, _) = b.run(&["commit", "--help"], &[]);
    assert_eq!(code, 0);
    for text in [
        "Usage: kluris commit [OPTIONS]",
        "--paths",
        "-m, --message",
        "--no-verify",
        "--brain",
        "--json",
    ] {
        assert!(out.contains(text), "{text}: {out}");
    }
}

// --- the committed maps come from the commit, not the work tree ----------------------

/// A file at `rev`, as text.
fn show(b: &Brain, rev: &str, rel: &str) -> String {
    b.git(&["show", &format!("{rev}:./{rel}")])
}

#[test]
fn an_uncommitted_draft_never_reaches_the_committed_map() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    b.write("notes/draft.md", &neuron("Secret draft"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(map.contains("- [second.md](./second.md) — Second"), "{map}");
    assert!(!map.contains("draft") && !map.contains("Secret"), "{map}");
    // The work tree's map was clean: it gets the committed bytes.
    assert_eq!(b.read("notes/map.md"), map);
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["left_dirty"], json!(["notes/draft.md"]));
    assert_eq!(b.status(), ["?? notes/draft.md"]);
}

#[test]
fn a_dirty_tracked_neuron_keeps_its_committed_title_in_the_map() {
    let b = Brain::new();
    let renamed = b
        .read("notes/first.md")
        .replace("# First", "# First Renamed");
    b.write("notes/first.md", &renamed);
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(map.contains("- [first.md](./first.md) — First\n"), "{map}");
    assert!(!map.contains("Renamed"), "{map}");
    assert_eq!(doc["left_dirty"], json!(["notes/first.md"]));
    assert_eq!(b.status(), [" M notes/first.md"]);
}

#[test]
fn an_ignored_file_is_not_in_the_committed_map() {
    let b = Brain::new();
    b.write(
        ".gitignore",
        "kluris.yml\nbrain-mri.html\nnotes/private.md\n",
    );
    b.git(&["commit", "-q", "-am", "ignore a private note"]);
    b.write("notes/private.md", &neuron("Private"));
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(!map.contains("private"), "{map}");
    assert_eq!(b.read("notes/map.md"), map);
    assert_eq!(doc["left_dirty"], json!([]));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_map_dream_rewrote_with_a_draft_is_left_alone_and_reported() {
    let b = Brain::new();
    b.write("notes/second.md", &neuron("Second"));
    b.write("notes/draft.md", &neuron("Secret draft"));
    let (code, out, err) = b.run(&["dream", "--brain", "demo", "--json"], &[]);
    assert_eq!(code, 0, "{out}{err}");
    let dreamt = b.read("notes/map.md");
    assert!(dreamt.contains("Secret draft"));

    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(map.contains("second.md") && !map.contains("draft"), "{map}");
    assert_eq!(b.read("notes/map.md"), dreamt, "a dirty map was rewritten");
    let left: Vec<String> = doc["left_dirty"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(left.contains(&"notes/map.md".to_string()), "{left:?}");
    assert!(left.contains(&"notes/draft.md".to_string()), "{left:?}");
    assert!(b.status().contains(&" M notes/map.md".to_string()));
}

#[test]
fn a_clean_map_with_crlf_endings_keeps_them() {
    let b = Brain::new();
    let crlf = b.read("notes/map.md").replace('\n', "\r\n");
    b.write("notes/map.md", &crlf);
    b.git(&["commit", "-q", "-am", "crlf map"]);
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    let map = b.read("notes/map.md");
    assert!(map.contains("second.md"), "{map}");
    assert_eq!(
        map.matches('\n').count(),
        map.matches("\r\n").count(),
        "{map:?}"
    );
    assert_eq!(b.status(), Vec::<String>::new());
}

// --- hooks ---------------------------------------------------------------------------

/// A hook script (`#!/bin/sh`; Git for Windows runs it with its sh).
fn hook(dir: &Path, name: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Brain {
    fn hooks(&self) -> PathBuf {
        self.repo.join(".git").join("hooks")
    }

    fn out_file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

#[test]
fn a_passing_pre_commit_hook_sees_what_is_committed() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "pre-commit",
        "git diff --cached --name-only > \"$HOOK_OUT\"\necho 'scanned: clean'",
    );
    b.write("notes/second.md", &neuron("Second"));
    b.write("glossary.md", &format!("{GLOSSARY}\n- term: other\n"));
    let out_file = b.out_file("staged.txt");
    let out = out_file.display().to_string();
    let (code, stdout, stderr) = b.run(
        &[
            "commit",
            "--brain",
            "demo",
            "--paths",
            "notes/second.md",
            "--json",
        ],
        &[("HOOK_OUT", &out)],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    // The hook's output goes to stderr; stdout stays one JSON document.
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["committed"], true);
    assert!(stderr.contains("scanned: clean"), "{stderr}");
    let staged = fs::read_to_string(&out_file).unwrap();
    assert_eq!(lines(&staged), ["notes/map.md", "notes/second.md"]);
}

#[test]
fn a_failing_pre_commit_hook_stops_the_commit() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "pre-commit",
        "echo 'secret found in notes/second.md' >&2\nexit 1",
    );
    b.write("notes/second.md", &neuron("Second"));
    let before = b.head();
    let map = b.read("notes/map.md");
    let status = b.status();
    let (code, stdout, stderr) = b.run(
        &["commit", "--brain", "demo", "--paths", "notes/second.md"],
        &[],
    );
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stderr.contains("secret found in notes/second.md"),
        "{stderr}"
    );
    assert!(stderr.contains("pre-commit"), "{stderr}");
    assert_eq!(b.head(), before);
    assert_eq!(b.staged(), Vec::<String>::new());
    assert_eq!(b.read("notes/map.md"), map);
    assert_eq!(b.status(), status);

    let (code, stdout, _) = b.run(
        &[
            "commit",
            "--brain",
            "demo",
            "--paths",
            "notes/second.md",
            "--json",
        ],
        &[],
    );
    assert_eq!(code, 1);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"].as_str().unwrap().contains("pre-commit"),
        "{doc}"
    );
    assert_eq!(b.head(), before);
}

#[test]
fn the_message_hooks_shape_the_message() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "prepare-commit-msg",
        "echo 'Prepared: yes' >> \"$1\"",
    );
    hook(
        &b.hooks(),
        "commit-msg",
        "sed 's/^kluris: update/brain: update/' \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"",
    );
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        b.message(),
        "brain: update notes/second.md\n\nKluris-Approved-By: Ana Tester\nPrepared: yes\n\n"
    );
}

#[test]
fn a_failing_commit_msg_hook_stops_the_commit() {
    let b = Brain::new();
    hook(&b.hooks(), "commit-msg", "echo 'bad message' >&2\nexit 1");
    b.write("notes/second.md", &neuron("Second"));
    let before = b.head();
    let map = b.read("notes/map.md");
    let (code, stdout, stderr) = b.run(
        &["commit", "--brain", "demo", "--paths", "notes/second.md"],
        &[],
    );
    assert_eq!(code, 1, "{stdout}");
    assert!(
        stderr.contains("bad message") && stderr.contains("commit-msg"),
        "{stderr}"
    );
    assert_eq!(b.head(), before);
    assert_eq!(b.staged(), Vec::<String>::new());
    assert_eq!(b.read("notes/map.md"), map);
}

#[test]
fn core_hooks_path_is_honoured() {
    let b = Brain::new();
    hook(&b.hooks(), "pre-commit", "exit 1");
    let custom = b.dir.path().join("team-hooks");
    hook(&custom, "pre-commit", "echo ran > \"$HOOK_OUT\"");
    b.git(&["config", "core.hooksPath", custom.to_str().unwrap()]);
    b.write("notes/second.md", &neuron("Second"));
    let out_file = b.out_file("ran.txt");
    let out = out_file.display().to_string();
    let (code, doc) = b.commit_env("notes/second.md", &[], &[("HOOK_OUT", &out)]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(fs::read_to_string(&out_file).unwrap().trim(), "ran");
}

#[test]
fn no_verify_skips_pre_commit_and_commit_msg() {
    let b = Brain::new();
    hook(&b.hooks(), "pre-commit", "exit 1");
    hook(&b.hooks(), "commit-msg", "exit 1");
    hook(
        &b.hooks(),
        "prepare-commit-msg",
        "echo 'Prepared: yes' >> \"$1\"",
    );
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &["--no-verify"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true);
    assert!(b.message().contains("\nPrepared: yes\n"), "{}", b.message());
}

#[test]
fn post_commit_runs_after_the_commit_lands() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "post-commit",
        "git rev-parse HEAD > \"$HOOK_OUT\"\nexit 1",
    );
    b.write("notes/second.md", &neuron("Second"));
    let out_file = b.out_file("head.txt");
    let out = out_file.display().to_string();
    let (code, stdout, _) = b.run(
        &[
            "commit",
            "--brain",
            "demo",
            "--paths",
            "notes/second.md",
            "--json",
        ],
        &[("HOOK_OUT", &out)],
    );
    // A failing post-commit hook does not undo or fail the commit (git's rule).
    assert_eq!(code, 0, "{stdout}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["commit"], b.head().as_str());
    assert_eq!(fs::read_to_string(&out_file).unwrap().trim(), b.head());
}

/// A refused commit left everything as it was: HEAD, the user's index, the
/// work tree.
fn assert_untouched(b: &Brain, before: &str, status: &[String]) {
    assert_eq!(b.head(), before);
    assert_eq!(b.staged(), Vec::<String>::new());
    assert_eq!(b.status(), status);
}

#[test]
fn a_hook_that_stages_an_unapproved_file_stops_the_commit() {
    let b = Brain::new();
    hook(&b.hooks(), "pre-commit", "git add -A");
    b.write("notes/second.md", &neuron("Second"));
    b.write("notes/private.md", &neuron("UNAPPROVED_SECRET"));
    let before = b.head();
    let status = b.status();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(keys(&doc), ["ok", "error"]);
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("a hook staged files you didn't approve: notes/private.md")
            && error.contains("nothing was committed"),
        "{error}"
    );
    assert_untouched(&b, &before, &status);
    assert!(!b.git_ok(&["cat-file", "-e", "HEAD:notes/private.md"]));
    assert!(status.contains(&"?? notes/private.md".to_string()));
}

#[test]
fn a_hook_that_unstages_a_tracked_file_stops_the_commit() {
    let b = Brain::new();
    hook(&b.hooks(), "pre-commit", "git rm -q --cached notes/keep.md");
    b.write("notes/second.md", &neuron("Second"));
    let before = b.head();
    let status = b.status();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("a hook staged files you didn't approve: notes/keep.md"),
        "{error}"
    );
    assert_untouched(&b, &before, &status);
}

#[test]
fn a_hook_that_stages_a_file_outside_a_nested_brain_stops_the_commit() {
    let b = Brain::nested();
    hook(&b.hooks(), "pre-commit", "git add -A");
    fs::write(b.repo.join("README.md"), "OUTSIDE_CHANGE\n").unwrap();
    b.write("notes/second.md", &neuron("Second"));
    let before = b.head();
    let status = b.status();
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 1, "{doc}");
    let error = doc["error"].as_str().unwrap();
    assert!(
        error.contains("a hook staged files you didn't approve: ../../README.md"),
        "{error}"
    );
    assert_untouched(&b, &before, &status);
}

#[test]
fn a_hook_may_reformat_an_approved_file() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "pre-commit",
        "printf '\\nFORMATTED\\n' >> notes/second.md && git add notes/second.md",
    );
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true);
    assert!(show(&b, "HEAD", "notes/second.md").ends_with("FORMATTED\n"));
    assert_eq!(b.committed("HEAD"), ["notes/map.md", "notes/second.md"]);
}

#[test]
fn a_map_edited_while_the_hooks_run_is_kept_and_reported() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "pre-commit",
        "printf 'USER_MAP_EDIT_DURING_HOOK\\n' > notes/map.md",
    );
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["left_dirty"], json!(["notes/map.md"]));
    assert_eq!(b.read("notes/map.md"), "USER_MAP_EDIT_DURING_HOOK\n");
    assert!(show(&b, "HEAD", "notes/map.md").contains("second.md"));
}

#[cfg(unix)]
#[test]
fn a_linked_map_is_regenerated_through_its_target() {
    let b = Brain::new();
    let original = b.read("notes/map.md");
    b.write("maps/base.md", &original);
    fs::remove_file(b.path("notes/map.md")).unwrap();
    std::os::unix::fs::symlink("../maps/base.md", b.path("notes/map.md")).unwrap();
    b.git(&["add", "-A"]);
    b.git(&["commit", "-q", "-m", "linked map"]);
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["maps/base.md"]));
    assert_eq!(doc["left_dirty"], json!([]));
    assert_eq!(b.committed("HEAD"), ["maps/base.md", "notes/second.md"]);
    assert!(
        b.git(&["ls-tree", "HEAD", "notes/map.md"])
            .starts_with("120000 ")
    );
    assert!(show(&b, "HEAD", "maps/base.md").contains("second.md"));
    assert_eq!(
        fs::read_link(b.path("notes/map.md")).unwrap(),
        Path::new("../maps/base.md")
    );
    assert!(b.read("maps/base.md").contains("second.md"));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_hook_that_stages_the_stale_work_tree_map_still_commits_kluris_map() {
    let b = Brain::new();
    hook(&b.hooks(), "pre-commit", "git add -A");
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(doc["left_dirty"], json!([]));
    assert_eq!(b.committed("HEAD"), ["notes/map.md", "notes/second.md"]);
    assert!(show(&b, "HEAD", "notes/map.md").contains("second.md"));
    assert_eq!(b.read("notes/map.md"), show(&b, "HEAD", "notes/map.md"));
    assert_eq!(b.status(), Vec::<String>::new());
}

#[test]
fn a_hook_that_rewrites_a_map_never_reaches_the_commit() {
    let b = Brain::new();
    hook(
        &b.hooks(),
        "pre-commit",
        "printf 'HOOK_MAP\\n' > notes/map.md && git add notes/map.md",
    );
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(
        map.contains("second.md") && !map.contains("HOOK_MAP"),
        "{map}"
    );
    // The hook's work-tree edit is its own: kept, and left dirty.
    assert_eq!(b.read("notes/map.md"), "HOOK_MAP\n");
    assert_eq!(doc["left_dirty"], json!(["notes/map.md"]));
}

/// A formatter hook that renames `title` to `FORMATTED title` in `rel`.
fn title_formatter(b: &Brain, rel: &str, title: &str) {
    hook(
        &b.hooks(),
        "pre-commit",
        &format!(
            "sed 's/^# {title}$/# FORMATTED {title}/' {rel} > {rel}.tmp && mv {rel}.tmp {rel} && git add {rel}"
        ),
    );
}

#[test]
fn the_committed_map_shows_a_title_a_hook_rewrote() {
    let b = Brain::new();
    title_formatter(&b, "notes/second.md", "Second");
    b.write("notes/second.md", &neuron("Second"));
    let (code, doc) = b.commit("notes/second.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert!(show(&b, "HEAD", "notes/second.md").contains("# FORMATTED Second"));
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(map.contains("FORMATTED Second"), "{map}");
    assert_eq!(b.read("notes/map.md"), map);
}

#[test]
fn a_hook_that_retitles_a_neuron_whose_map_had_not_changed_updates_the_map() {
    let b = Brain::new();
    title_formatter(&b, "notes/first.md", "First");
    b.write(
        "notes/first.md",
        &format!("{}\nMore text.\n", neuron("First")),
    );
    let (code, doc) = b.commit("notes/first.md", &[]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["regenerated"], json!(["notes/map.md"]));
    assert_eq!(b.committed("HEAD"), ["notes/first.md", "notes/map.md"]);
    let map = show(&b, "HEAD", "notes/map.md");
    assert!(map.contains("FORMATTED First"), "{map}");
    assert_eq!(b.read("notes/map.md"), map);
}
