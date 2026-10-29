//! Golden tests of `kluris create`, `register` and `remove`, as snapshots
//! of Kluris's own output.
//!
//! Each run is snapshotted as `<name>__run` (stdout, stderr, exit code).
//! A run that writes adds `<name>__tree` (every file under `work/`: the
//! scaffolded brain, `config.yml`, the HOME skills, with each installed
//! companion file and each brain's skill shown by reference, see [`tree`])
//! and, for a new brain,
//! `<name>__git` (its first commit, history, branch, remote, status and
//! the bytes of each file it committed, see [`git_state`]). A run that
//! must write nothing is checked against the tree from before it. The paths
//! Kluris prints and writes as the OS writes them are checked in that form
//! and shown `/`-joined, so one snapshot holds on every OS (see [`shown`]).
//!
//! Scenarios that start from brains made earlier build them from
//! `tests/fixtures/create`: a brain's files and the skill it had in every
//! agent's own folder, beside its Windsurf workflow (see [`Prior`]).
//!
//! The wizard and the pickers need a terminal on stdin: those scenarios run
//! Kluris on a pseudo-terminal and exist on Unix only.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use common::sandbox::GITCONFIG;
use common::shared::{skill_brain, skill_reference, vendored_marker};
use common::{
    Dump, Entry, NativePaths, PINNED_GIT_DATE, Run, Sandbox, fixture, git_state, normalize,
    slashed, tree_dump, tree_files, windows_variant, write_files,
};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::string::string_regex;
use proptest::test_runner::{Config as ProptestConfig, TestCaseError, TestRunner};
use regex::Regex;

#[cfg(unix)]
use common::pty::{End, run_on_terminal};

/// The lobes each `--type` scaffolds.
const TYPES: &[(&str, i64)] = &[
    ("product-group", 3),
    ("personal", 3),
    ("product", 6),
    ("research", 6),
    ("blank", 0),
];

const DESCRIPTION: &str = "Dëmo \"brain\" — it's ünïcode";

const IDENTITY: &[&str] = &[
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
];

// --- snapshots -------------------------------------------------------------------

/// The snapshots with a Windows variant: `register_by_home`'s tree has no
/// `link`, as the test makes no symlink on Windows.
const WINDOWS_VARIANTS: &[&str] = &["register_by_home__tree"];

/// `text` as the snapshot `name` (its Windows variant when
/// [`WINDOWS_VARIANTS`] names it).
fn snapshot(name: &str, text: impl AsRef<str>) {
    let check = || insta::assert_snapshot!(name, text.as_ref());
    if WINDOWS_VARIANTS.contains(&name) {
        windows_variant(check);
    } else {
        check();
    }
}

/// The paths a run in `sb` prints and `config.yml` holds as the OS writes
/// them: the config's own and each registered brain's
/// ([`Sandbox::configured_paths`]), `work/` and every folder under it whose
/// name does not start with `.` (where brains are made and registered;
/// Kluris's and the agents' own folders under HOME are dot-named) with the
/// `kluris.yml` in it, then `named`, paths a case names that do not exist.
/// `config.yml` folds a long value at a space, so each part of a folder's
/// path before a space is one of these paths too.
fn paths_in_os_form(sb: &Sandbox, named: &[PathBuf]) -> Vec<PathBuf> {
    fn folders(dir: &Path, out: &mut Vec<PathBuf>) {
        out.push(dir.to_path_buf());
        out.push(dir.join("kluris.yml"));
        let text = dir.display().to_string();
        out.extend(
            text.match_indices(' ')
                .map(|(at, _)| PathBuf::from(&text[..at])),
        );
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("{e}"));
            let dot = entry.file_name().to_string_lossy().starts_with('.');
            if !dot && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders(&entry.path(), out);
            }
        }
    }
    let mut paths = sb.configured_paths();
    folders(&sb.work, &mut paths);
    paths.extend(named.iter().cloned());
    paths
}

/// The paths of [`paths_in_os_form`], to be checked in the OS's form and
/// shown `/`-joined in a run's stream or a body of a dump of `work/`.
fn native(sb: &Sandbox, named: &[PathBuf]) -> NativePaths {
    let paths = paths_in_os_form(sb, named);
    let shown: Vec<String> = paths.iter().map(|path| slashed(path)).collect();
    let pairs: Vec<(&Path, &str)> = paths
        .iter()
        .zip(&shown)
        .map(|(path, shown)| (path.as_path(), shown.as_str()))
        .collect();
    NativePaths::new(&pairs)
}

/// `text`, a run's stream, with the paths of [`native`] checked and shown.
fn shown(sb: &Sandbox, text: &str, named: &[PathBuf]) -> String {
    native(sb, named).show(text)
}

/// `run` as its snapshot holds it, its streams [`shown`].
fn run_text(sb: &Sandbox, run: &Run, named: &[PathBuf]) -> String {
    Run {
        code: run.code,
        stdout: shown(sb, &run.stdout, named),
        stderr: shown(sb, &run.stderr, named),
    }
    .render(&sb.root)
}

/// `kluris args` with stdin a pseudo-terminal and `KLURIS_NO_PROMPT` unset:
/// `lines` are typed one per line, then Ctrl-D.
#[cfg(unix)]
fn on_terminal(sb: &Sandbox, args: &[&str], lines: &[&str]) -> Run {
    let mut cmd = sb.command(args);
    cmd.env_remove("KLURIS_NO_PROMPT");
    run_on_terminal(cmd, lines, End::Eof)
}

/// Every file under `work/` (HOME included), normalized.
fn files(sb: &Sandbox) -> String {
    tree_dump(&sb.work, |text| normalize(text, &sb.root))
}

/// [`files`] as a snapshot holds it: an installed companion file that holds
/// its `vendored/` bytes reads `<vendored/NAME/PATH>`, and a brain's
/// SKILL.md, after the test asserts every copy of it is the same, reads
/// `<snapshot skill_...>` (see [`common::shared`] and [`skill_variant`]).
fn tree(sb: &Sandbox) -> String {
    let files = tree_files(&sb.work);
    let mut skills: BTreeMap<&str, (&str, &Entry)> = BTreeMap::new();
    for (path, entry) in &files {
        if let Some(brain) = skill_brain(path) {
            let (first, copy) = *skills.entry(brain).or_insert((path, entry));
            assert!(copy == entry, "{path} differs from {first}");
        }
    }
    let native = native(sb, &[]);
    let mut dump = Dump::new(|text: &str| normalize(&native.show(text), &sb.root));
    for (path, entry) in &files {
        match vendored_marker(path, entry)
            .or_else(|| skill_reference(path, entry, &sb.root, skill_variant))
        {
            Some(marker) => dump.reference(path, entry, &marker),
            None => dump.entry(path, entry),
        }
    }
    dump.end()
}

/// What tells a brain's skill text from the others of the same brain and
/// companions, as it names the brain's folder, git and description: `at-`
/// and the folder below `work/` unless it is `brains/<brain>`, `git` for a
/// git repository, and `desc-` and the description unless it is `<brain>
/// knowledge base`, each lowercased with every run of other characters than
/// ASCII letters and digits as one `-`.
fn skill_variant(text: &str) -> String {
    static FACTS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?m)^You are the SME for the \*\*(.+)\*\* brain\. The brain lives at `<ROOT>/work/(.+)` \((git|no git)\)\.\nDescription: (.*)$",
        )
        .unwrap()
    });
    let caps = FACTS
        .captures(text)
        .unwrap_or_else(|| panic!("a skill without its brain's facts:\n{text}"));
    let slug = |s: &str| {
        s.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    };
    let brain = &caps[1];
    let mut parts = Vec::new();
    if caps[2] != format!("brains/{brain}") {
        parts.push(format!("at-{}", slug(&caps[2])));
    }
    if &caps[3] == "git" {
        parts.push("git".to_string());
    }
    if caps[4] != format!("{brain} knowledge base") {
        parts.push(format!("desc-{}", slug(&caps[4])));
    }
    parts.join("_")
}

/// The repository at `rel` (relative to `work/`), normalized.
fn repo_state(sb: &Sandbox, rel: &str) -> String {
    git_state(&sb.work.join(rel), |text| normalize(text, &sb.root))
}

/// `rel`, a `/`-joined path, under `base` as the OS writes it.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// The folder `create` makes from its `given` arguments (`NAME [--path
/// PATH]`, PATH under `work/`, `work/` itself when none is given), which its
/// errors print and which does not exist.
fn create_target(sb: &Sandbox, given: &[&str]) -> PathBuf {
    let base = match given.iter().position(|arg| *arg == "--path") {
        Some(at) => under(&sb.work, given[at + 1]),
        None => sb.work.clone(),
    };
    base.join(given[0])
}

fn write(sb: &Sandbox, rel: &str, text: &str) {
    write_files(&sb.work, &[(rel, text)]);
}

fn mkdir(sb: &Sandbox, rel: &str) {
    fs::create_dir_all(sb.work.join(rel)).unwrap();
}

fn git(sb: &Sandbox, rel: &str, args: &[&str]) {
    sb.git(&sb.work.join(rel), args, PINNED_GIT_DATE);
}

/// The run, the tree it leaves and, with `repo` (relative to `work/`), the
/// repository there.
fn assert_golden(name: &str, sb: &Sandbox, run: &Run, repo: Option<&str>) {
    snapshot(&format!("{name}__run"), run_text(sb, run, &[]));
    snapshot(&format!("{name}__tree"), tree(sb));
    if let Some(repo) = repo {
        snapshot(&format!("{name}__git"), repo_state(sb, repo));
    }
}

/// The run of a command that writes nothing: `before` is the tree from
/// before it.
fn assert_golden_unchanged(name: &str, sb: &Sandbox, before: &str, run: &Run) {
    assert_golden_unchanged_naming(name, sb, before, run, &[]);
}

/// [`assert_golden_unchanged`] for a run that also prints `named`, paths
/// that do not exist.
fn assert_golden_unchanged_naming(
    name: &str,
    sb: &Sandbox,
    before: &str,
    run: &Run,
    named: &[PathBuf],
) {
    assert_eq!(files(sb), before, "{name}: the run changed the tree");
    snapshot(&format!("{name}__run"), run_text(sb, run, named));
}

// --- brains made earlier -----------------------------------------------------------

/// The folders that each held a copy of a brain's skill, under HOME; its
/// Windsurf workflow was `.codeium/windsurf/global_workflows/kluris-<brain>.md`.
const SKILL_FOLDERS: &[&str] = &[
    ".agents/skills",
    ".claude/skills",
    ".codeium/windsurf/skills",
    ".codex/skills",
    ".copilot/skills",
    ".cursor/skills",
    ".gemini/skills",
    ".hermes/skills",
    ".junie/skills",
    ".kilo/skills",
];

/// The state a scenario starts from: brains made and registered earlier.
///
/// `brains`: each brain's name, its folder under `work/`, whether it is a
/// git repository and its description. Its files are `brain/` in the
/// fixtures, with `<BRAIN>`, `<DESCRIPTION>` and `<TODAY>` filled in and
/// `gitignore` placed as `.gitignore` (so it ignores nothing in this
/// repository); with git, its first commit is made as `create` makes it.
/// `skills`: the brains whose skill sits in every [`SKILL_FOLDERS`] folder
/// beside its Windsurf workflow: the fixture `SKILL.md` with the brain's
/// name, path (`<BRAIN PATH>`), `<GIT>` (`git` or `no git`) and
/// description filled in, and the workflow that skill without its `name:`
/// line and with its description cut to 200 characters. `config`:
/// `config.yml`. In the skills and the config, `<ROOT>` is the sandbox's
/// root.
struct Prior {
    brains: &'static [(&'static str, &'static str, bool, &'static str)],
    skills: &'static [&'static str],
    config: &'static str,
}

/// `taken`, at `brains/taken` without git.
const TAKEN: Prior = Prior {
    brains: &[("taken", "brains/taken", false, "taken knowledge base")],
    skills: &["taken"],
    config: "brains:\n  taken:\n    path: <ROOT>/work/brains/taken\n    description: taken knowledge base\n",
};

/// `first`, at `first` without git.
const FIRST: Prior = Prior {
    brains: &[("first", "first", false, "first knowledge base")],
    skills: &["first"],
    config: "brains:\n  first:\n    path: <ROOT>/work/first\n    description: first knowledge base\n",
};

/// `a--b`, at `a--b` with git: a name `create` refuses now.
const A_B: Prior = Prior {
    brains: &[("a--b", "a--b", true, "a--b knowledge base")],
    skills: &["a--b"],
    config: "brains:\n  a--b:\n    path: <ROOT>/work/a--b\n    description: a--b knowledge base\n",
};

/// `team`, at `brains/team` with git, unregistered.
const TEAM_UNREGISTERED: Prior = Prior {
    brains: &[("team", "brains/team", true, "The team brain")],
    skills: &[],
    config: "brains: {}\n",
};

/// `team`, at `brains/team` with git, registered.
const TEAM: Prior = Prior {
    brains: &[("team", "brains/team", true, "The team brain")],
    skills: &["team"],
    config: "brains:\n  team:\n    path: <ROOT>/work/brains/team\n    description: The team brain\n",
};

/// `a` (git) and `b` (no git) in `brains`.
const TWO: Prior = Prior {
    brains: &[
        ("a", "brains/a", true, "a knowledge base"),
        ("b", "brains/b", false, "b knowledge base"),
    ],
    skills: &["a", "b"],
    config: "brains:\n  a:\n    path: <ROOT>/work/brains/a\n    description: a knowledge base\n  b:\n    path: <ROOT>/work/brains/b\n    description: b knowledge base\n",
};

/// `only`, at `only` with git.
const ONLY: Prior = Prior {
    brains: &[("only", "only", true, "only knowledge base")],
    skills: &["only"],
    config: "brains:\n  only:\n    path: <ROOT>/work/only\n    description: only knowledge base\n",
};

fn place(sb: &Sandbox, prior: &Prior) {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string();
    for (name, rel, with_git, description) in prior.brains {
        for (file, text) in fixture_files(&fixture("create/brain")) {
            let file = if file == "gitignore" {
                ".gitignore".to_string()
            } else {
                file
            };
            let text = text
                .replace("<DESCRIPTION>", description)
                .replace("<BRAIN>", name)
                .replace("<TODAY>", &today);
            write(sb, &format!("{rel}/{file}"), &text);
        }
        if *with_git {
            git(sb, rel, &["init", "-q"]);
            git(sb, rel, &["checkout", "-q", "-b", "main"]);
            git(sb, rel, &["add", "-A"]);
            git(
                sb,
                rel,
                &["commit", "-q", "-m", &format!("brain: initialize {name}")],
            );
        }
    }
    let template = fs::read_to_string(fixture("create/SKILL.md")).unwrap();
    for skill_of in prior.skills {
        let Some((name, rel, with_git, description)) =
            prior.brains.iter().find(|(name, ..)| name == skill_of)
        else {
            panic!("no brain {skill_of} for its skill");
        };
        let skill = template
            .replace("<BRAIN PATH>", &format!("<ROOT>/work/{rel}"))
            .replace("<GIT>", if *with_git { "git" } else { "no git" })
            .replace("<DESCRIPTION>", description)
            .replace("<BRAIN>", name);
        let skill = expand(&skill, &sb.root);
        for folder in SKILL_FOLDERS {
            write(sb, &format!("home/{folder}/kluris-{name}/SKILL.md"), &skill);
        }
        write(
            sb,
            &format!("home/.codeium/windsurf/global_workflows/kluris-{name}.md"),
            &workflow(&skill),
        );
    }
    write(
        sb,
        "home/.kluris/config.yml",
        &expand(prior.config, &sb.root),
    );
}

/// The Windsurf workflow beside `skill`: the skill without its `name:` line,
/// its description cut to its first 200 characters.
fn workflow(skill: &str) -> String {
    let mut lines: Vec<String> = skill.split('\n').map(str::to_string).collect();
    assert!(lines[1].starts_with("name: "), "{}", lines[1]);
    lines.remove(1);
    let quoted = lines[1]
        .strip_prefix("description: \"")
        .and_then(|d| d.strip_suffix('"'))
        .unwrap_or_else(|| panic!("no quoted description: {}", lines[1]));
    lines[1] = format!(
        "description: \"{}\"",
        quoted.chars().take(200).collect::<String>()
    );
    lines.join("\n")
}

/// Every file under `dir`, `.git` left out, as (its `/`-joined relative
/// path, its text).
fn fixture_files(dir: &Path) -> Vec<(String, String)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let rel = format!("{prefix}{name}");
            if name == ".git" {
                continue;
            } else if path.is_dir() {
                walk(&path, &format!("{rel}/"), out);
            } else {
                let text =
                    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                out.push((rel, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out
}

/// `text` with each `<ROOT>/a/b` as the native path of `root/a/b`.
fn expand(text: &str, root: &Path) -> String {
    static AT_ROOT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"<ROOT>((?:/[A-Za-z0-9._-]+)*)").unwrap_or_else(|e| panic!("{e}"))
    });
    AT_ROOT
        .replace_all(text, |caps: &regex::Captures| {
            let path: PathBuf = caps[1]
                .split('/')
                .filter(|s| !s.is_empty())
                .fold(root.to_path_buf(), |p, s| p.join(s));
            path.display().to_string()
        })
        .into_owned()
}

// --- create: scaffolds -------------------------------------------------------------

#[test]
fn create_every_type() {
    for (brain_type, lobes) in TYPES {
        for git in [true, false] {
            let sb = &Sandbox::new();
            mkdir(sb, "brains");
            let mut args = vec!["create", "demo", "--path", "brains", "--type", brain_type];
            args.extend(["--description", DESCRIPTION]);
            if !git {
                args.push("--no-git");
            }
            args.push("--json");
            let run = sb.kluris(&args);
            assert!(
                run.stdout.contains(&format!("\"lobes\": {lobes},")),
                "{brain_type}: {}",
                run.stdout
            );
            let name = format!(
                "create_every_type__{brain_type}__{}",
                if git { "git" } else { "no-git" }
            );
            assert_golden(&name, sb, &run, Some("brains/demo"));
        }
    }
}

/// No `--path`, `--type` or `--description`: `<cwd>/<name>`, the
/// product-group lobes, `<name> knowledge base`.
#[test]
fn create_defaults_with_json() {
    let sb = &Sandbox::new();
    let run = sb.kluris(&["create", "solo", "--json"]);
    assert_golden("create_defaults_with_json", sb, &run, Some("solo"));
}

#[test]
fn create_text_output() {
    let sb = &Sandbox::new();
    mkdir(sb, "brains");
    let args = [
        "create",
        "demo",
        "--path",
        "brains",
        "--type",
        "personal",
        "--description",
        "d",
        "--no-git",
    ];
    let run = sb.kluris(&args);
    assert_golden("create_text_output", sb, &run, None);
}

const GIT_SETUPS: &[(&str, &[&str])] = &[
    ("branch", &["--branch", "dev"]),
    ("remote", &["--remote", "git@example.com:team/brain.git"]),
    (
        "remote-branch",
        &[
            "--remote",
            "https://example.com/brain.git",
            "--branch",
            "trunk",
        ],
    ),
    ("branch-main", &["--branch", "main"]),
    (
        "remote-no-git",
        &["--remote", "git@example.com:team/brain.git", "--no-git"],
    ),
];

#[test]
fn create_git_setup() {
    for (case, extra) in GIT_SETUPS {
        let sb = &Sandbox::new();
        let mut args = vec!["create", "demo", "--type", "blank"];
        args.extend(*extra);
        args.push("--json");
        let run = sb.kluris(&args);
        assert_golden(&format!("create_git_setup__{case}"), sb, &run, Some("demo"));
    }
}

/// Without a git identity the first commit is `kluris <kluris@local>`.
#[test]
fn create_commit_identity_fallback() {
    let sb = &Sandbox::new();
    let mut cmd = sb.command(&["create", "demo", "--json"]);
    IDENTITY.iter().for_each(|key| {
        cmd.env_remove(key);
    });
    let run = Run::of(cmd);
    assert_golden("create_commit_identity_fallback", sb, &run, Some("demo"));
}

/// With `user.name`/`user.email` in git config and no identity in the
/// environment, the commit is by that identity.
#[test]
fn create_keeps_a_git_identity() {
    let sb = &Sandbox::new();
    let gitconfig = format!("{GITCONFIG}[user]\n\tname = Ada\n\temail = ada@example.com\n");
    fs::write(sb.gitconfig(), gitconfig).unwrap_or_else(|e| panic!("{e}"));
    let mut cmd = sb.command(&["create", "demo", "--json"]);
    IDENTITY.iter().for_each(|key| {
        cmd.env_remove(key);
    });
    let run = Run::of(cmd);
    assert_golden("create_keeps_a_git_identity", sb, &run, Some("demo"));
}

#[test]
fn create_into_an_existing_empty_directory() {
    let sb = &Sandbox::new();
    mkdir(sb, "brains/demo");
    let run = sb.kluris(&["create", "demo", "--path", "brains", "--json"]);
    assert_golden(
        "create_into_an_existing_empty_directory",
        sb,
        &run,
        Some("brains/demo"),
    );
}

/// A second brain beside `first`, whose skill sits in every agent folder:
/// both brains' skills end up in the current folders.
#[test]
fn create_second_brain_installs_both_skills() {
    let sb = &Sandbox::new();
    place(sb, &FIRST);
    let run = sb.kluris(&["create", "second", "--no-git", "--json"]);
    assert_golden("create_second_brain_installs_both_skills", sb, &run, None);
}

/// A clean install adds no `skills` key to `create --json`, and the user's
/// own plain `kluris-notes` skill is left alone.
#[test]
fn create_clean_json_keeps_its_bytes() {
    let sb = &Sandbox::new();
    write(
        sb,
        "home/.agents/skills/kluris-notes/SKILL.md",
        "---\nname: kluris-notes\ndescription: mine\n---\n\n# Notes\n",
    );
    let run = sb.kluris(&["create", "demo", "--no-git", "--json"]);
    assert!(!run.stdout.contains("\"skills\""), "{}", run.stdout);
    assert_golden("create_clean_json_keeps_its_bytes", sb, &run, None);
}

// --- create: validation --------------------------------------------------------------

/// A registered brain `taken`, a file, a non-empty directory, an empty one
/// and two holding a `kluris.yml`.
fn validation_scenario() -> Sandbox {
    let sb = Sandbox::new();
    place(&sb, &TAKEN);
    write(&sb, "afile", "x");
    write(&sb, "brains/full/notes.md", "x");
    mkdir(&sb, "brains/empty");
    write(&sb, "brains/hasyml/kluris.yml", "name: x\n");
    write(&sb, "brains/outer/kluris.yml", "name: outer\n");
    sb
}

/// The `create` arguments each validation error is asked with.
const ERRORS: &[(&str, &[&str])] = &[
    ("reserved", &["all"]),
    (
        "too-long",
        &["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
    ),
    ("upper", &["Bad"]),
    ("digit-first", &["9x"]),
    ("dot", &[".x"]),
    ("empty", &[""]),
    ("registered", &["taken"]),
    ("path-file", &["new", "--path", "afile"]),
    ("path-missing", &["new", "--path", "no/such"]),
    ("path-brain", &["new", "--path", "brains/taken"]),
    (
        "inside-brain",
        &["inner", "--path", "brains/taken/projects"],
    ),
    ("not-empty", &["full", "--path", "brains"]),
    ("has-yml", &["hasyml", "--path", "brains"]),
    ("order-reserved", &["all", "--path", "afile"]),
    ("order-registered", &["taken", "--path", "afile"]),
    ("order-path", &["inner", "--path", "brains/taken"]),
];

#[test]
fn create_validation_json() {
    let sb = &validation_scenario();
    let before = files(sb);
    for (case, given) in ERRORS {
        let mut args = vec!["create"];
        args.extend(*given);
        args.push("--json");
        let run = sb.kluris(&args);
        assert_golden_unchanged_naming(
            &format!("create_validation_json__{case}"),
            sb,
            &before,
            &run,
            &[create_target(sb, given)],
        );
    }
}

/// Every value given, so nothing is asked. An empty NAME is a missing one
/// in text mode, so `empty` has no text case.
#[test]
fn create_validation_text() {
    let sb = &validation_scenario();
    let before = files(sb);
    for (case, given) in ERRORS.iter().filter(|(case, _)| *case != "empty") {
        let mut args = vec!["create"];
        args.extend(*given);
        args.extend(["--description", "d", "--type", "blank", "--no-git"]);
        if !given.contains(&"--path") {
            args.extend(["--path", "brains"]);
        }
        let run = sb.kluris(&args);
        assert_golden_unchanged_naming(
            &format!("create_validation_text__{case}"),
            sb,
            &before,
            &run,
            &[create_target(sb, &args[1..])],
        );
    }
}

const OPTION_VALUES: &[(&str, &[&str])] = &[
    ("type", &["create", "x", "--type", "bogus"]),
    ("type-json", &["create", "x", "--type", "bogus", "--json"]),
    ("from-config", &["create", "x", "--from-config", "nope.yml"]),
    (
        "from-config-json",
        &["create", "x", "--from-config", "nope.yml", "--json"],
    ),
];

#[test]
fn create_option_values() {
    for (case, args) in OPTION_VALUES {
        let sb = &Sandbox::new();
        let before = files(sb);
        let run = sb.kluris(args);
        assert_golden_unchanged(&format!("create_option_values__{case}"), sb, &before, &run);
    }
}

/// The last `--type` given wins.
#[test]
fn create_option_values_last_type_wins() {
    let sb = &Sandbox::new();
    let run = sb.kluris(&[
        "create", "x", "--type", "bogus", "--type", "blank", "--no-git", "--json",
    ]);
    assert_golden("create_option_values__last-type-wins", sb, &run, None);
}

// --- create: --from-config ------------------------------------------------------------

const FROM_CONFIG: &[(&str, &str)] = &[
    (
        "flat",
        "structure:\n  alpha-beta: First lobe\n  zeta: Ünïcode — dash\n  mid-lobe-name: Mid\n",
    ),
    ("no-structure", "other: 1\n"),
    ("empty-file", ""),
    ("empty-structure", "structure: {}\n"),
    ("number-value", "structure:\n  stats: 42\n"),
];

/// The lobes come from the file; `lobes` in the JSON counts `--type`'s.
#[test]
fn create_from_config() {
    for (case, text) in FROM_CONFIG {
        let sb = &Sandbox::new();
        write(sb, "structure.yml", text);
        let run = sb.kluris(&[
            "create",
            "demo",
            "--type",
            "research",
            "--from-config",
            "structure.yml",
            "--json",
        ]);
        assert_golden(
            &format!("create_from_config__{case}"),
            sb,
            &run,
            Some("demo"),
        );
    }
}

const FROM_CONFIG_REFUSED: &[(&str, &str)] = &[
    ("dotdot", "structure:\n  ../escape: x\n"),
    ("slash", "structure:\n  a/b: x\n"),
    ("backslash", "structure:\n  a\\b: x\n"),
    ("absolute", "structure:\n  /abs: x\n"),
    ("parent", "structure:\n  ..: x\n"),
    ("nested-map", "structure:\n  lobe:\n    sub: x\n"),
    ("nested-list", "structure:\n  lobe: [a, b]\n"),
];

/// A lobe name that is a path, or a nested value, is refused before
/// anything is written.
#[test]
fn from_config_refuses_paths_and_nested_values() {
    for (case, text) in FROM_CONFIG_REFUSED {
        let sb = &Sandbox::new();
        write(sb, "structure.yml", text);
        let before = files(sb);
        let run = sb.kluris(&["create", "demo", "--from-config", "structure.yml", "--json"]);
        assert_golden_unchanged(&format!("from_config_refused__{case}"), sb, &before, &run);
    }
}

// --- create: the wizard (a terminal) ---------------------------------------------------

/// Every prompt, empty answers asked again, answers kept unstripped, remote
/// and branch, and the companion picker (an invalid choice asked again).
#[cfg(unix)]
#[test]
fn wizard_full() {
    let sb = &Sandbox::new();
    let lines = [
        "",
        "wiz",
        "",
        " my desc ",
        "",
        "personal",
        "2",
        "",
        "git@x:y.git",
        "dev",
        "9",
        "3",
    ];
    let run = on_terminal(sb, &["create"], &lines);
    assert_golden("wizard_full", sb, &run, Some("wiz"));
}

#[cfg(unix)]
const WIZARD_GIT_CHOICES: &[(&str, &[&str])] = &[
    ("no-git", &["wiz", "d", "", "", "3", "4"]),
    (
        "local-all-companions",
        &["wiz", "d", "", "blank", "1", "", "7"],
    ),
    (
        "other-choice-is-local",
        &["wiz", "d", "", "research", "x", "trunk", ""],
    ),
];

#[cfg(unix)]
#[test]
fn wizard_git_choices() {
    for (case, lines) in WIZARD_GIT_CHOICES {
        let sb = &Sandbox::new();
        let run = on_terminal(sb, &["create"], lines);
        assert_golden(
            &format!("wizard_git_choices__{case}"),
            sb,
            &run,
            Some("wiz"),
        );
    }
}

#[cfg(unix)]
#[test]
fn wizard_asks_only_what_is_missing() {
    let sb = &Sandbox::new();
    mkdir(sb, "brains");
    let args = [
        "create",
        "demo",
        "--description",
        "d",
        "--path",
        "brains",
        "--type",
        "blank",
    ];
    let run = on_terminal(sb, &args, &["", "", "5"]);
    assert_golden(
        "wizard_asks_only_what_is_missing",
        sb,
        &run,
        Some("brains/demo"),
    );
}

#[cfg(unix)]
#[test]
fn wizard_with_every_value_given_asks_nothing() {
    let sb = &Sandbox::new();
    let args = [
        "create",
        "demo",
        "--description",
        "d",
        "--path",
        ".",
        "--type",
        "blank",
        "--no-git",
    ];
    let run = on_terminal(sb, &args, &[]);
    assert_golden("wizard_with_every_value_given_asks_nothing", sb, &run, None);
}

#[cfg(unix)]
#[test]
fn wizard_end_of_input_aborts() {
    let sb = &Sandbox::new();
    let before = files(sb);
    let run = on_terminal(sb, &["create"], &["wiz"]);
    assert_golden_unchanged("wizard_end_of_input_aborts", sb, &before, &run);
}

#[cfg(unix)]
#[test]
fn wizard_validates_after_the_prompts() {
    let sb = &Sandbox::new();
    let before = files(sb);
    let run = on_terminal(sb, &["create"], &["Bad", "d", "", "", "3"]);
    assert_golden_unchanged("wizard_validates_after_the_prompts", sb, &before, &run);
}

/// The brain type answer is checked like `--type` and asked again.
#[cfg(unix)]
#[test]
fn wizard_brain_type_is_checked_like_the_option() {
    let sb = &Sandbox::new();
    let lines = ["wiz", "d", "", "foo", "Personal", "personal", "3", "4"];
    let run = on_terminal(sb, &["create"], &lines);
    assert_golden(
        "wizard_brain_type_is_checked_like_the_option",
        sb,
        &run,
        None,
    );
}

/// `Aborted!` after the scaffold: the files stay, nothing is registered or
/// committed.
#[cfg(unix)]
#[test]
fn create_end_of_input_at_the_picker() {
    let sb = &Sandbox::new();
    let run = on_terminal(sb, &["create"], &["wiz", "d", "", "", "1", ""]);
    assert_golden("create_end_of_input_at_the_picker", sb, &run, Some("wiz"));
}

#[cfg(unix)]
#[test]
fn create_with_no_prompt_set_asks_nothing_on_a_terminal() {
    let sb = &Sandbox::new();
    let mut cmd = sb.command(&["create", "demo", "--no-git"]);
    cmd.env("KLURIS_NO_PROMPT", "1");
    let run = run_on_terminal(cmd, &[], End::Eof);
    assert_golden(
        "create_with_no_prompt_set_asks_nothing_on_a_terminal",
        sb,
        &run,
        None,
    );
}

// --- create: no terminal -------------------------------------------------------------

/// Nothing is asked; `create` makes what `--json` would.
#[test]
fn create_without_a_terminal_uses_the_defaults() {
    let sb = &Sandbox::new();
    let run = sb.kluris(&["create", "demo"]);
    assert_golden(
        "create_without_a_terminal_uses_the_defaults",
        sb,
        &run,
        Some("demo"),
    );
}

#[test]
fn create_without_a_name_and_no_terminal_is_an_error() {
    for (case, args) in [
        ("text", &["create"][..]),
        ("json", &["create", "--json"][..]),
    ] {
        let sb = &Sandbox::new();
        let before = files(sb);
        let run = sb.kluris(args);
        assert_golden_unchanged(
            &format!("create_without_a_name_and_no_terminal_is_an_error__{case}"),
            sb,
            &before,
            &run,
        );
    }
}

// --- create: the name rule -------------------------------------------------------------

/// New names follow `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`.
#[test]
fn new_names_follow_the_stricter_rule() {
    for name in ["a--b", "a-", "ab-", "x--", "a-b--c"] {
        let sb = &Sandbox::new();
        let before = files(sb);
        let run = sb.kluris(&["create", name, "--no-git", "--json"]);
        assert_golden_unchanged(
            &format!("new_names_follow_the_stricter_rule__{name}"),
            sb,
            &before,
            &run,
        );
    }
}

/// How many names [`create_answers_every_name_by_the_rule`] tries: enough
/// that valid names, names past the limit of 48 and names that break only
/// the hyphen rules each come up many times over, in about ten seconds of
/// `create` runs on a Mac and a few minutes at most where starting a
/// process costs more (Windows). `all` is checked by the validation
/// goldens.
const NAME_CASES: u32 = 2000;

/// The rule a new name follows, as a pattern: lowercase letters and
/// digits, a letter first, single hyphens between them.
static NEW_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[a-z][a-z0-9]*(?:-[a-z0-9]+)*\z").unwrap());

/// The looser rule names were checked by before the hyphens were: a letter,
/// then lowercase letters, digits and hyphens, and the one newline a
/// pattern's `$` lets through at the end.
static OLDER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[a-z][a-z0-9-]*\n?\z").unwrap());

/// Names of three kinds: short ones over letters, digits and the
/// characters a name gets wrong (`-`, `.`, `_`, `A`, `/`, a space, a
/// newline); ones that match the looser rule, up to 51 characters, past
/// the limit of 48; and up to six characters of any kind but NUL, which no
/// command line can hold.
fn names() -> impl Strategy<Value = String> {
    prop_oneof![
        string_regex("[az09._A/ \n-]{1,12}").unwrap(),
        string_regex("[a-z][a-z0-9-]{0,50}").unwrap(),
        vec(any::<char>(), 1..=6)
            .prop_map(String::from_iter)
            .prop_filter("a command line holds no NUL", |name| !name.contains('\0')),
    ]
}

/// What `create` answers for `name` when nothing else is wrong but its
/// `--path`, the file `afile`: the first rule the name breaks, in the order
/// `create` checks them, or, for a valid name, the path.
fn expected_error(name: &str) -> String {
    let length = name.chars().count();
    let older = OLDER_NAME.is_match(name) && !name.contains("..") && !name.starts_with('.');
    let new = NEW_NAME.is_match(name) && length <= 48 && name != "all";
    assert!(!new || older, "{name:?} is new but not older");
    if name == "all" {
        "'all' is a reserved brain name (used by --brain all). Pick a different name.".to_string()
    } else if length > 48 {
        format!("Brain name '{name}' is too long ({length} chars). Maximum is 48.")
    } else if !older {
        format!("Brain name '{name}' is invalid. Use lowercase letters, numbers, and hyphens only.")
    } else if !new {
        format!(
            "Brain name '{name}' is invalid. Use lowercase letters and numbers, with single \
             hyphens between them (no '--', no '-' at the end)."
        )
    } else {
        "--path 'afile' is not a directory.".to_string()
    }
}

/// `create` accepts exactly the names [`NEW_NAME`] matches, at most 48
/// characters and not `all`, each also one the looser rule accepts, and
/// refuses every other with the message of the first rule it breaks.
/// `--path` is a file, so a valid name stops there and nothing is written;
/// the name comes after `--`, so one that starts with `-` is still a name.
#[test]
fn create_answers_every_name_by_the_rule() {
    let sb = &Sandbox::new();
    write(sb, "afile", "x");
    let before = tree_files(&sb.work);
    let config = ProptestConfig {
        cases: NAME_CASES,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    let result = TestRunner::new(config).run(&names(), |name| {
        let run = sb.kluris(&["create", "--path", "afile", "--json", "--", &name]);
        let stdout: serde_json::Value = serde_json::from_str(&run.stdout)
            .map_err(|e| TestCaseError::fail(format!("{name:?}: {e}: {run:?}")))?;
        let expected = serde_json::json!({"ok": false, "error": expected_error(&name)});
        prop_assert_eq!(&stdout, &expected, "{:?}", name);
        prop_assert_eq!((run.code, run.stderr.as_str()), (1, ""), "{:?}", name);
        prop_assert!(
            tree_files(&sb.work) == before,
            "{:?} changed the files",
            name
        );
        Ok(())
    });
    result.unwrap_or_else(|e| panic!("{e}"));
}

/// A registered `a--b`, a name `create` refuses, keeps working.
#[test]
fn a_registered_name_create_refuses_still_loads() {
    let commands: &[(&str, &[&str])] = &[
        ("wake-up", &["wake-up", "--brain", "a--b", "--json"]),
        ("list", &["list", "--json"]),
        ("status", &["status", "--brain", "a--b", "--json"]),
        ("remove", &["remove", "a--b", "--json"]),
    ];
    for (case, args) in commands {
        let sb = &Sandbox::new();
        place(sb, &A_B);
        let run = sb.kluris(args);
        snapshot(
            &format!("a_registered_name_create_refuses_still_loads__{case}__run"),
            run_text(sb, &run, &[]),
        );
    }
}

// --- create: git failure ---------------------------------------------------------------

/// The message names the git command, what git said and what is left; the
/// scaffold stays on disk and nothing is registered. git words its own
/// errors by version, so what it said is `<GIT STDERR>` in the snapshot.
#[test]
fn a_git_failure_leaves_the_brain_unregistered() {
    let sb = &Sandbox::new();
    let mut run = sb.kluris(&["create", "demo", "--branch", "bad..name", "--json"]);
    let start = "failed (exit 128): ";
    let end = " (the brain files are left in";
    let (Some(from), Some(to)) = (run.stdout.find(start), run.stdout.find(end)) else {
        panic!("no git error in {}", run.stdout);
    };
    let said = &run.stdout[from + start.len()..to];
    assert!(
        said.starts_with("fatal: 'bad..name' is not a valid branch name"),
        "{said}"
    );
    run.stdout
        .replace_range(from + start.len()..to, "<GIT STDERR>");
    assert_golden(
        "a_git_failure_leaves_the_brain_unregistered",
        sb,
        &run,
        None,
    );
}

// --- register --------------------------------------------------------------------------------

/// `team`'s files and git in `brains/team`, unregistered.
fn team_brain() -> Sandbox {
    let sb = Sandbox::new();
    place(&sb, &TEAM_UNREGISTERED);
    sb
}

#[test]
fn register_a_brain_made_earlier() {
    for (case, args) in [
        ("json", &["register", "brains/team", "--json"][..]),
        ("text", &["register", "brains/team"][..]),
    ] {
        let sb = &team_brain();
        let run = sb.kluris(args);
        assert_golden(
            &format!("register_a_brain_made_earlier__{case}"),
            sb,
            &run,
            None,
        );
    }
}

#[test]
fn register_writes_a_missing_kluris_yml() {
    let sb = &team_brain();
    fs::remove_file(sb.work.join("brains/team/kluris.yml")).unwrap_or_else(|e| panic!("{e}"));
    let run = sb.kluris(&["register", "brains/team", "--json"]);
    assert_golden("register_writes_a_missing_kluris_yml", sb, &run, None);
}

const IDENTITIES: &[(&str, &str, &str)] = &[
    (
        "h1",
        "brains/Some Folder",
        "---\nauto_generated: true\n---\n\n# team-x\n\nThe team brain.\n\n## Lobes\n",
    ),
    (
        "fallback-lowered",
        "brains/My Brain",
        "# Not Valid!\n\n## Lobes\n\nAfter lobes\n",
    ),
    ("no-heading", "brains/plain", "no heading here\n"),
    (
        "frontmatter-fails",
        "brains/rawscan",
        "---\nkey: [unclosed\n---\n# raw-name\n\nraw desc\n",
    ),
    (
        "spaces",
        "brains/team-y",
        "\n\n#   spaced-name  \n\n  Indented description.  \n",
    ),
    (
        "two-h1",
        "brains/team-z",
        "# first\n# second\n\n## Sub\n\nlater line\n",
    ),
];

/// A folder with only a `brain.md`: the name and description come from it.
#[test]
fn register_identity_from_brain_md() {
    for (case, rel, brain_md) in IDENTITIES {
        let sb = &Sandbox::new();
        write(sb, &format!("{rel}/brain.md"), brain_md);
        let run = sb.kluris(&["register", rel, "--json"]);
        assert_golden(
            &format!("register_identity_from_brain_md__{case}"),
            sb,
            &run,
            None,
        );
    }
}

#[test]
fn register_an_invalid_identity() {
    let sb = &Sandbox::new();
    write(sb, "brains/9bad/brain.md", "# Also Bad\n");
    let before = files(sb);
    for (case, args) in [
        ("json", &["register", "brains/9bad", "--json"][..]),
        ("text", &["register", "brains/9bad"][..]),
    ] {
        let run = sb.kluris(args);
        assert_golden_unchanged(
            &format!("register_an_invalid_identity__{case}"),
            sb,
            &before,
            &run,
        );
    }
}

const REGISTER_PATH_ERRORS: &[(&str, &str)] = &[
    ("zip", "x.zip"),
    ("zip-upper", "brains/X.ZIP"),
    ("zip-home", "~/b.Zip"),
    ("missing", "nope"),
    ("missing-home", "~/nope"),
    ("file", "afile"),
    ("empty-dir", "empty"),
    ("missing-deeper", "./nope//deeper/"),
];

#[test]
fn register_path_errors() {
    let sb = &Sandbox::new();
    write(sb, "afile", "x");
    mkdir(sb, "empty");
    let before = files(sb);
    for (case, source) in REGISTER_PATH_ERRORS {
        // The paths these errors print, which do not exist.
        let named = match *case {
            "missing-home" => vec![sb.home.join("nope")],
            "missing-deeper" => vec![Path::new("nope").join("deeper")],
            _ => vec![],
        };
        for (mode, json) in [("json", true), ("text", false)] {
            let mut args = vec!["register", source];
            if json {
                args.push("--json");
            }
            let run = sb.kluris(&args);
            assert_golden_unchanged_naming(
                &format!("register_path_errors__{case}__{mode}"),
                sb,
                &before,
                &run,
                &named,
            );
        }
    }
}

/// Same name and path: success with `already_registered`, and nothing is
/// written.
#[test]
fn register_the_same_brain_again() {
    let sb = &Sandbox::new();
    place(sb, &TEAM);
    let before = files(sb);
    for (case, args) in [
        ("json", &["register", "brains/team", "--json"][..]),
        ("text", &["register", "brains/team"][..]),
    ] {
        let run = sb.kluris(args);
        assert_golden_unchanged(
            &format!("register_the_same_brain_again__{case}"),
            sb,
            &before,
            &run,
        );
    }
}

/// The same name at another path, then the same path under another name.
#[test]
fn register_conflicts() {
    let sb = &Sandbox::new();
    place(sb, &TEAM);
    for (file, text) in fixture_files(&sb.work.join("brains/team")) {
        write(sb, &format!("brains/copy/{file}"), &text);
    }
    let before = files(sb);
    let run = sb.kluris(&["register", "brains/copy", "--json"]);
    assert_golden_unchanged("register_conflicts__same-name", sb, &before, &run);

    let brain_md = sb.work.join("brains/team/brain.md");
    let text = fs::read_to_string(&brain_md).unwrap_or_else(|e| panic!("{e}"));
    write(
        sb,
        "brains/team/brain.md",
        &text.replace("# team", "# other"),
    );
    let before = files(sb);
    let run = sb.kluris(&["register", "brains/team", "--json"]);
    assert_golden_unchanged("register_conflicts__same-path", sb, &before, &run);
}

/// A brain inside a registered one registers like any other.
#[test]
fn register_a_brain_inside_another() {
    let sb = &Sandbox::new();
    place(sb, &TEAM);
    write(
        sb,
        "brains/team/projects/sub/brain.md",
        "# sub-brain\n\nInner.\n",
    );
    let run = sb.kluris(&["register", "brains/team/projects/sub", "--json"]);
    assert_golden("register_a_brain_inside_another", sb, &run, None);
}

/// `team` moved to `~/brains/team`, and on Unix `link` pointing at it.
fn team_in_home() -> Sandbox {
    let sb = team_brain();
    mkdir(&sb, "home/brains");
    fs::rename(sb.work.join("brains/team"), sb.home.join("brains/team"))
        .unwrap_or_else(|e| panic!("{e}"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(sb.home.join("brains/team"), sb.work.join("link"))
        .unwrap_or_else(|e| panic!("{e}"));
    sb
}

#[test]
fn register_by_home() {
    let sb = &team_in_home();
    let run = sb.kluris(&["register", "~/brains/team", "--json"]);
    assert_golden("register_by_home", sb, &run, None);
}

#[cfg(unix)]
#[test]
fn register_by_symlink() {
    let sb = &team_in_home();
    let run = sb.kluris(&["register", "link", "--json"]);
    assert_golden("register_by_symlink", sb, &run, None);
}

#[test]
fn register_with_an_unreadable_kluris_yml() {
    let sb = &team_brain();
    write(sb, "brains/team/kluris.yml", "name: [broken\n");
    let before = files(sb);
    let run = sb.kluris(&["register", "brains/team", "--json"]);
    assert_golden_unchanged("register_with_an_unreadable_kluris_yml", sb, &before, &run);
}

#[cfg(unix)]
#[test]
fn register_prompts_on_a_terminal() {
    let sb = &team_brain();
    let run = on_terminal(sb, &["register"], &["", "brains/team", "6"]);
    assert_golden("register_prompts_on_a_terminal", sb, &run, None);
}

/// End of input at the companion picker: `Register failed: ` with nothing
/// after it, and the `kluris.yml` written before it stays.
#[cfg(unix)]
#[test]
fn register_end_of_input_at_the_picker() {
    let sb = &team_brain();
    fs::remove_file(sb.work.join("brains/team/kluris.yml")).unwrap_or_else(|e| panic!("{e}"));
    let run = on_terminal(sb, &["register"], &["brains/team"]);
    assert_golden("register_end_of_input_at_the_picker", sb, &run, None);
}

#[test]
fn register_without_a_path_and_no_terminal_is_an_error() {
    for (case, args) in [
        ("text", &["register"][..]),
        ("json", &["register", "--json"][..]),
    ] {
        let sb = &Sandbox::new();
        let before = files(sb);
        let run = sb.kluris(args);
        assert_golden_unchanged(
            &format!("register_without_a_path_and_no_terminal_is_an_error__{case}"),
            sb,
            &before,
            &run,
        );
    }
}

// --- remove ------------------------------------------------------------------------------------

/// Unregisters `a` and removes its skills from every folder; `b`'s skill
/// moves to the current folders; the files stay.
#[test]
fn remove() {
    for (case, args) in [
        ("json", &["remove", "a", "--json"][..]),
        ("text", &["remove", "a"][..]),
    ] {
        let sb = &Sandbox::new();
        place(sb, &TWO);
        let run = sb.kluris(args);
        assert_golden(&format!("remove__{case}"), sb, &run, None);
    }
}

#[test]
fn remove_the_last_brain() {
    let sb = &Sandbox::new();
    place(sb, &ONLY);
    let run = sb.kluris(&["remove", "only", "--json"]);
    assert_golden("remove_the_last_brain", sb, &run, None);
}

#[test]
fn remove_errors() {
    let cases: &[(&str, &[&str])] = &[
        ("unknown", &["remove", "nope"]),
        ("unknown-json", &["remove", "nope", "--json"]),
        ("missing", &["remove"]),
        ("missing-json", &["remove", "--json"]),
    ];
    let sb = &Sandbox::new();
    place(sb, &TWO);
    let before = files(sb);
    for (case, args) in cases {
        let run = sb.kluris(args);
        assert_golden_unchanged(&format!("remove_errors__{case}"), sb, &before, &run);
    }
}

// --- help ----------------------------------------------------------------------------------------

#[test]
fn help_of_the_commands() {
    let sb = &Sandbox::new();
    for (case, args) in [
        ("root", &["--help"][..]),
        ("create", &["create", "--help"][..]),
        ("register", &["register", "--help"][..]),
        ("remove", &["remove", "--help"][..]),
    ] {
        let run = sb.kluris(args);
        snapshot(&format!("help__{case}__run"), run_text(sb, &run, &[]));
    }
}

// --- write safety -----------------------------------------------------------------------------

/// A target folder that cannot be listed is refused, and nothing is
/// written.
#[cfg(unix)]
#[test]
fn create_into_a_directory_that_cannot_be_listed() {
    use std::os::unix::fs::PermissionsExt;

    let sb = &Sandbox::new();
    write(sb, "brains/demo/README.md", "my own readme\n");
    let target = sb.work.join("brains/demo");
    let mode = |m| {
        fs::set_permissions(&target, fs::Permissions::from_mode(m))
            .unwrap_or_else(|e| panic!("{e}"))
    };
    mode(0o300);
    if fs::read_dir(&target).is_ok() {
        mode(0o755);
        eprintln!("skipped: running as root, every directory can be listed");
        return;
    }
    let run = sb.kluris(&[
        "create", "demo", "--path", "brains", "--type", "blank", "--no-git", "--json",
    ]);
    mode(0o755);
    assert_golden(
        "create_into_a_directory_that_cannot_be_listed",
        sb,
        &run,
        None,
    );
}
