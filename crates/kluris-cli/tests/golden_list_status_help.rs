//! Golden tests of `list`, `status` and `help`: their JSON and text over real
//! git brains built with the sandbox's pinned identity and dates, the help
//! of each command, and how the command line is parsed (usage errors, the
//! order Kluris decides them in, repeated options).
//!
//! A test with one run snapshots it alone; a table snapshots all its rows in
//! one file, each run in a fresh sandbox under its `$ kluris …` line. The
//! paths Kluris prints as the OS writes them (a brain's, the config's) are
//! checked in that form and shown `/`-joined ([`Sandbox::kluris_shown`]).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{PINNED_GIT_DATE, Run, Sandbox, command_line, normalize, tree_dump, write_files};

// --- snapshots -----------------------------------------------------------------

/// Runs `kluris args` in `sb` and checks the run against the snapshot `name`.
fn golden(sb: &Sandbox, name: &str, args: &[&str]) {
    let run = sb.kluris_shown(args);
    insta::with_settings!({ description => command_line(args), omit_expression => true }, {
        insta::assert_snapshot!(name, run.render(&sb.root));
    });
}

/// Runs each row in a sandbox of its own from `setup` and checks them, in
/// order, against the snapshot `name`: each row's `$ kluris …` line, then
/// what `show` makes of its run.
fn golden_table(
    name: &str,
    rows: &[&[&str]],
    setup: fn() -> Sandbox,
    show: fn(&Run, &Path) -> String,
) {
    let mut out = String::new();
    for args in rows {
        let sb = setup();
        let run = sb.kluris_shown(args);
        out.push_str(&format!(
            "$ {}\n{}",
            command_line(args),
            show(&run, &sb.root)
        ));
    }
    insta::with_settings!({ omit_expression => true }, {
        insta::assert_snapshot!(name, out);
    });
}

/// The whole run: its streams and exit code.
fn whole(run: &Run, root: &Path) -> String {
    run.render(root)
}

/// How a run ended, in the terms its precedence is decided in: `help
/// <command path of its usage line>`, `version`, `error <code> <last stderr
/// line>`, or `ran <code>`.
fn outcome(run: &Run, root: &Path) -> String {
    let line = if run.code == 0 && run.stderr.is_empty() && run.stdout.contains("Usage: kluris") {
        let usage = run
            .stdout
            .lines()
            .find(|line| line.starts_with("Usage: "))
            .unwrap_or_default();
        let words: Vec<&str> = usage
            .split_whitespace()
            .skip(1)
            .filter(|w| !w.starts_with(['[', '<']) && !is_upper(w))
            .collect();
        format!("help {}", words.join(" "))
    } else if run.code == 0
        && run
            .stdout
            .strip_prefix("kluris ")
            .and_then(|rest| rest.strip_suffix('\n'))
            .is_some_and(|v| !v.is_empty() && !v.contains(char::is_whitespace))
    {
        "version".to_string()
    } else if run.code != 0 && run.stdout.is_empty() && !run.stderr.is_empty() {
        let last = run.stderr.trim().lines().last().unwrap_or_default();
        format!("error {} {}", run.code, normalize(last, root))
    } else {
        format!("ran {}", run.code)
    };
    format!("{line}\n")
}

/// A word with a cased letter and no lowercase one (`QUERY`).
fn is_upper(word: &str) -> bool {
    word.chars().any(char::is_uppercase) && !word.chars().any(char::is_lowercase)
}

/// The run's `Usage:` line.
fn usage_line(run: &Run, _root: &Path) -> String {
    let line = run
        .stdout
        .lines()
        .find(|line| line.starts_with("Usage: "))
        .unwrap_or_else(|| panic!("no usage line in {:?}", run.stdout));
    format!("{line}\n")
}

// --- building brains -----------------------------------------------------------

const DATE: &str = PINNED_GIT_DATE;

fn git(sb: &Sandbox, cwd: &Path, args: &[&str]) -> String {
    sb.git(cwd, args, DATE)
}

fn neuron(title: &str) -> String {
    format!("---\nparent: map.md\ncreated: 2026-01-01\nupdated: 2026-01-01\n---\n# {title}\n")
}

/// Two lobes (one nested), root files, and folders status never counts.
fn brain_files() -> Vec<(String, String)> {
    [
        ("brain.md", "# demo\n".to_string()),
        ("glossary.md", "# Glossary\n".to_string()),
        ("projects/map.md", "# Projects\n".to_string()),
        ("projects/alpha.md", neuron("Alpha")),
        ("projects/sub/beta.md", neuron("Beta")),
        ("decisions/map.md", "# Decisions\n".to_string()),
        ("decisions/d1.md", neuron("D1")),
        (".obsidian/notes.md", neuron("hidden")),
        ("node_modules/pkg/readme.md", neuron("vendored")),
        ("root-note.md", neuron("Root")),
    ]
    .into_iter()
    .map(|(path, text)| (path.to_string(), text))
    .collect()
}

fn write(root: &Path, files: &[(String, String)]) {
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    write_files(root, &files);
}

fn init(sb: &Sandbox, path: PathBuf) -> PathBuf {
    fs::create_dir_all(&path).unwrap();
    git(sb, &path, &["init", "-q"]);
    git(sb, &path, &["config", "core.autocrlf", "false"]);
    path
}

fn commit(sb: &Sandbox, repo: &Path, message: &str, files: &[(String, String)], date: &str) {
    write(repo, files);
    sb.git(repo, &["add", "-A"], date);
    sb.git(
        repo,
        &["commit", "-q", "--allow-empty", "-m", message],
        date,
    );
}

/// `demo` in `work/brains/demo`, its first commit `brain: initialize demo`
/// of [`brain_files`].
fn demo_brain(sb: &Sandbox) -> PathBuf {
    let brain = init(sb, sb.work.join("brains").join("demo"));
    commit(sb, &brain, "brain: initialize demo", &brain_files(), DATE);
    brain
}

fn register(sb: &Sandbox, brains: &[(&str, &Path)]) {
    register_described(sb, brains, "demo brain");
}

fn register_described(sb: &Sandbox, brains: &[(&str, &Path)], description: &str) {
    let brains: Vec<(&str, &Path, &str)> = brains
        .iter()
        .map(|(name, path)| (*name, *path, description))
        .collect();
    sb.write_config(&brains);
}

/// A git brain with 12 commits and a clean tree.
fn history_brain() -> Sandbox {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    for i in 1..12 {
        let alpha = [(
            "projects/alpha.md".to_string(),
            neuron(&format!("Alpha {i}")),
        )];
        let date = format!("2026-02-{i:02}T08:30:00+02:00");
        commit(&sb, &brain, &format!("update {i}"), &alpha, &date);
    }
    register(&sb, &[("demo", &brain)]);
    sb
}

/// Brains `a` (a lobe with one tagged neuron) and `b` (empty), no git.
fn two_brains() -> Sandbox {
    let sb = Sandbox::new();
    let a = sb.work.join("brains").join("a");
    let b = sb.work.join("brains").join("b");
    write_files(
        &a,
        &[
            ("brain.md", "# a\n"),
            ("lobe/map.md", "# Lobe\n"),
            (
                "lobe/needle.md",
                "---\nparent: map.md\ntags: [y]\nupdated: 2026-01-01\n---\n# Needle\n\nneedle text\n",
            ),
        ],
    );
    fs::create_dir_all(&b).unwrap();
    register(&sb, &[("a", &a), ("b", &b)]);
    sb
}

fn empty_sandbox() -> Sandbox {
    Sandbox::new()
}

// --- status ----------------------------------------------------------------------

#[test]
fn status_of_a_brain_with_history() {
    let sb = history_brain();
    golden(&sb, "status_json_history", &["status", "--json"]);
    golden(&sb, "status_text_history", &["status"]);
}

#[test]
fn status_shows_an_open_sync_conflict() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    register(&sb, &[("demo", &brain)]);
    let pending = brain.join(".git").join("kluris-sync").join("pending.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    let (a, b, c) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
    let (one, two, three) = ("1".repeat(40), "2".repeat(40), "3".repeat(40));
    fs::write(
        &pending,
        format!(
            "{{\"base\": \"{a}\", \"ours\": \"{b}\", \"theirs\": \"{c}\", \"conflicts\": [{{\"path\": \"lobe/a.md\", \"kind\": \"content\", \"base\": \"{one}\", \"ours\": \"{two}\", \"theirs\": \"{three}\"}}], \"created\": \"2026-01-01T00:00:00Z\"}}"
        ),
    )
    .unwrap();
    golden(&sb, "status_json_open_conflict", &["status", "--json"]);
    golden(&sb, "status_text_open_conflict", &["status"]);
}

#[test]
fn status_of_a_dirty_brain() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    // A modified tracked file sorts first, so the whole status text's
    // leading space is trimmed (" M brain.md" reads "M brain.md").
    write_files(
        &brain,
        &[
            ("brain.md", "# changed\n"),
            ("projects/alpha.md", "changed"),
            ("projects/new.md", &neuron("New")),
            ("sp ace.md", "x"),
        ],
    );
    git(&sb, &brain, &["add", "projects/alpha.md"]);
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_dirty", &["status", "--json"]);
    golden(&sb, "status_text_dirty", &["status"]);
}

#[test]
fn status_of_a_brain_without_git() {
    let sb = Sandbox::new();
    let brain = sb.work.join("brains").join("plain");
    write(&brain, &brain_files());
    register(&sb, &[("plain", &brain)]);
    golden(&sb, "status_json_without_git", &["status", "--json"]);
    golden(&sb, "status_text_without_git", &["status"]);
}

#[test]
fn status_of_every_brain_and_of_one() {
    let sb = Sandbox::new();
    let a = init(&sb, sb.work.join("brains").join("a"));
    commit(&sb, &a, "brain: initialize a", &brain_files(), DATE);
    write_files(&a, &[("projects/draft.md", "draft")]);
    let b = sb.work.join("brains").join("b");
    write_files(
        &b,
        &[
            ("brain.md", "# b\n"),
            ("only/map.md", "# Only\n"),
            ("only/n.md", &neuron("N")),
        ],
    );
    register(&sb, &[("a", &a), ("b", &b)]);
    golden(
        &sb,
        "status_json_brain_all",
        &["status", "--brain", "all", "--json"],
    );
    golden(&sb, "status_text_brain_all", &["status", "--brain", "all"]);
    golden(
        &sb,
        "status_json_brain_b",
        &["status", "--brain", "b", "--json"],
    );
}

fn two_empty_brains() -> Sandbox {
    let sb = Sandbox::new();
    let (a, b) = (sb.work.join("a"), sb.work.join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    register(&sb, &[("a", &a), ("b", &b)]);
    sb
}

#[test]
fn status_with_two_brains_and_no_choice() {
    golden_table(
        "status_with_two_brains_and_no_choice",
        &[&["status", "--json"], &["status"]],
        two_empty_brains,
        whole,
    );
}

#[test]
fn status_of_a_subject_with_pipes() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    let alpha = [("projects/alpha.md".to_string(), "x".to_string())];
    commit(&sb, &brain, "has | pipe | char", &alpha, DATE);
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_subject_with_pipes", &["status", "--json"]);
    golden(&sb, "status_text_subject_with_pipes", &["status"]);
}

#[test]
fn status_of_non_ascii_names() {
    let sb = Sandbox::new();
    let brain = init(&sb, sb.work.join("brains").join("demo"));
    let mut files = brain_files();
    files.push(("projects/ñandú.md".to_string(), neuron("Ñ")));
    commit(&sb, &brain, "brain: initialize demo", &files, DATE);
    write_files(
        &brain,
        &[
            ("projects/é.md", &neuron("E")),
            ("projects/ñandú.md", "changed"),
        ],
    );
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_non_ascii_names", &["status", "--json"]);
    golden(&sb, "status_text_non_ascii_names", &["status"]);
}

#[test]
fn status_of_a_nested_brain_is_scoped_to_the_brain() {
    let sb = Sandbox::new();
    let outer = init(&sb, sb.work.join("outer"));
    let readme = [("README.md".to_string(), "r\n".to_string())];
    commit(&sb, &outer, "outer: start", &readme, DATE);
    let nested: Vec<(String, String)> = brain_files()
        .into_iter()
        .map(|(path, text)| (format!("docs/brain/{path}"), text))
        .collect();
    commit(&sb, &outer, "brain: initialize demo", &nested, DATE);
    let code = [("src/main.rs".to_string(), "fn main() {}\n".to_string())];
    commit(&sb, &outer, "outer: code", &code, DATE);
    write_files(
        &outer,
        &[
            ("README.md", "changed\n"),
            ("docs/brain/projects/alpha.md", "changed"),
            ("docs/brain/n.md", "n"),
        ],
    );
    let brain = outer.join("docs").join("brain");
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_nested_brain", &["status", "--json"]);
    golden(&sb, "status_text_nested_brain", &["status"]);
}

#[test]
fn status_of_a_repo_without_commits() {
    let sb = Sandbox::new();
    let brain = init(&sb, sb.work.join("brains").join("demo"));
    write_files(&brain, &[("brain.md", "# demo\n")]);
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_without_commits", &["status", "--json"]);
}

#[test]
fn status_of_a_corrupt_repo() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    let head = git(&sb, &brain, &["rev-parse", "HEAD"]);
    let head = head.trim();
    let object = brain
        .join(".git")
        .join("objects")
        .join(&head[..2])
        .join(&head[2..]);
    // git writes loose objects read-only, which Windows will not remove.
    let mut permissions = fs::metadata(&object).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&object, permissions).unwrap();
    fs::remove_file(&object).unwrap();
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_corrupt_repo", &["status", "--json"]);
    golden(&sb, "status_text_corrupt_repo", &["status"]);
}

#[test]
fn status_when_the_branch_ref_is_broken() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    let branch = git(&sb, &brain, &["symbolic-ref", "HEAD"]);
    fs::write(brain.join(".git").join(branch.trim()), "invalid-oid\n").unwrap();
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_broken_ref", &["status", "--json"]);
}

#[test]
fn status_on_an_orphan_branch_beside_history() {
    let sb = Sandbox::new();
    let brain = demo_brain(&sb);
    git(&sb, &brain, &["checkout", "-q", "--orphan", "fresh"]);
    register(&sb, &[("demo", &brain)]);
    golden(&sb, "status_json_orphan_branch", &["status", "--json"]);
}

// --- human output prints user text literally ------------------------------------

#[test]
fn list_and_status_print_user_text_literally() {
    let sb = Sandbox::new();
    let brain = init(&sb, sb.work.join("brains").join("demo"));
    commit(
        &sb,
        &brain,
        "fix [bold]this[/bold] now",
        &brain_files(),
        DATE,
    );
    register_described(&sb, &[("demo", &brain)], "see [red]docs[/red]");
    golden(&sb, "list_text_markup_in_description", &["list"]);
    golden(&sb, "status_text_markup_in_subject", &["status"]);
}

#[test]
fn list_with_a_closing_tag_in_the_description() {
    let sb = Sandbox::new();
    let brain = sb.work.join("brains").join("demo");
    fs::create_dir_all(&brain).unwrap();
    register_described(&sb, &[("demo", &brain)], "ends [/oops]");
    golden(&sb, "list_text_closing_tag_in_description", &["list"]);
}

// --- list ------------------------------------------------------------------------

#[test]
fn list_of_three_brains() {
    let sb = Sandbox::new();
    let brains = sb.work.join("brains");
    write_files(
        &brains.join("alpha"),
        &[(
            "kluris.yml",
            "name: alpha\ncompanions:\n- specmint-core\n- specmint-tdd\n",
        )],
    );
    write_files(
        &brains.join("beta"),
        &[("kluris.yml", "name: beta\ncompanions: []\n")],
    );
    fs::create_dir_all(brains.join("gamma")).unwrap();
    register(
        &sb,
        &[
            ("alpha", &brains.join("alpha")),
            ("beta", &brains.join("beta")),
            ("gamma", &brains.join("gamma")),
        ],
    );
    golden(&sb, "list_text_three_brains", &["list"]);
    golden(&sb, "list_json_three_brains", &["list", "--json"]);
}

#[test]
fn list_without_brains() {
    golden(&Sandbox::new(), "list_text_without_brains", &["list"]);
}

#[test]
fn list_json_without_config() {
    let sb = Sandbox::new();
    assert!(!sb.config.exists());
    golden(&sb, "list_json_without_config", &["list", "--json"]);
}

/// Runs a setup step, which must succeed.
fn setup(sb: &Sandbox, args: &[&str]) {
    let run = sb.kluris(args);
    assert_eq!(
        run.code,
        0,
        "{}: {}",
        command_line(args),
        run.render(&sb.root)
    );
}

#[test]
fn list_json_of_two_brains_writes_nothing() {
    let sb = Sandbox::new();
    let brains = sb.work.join("brains");
    fs::create_dir_all(&brains).unwrap();
    let path = brains.display().to_string();
    setup(&sb, &["create", "a", "--path", &path, "--no-git", "--json"]);
    setup(&sb, &["create", "b", "--path", &path, "--json"]);
    setup(
        &sb,
        &[
            "companion",
            "add",
            "specmint-core",
            "--brain",
            "b",
            "--json",
        ],
    );
    let before = tree_dump(&sb.work);
    golden(&sb, "list_json_two_brains", &["list", "--json"]);
    assert_eq!(tree_dump(&sb.work), before, "list must not write anything");
}

// --- help ------------------------------------------------------------------------

/// Every command `help` lists, in its order.
const COMMANDS: &[&str] = &[
    "create",
    "register",
    "list",
    "status",
    "search",
    "wake-up",
    "companion",
    "dream",
    "mri",
    "remove",
    "doctor",
    "help",
    "commit",
    "sync",
    "resolve",
    "self-update",
    "self-uninstall",
];

/// The commands whose `help COMMAND --json` row is checked on its own.
const ONE_COMMAND_JSON: &[&str] = &["commit", "sync", "resolve", "self-update", "self-uninstall"];

#[test]
fn help_text_with_two_brains() {
    let sb = Sandbox::new();
    sb.write_config(&[
        ("a", &sb.work.join("a"), "x"),
        ("b", &sb.work.join("b"), "y"),
    ]);
    golden(&sb, "help_text", &["help"]);
}

#[test]
fn help_json() {
    golden(&Sandbox::new(), "help_json", &["help", "--json"]);
}

#[test]
fn help_for_each_command() {
    let rows: Vec<Vec<&str>> = COMMANDS.iter().map(|c| vec!["help", *c]).collect();
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    golden_table("help_for_each_command", &rows, empty_sandbox, whole);
}

#[test]
fn help_json_for_one_command() {
    let rows: Vec<Vec<&str>> = ONE_COMMAND_JSON
        .iter()
        .map(|c| vec!["help", *c, "--json"])
        .collect();
    let rows: Vec<&[&str]> = rows.iter().map(Vec::as_slice).collect();
    golden_table("help_json_for_one_command", &rows, empty_sandbox, whole);
}

#[test]
fn help_for_an_unknown_command() {
    golden_table(
        "help_for_an_unknown_command",
        &[&["help", "bogus"], &["help", "pack"]],
        empty_sandbox,
        whole,
    );
}

#[test]
fn help_json_with_a_command() {
    golden_table(
        "help_json_with_a_command",
        &[
            &["help", "status", "--json"],
            &["help", "bogus", "--json"],
            &["help", "pack", "--json"],
        ],
        empty_sandbox,
        whole,
    );
}

#[test]
fn help_with_a_corrupt_config() {
    let sb = Sandbox::new();
    fs::create_dir_all(sb.config.parent().unwrap()).unwrap();
    fs::write(&sb.config, "brains: [\n").unwrap();
    golden(&sb, "help_text_corrupt_config", &["help"]);
}

// --- the --help flag -------------------------------------------------------------

#[test]
fn command_help_flag() {
    golden_table(
        "command_help_flag",
        &[
            &["doctor", "--help"],
            &["help", "--help"],
            &["list", "--help"],
            &["status", "--help"],
            &["wake-up", "--help"],
        ],
        empty_sandbox,
        whole,
    );
}

#[test]
fn root_help_flag() {
    golden(&Sandbox::new(), "root_help_flag", &["--help"]);
}

// --- usage errors ----------------------------------------------------------------

#[test]
fn usage_errors() {
    golden_table(
        "usage_errors",
        &[
            &["-h"],
            &["-V"],
            &["status", "-h"],
            &["help", "-h"],
            &["help", "--bogus"],
            &["help", "a", "b"],
            &["bogus"],
            &["list", "-h"],
        ],
        empty_sandbox,
        whole,
    );
}

#[test]
fn usage_errors_with_json() {
    golden_table(
        "usage_errors_with_json",
        &[
            &["help", "--json", "bogus", "extra"],
            &["bogus", "--json"],
            &["pack", "--json"],
        ],
        empty_sandbox,
        whole,
    );
}

/// Every option may repeat, the last value winning.
const REPEATED: &[&[&str]] = &[
    &["list", "--json", "--json"],
    &["help", "--json", "--json"],
    &["help", "status", "--json", "--json"],
    &["status", "--brain", "b", "--brain", "a", "--json", "--json"],
    &[
        "wake-up", "--brain", "b", "--brain", "a", "--json", "--json",
    ],
    &[
        "search", "needle", "--brain", "b", "--brain", "a", "--json", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--lobe", "nope", "--lobe", "lobe", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--lobe", "lobe", "--lobe", "nope", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--tag", "x", "--tag", "y", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--tag", "y", "--tag", "x", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "0", "--limit", "1", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "1", "--limit", "0", "--json",
    ],
];

#[test]
fn a_repeated_option_takes_its_last_value() {
    golden_table("repeated_options", REPEATED, two_brains, whole);
}

/// Every token is parsed first (an unknown option, an option without its
/// value, a value given to a flag fail there, wherever `--help` is), then
/// the eager `--help` / `--version` act in the order given, before values
/// are converted, required arguments checked or extra ones refused. The
/// root's eager options act before the command is parsed, and the first
/// occurrence decides their order.
const PRECEDENCE: &[&[&str]] = &[
    &["list", "--help", "--bogus"],
    &["list", "--bogus", "--help"],
    &["list", "--help", "--json=1"],
    &["list", "--json=1", "--help"],
    &["list", "--help", "--help"],
    &["list", "--json", "--help", "--json"],
    &["status", "--help", "--brain"],
    &["status", "--brain", "x", "--help"],
    &["status", "--brain=x", "--help", "--bogus"],
    &["wake-up", "--help", "--brain"],
    &["search", "--help"],
    &["search", "needle", "--limit", "x", "--help"],
    &["search", "--limit", "x", "needle", "--help"],
    &["search", "needle", "--limit=x", "--help"],
    &["search", "needle", "--help", "extra"],
    &["search", "--help", "--limit"],
    &["search", "--tag", "--help"],
    &["help", "a", "b", "--help"],
    &["help", "bogus", "--help"],
    &["help", "--json", "--json", "--help"],
    &["--help", "--help"],
    &["--help", "list"],
    &["--help", "bogus"],
    &["--help", "--bogus"],
    &["bogus", "--help"],
    &["--version", "--version"],
    &["--version", "--help"],
    &["--help", "--version"],
    &["--version", "--bogus"],
    &["--bogus", "--version"],
    &["--version", "list"],
    &["--help", "list", "--bogus"],
    &["--version", "list", "--bogus"],
    &["--version", "bogus", "--bogus"],
    &["--help", "search"],
    &["--help", "--version", "--help"],
    &["--version", "--help", "--version"],
    &["--version", "--help", "--help", "--version"],
];

#[test]
fn help_and_usage_errors_in_their_order() {
    golden_table("precedence", PRECEDENCE, two_brains, outcome);
}

/// Every option that takes a value, repeated with an invalid value before or
/// after a valid one: only the value that wins is converted.
const REPEATED_VALUES: &[&[&str]] = &[
    &[
        "search", "needle", "--brain", "a", "--limit", "", "--limit", "2", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "invalid", "--limit", "2", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "2.0", "--limit", "2", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "2", "--limit", "x", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "2", "--limit", "x",
    ],
    &[
        "search", "needle", "--brain", "a", "--limit", "2", "--limit", "", "--json",
    ],
    &[
        "search", "needle", "--brain", "nope", "--brain", "a", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--brain", "nope", "--json",
    ],
    &["search", "needle", "--brain", "", "--brain", "a", "--json"],
    &[
        "search", "needle", "--brain", "a", "--lobe", "", "--lobe", "lobe", "--json",
    ],
    &[
        "search", "needle", "--brain", "a", "--tag", "", "--tag", "y", "--json",
    ],
    &["status", "--brain", "nope", "--brain", "a", "--json"],
    &["status", "--brain", "a", "--brain", "nope", "--json"],
    &["status", "--brain", "a", "--brain", "nope"],
    &["status", "--brain", "", "--brain", "all", "--json"],
    &["wake-up", "--brain", "nope", "--brain", "a", "--json"],
    &["wake-up", "--brain", "a", "--brain", "nope", "--json"],
    &["wake-up", "--brain", "all", "--brain", "a", "--json"],
];

#[test]
fn only_the_winning_value_is_converted() {
    golden_table("repeated_values", REPEATED_VALUES, two_brains, whole);
}

#[test]
fn usage_lines() {
    golden_table(
        "usage_lines",
        &[
            &["--help"],
            &["list", "--help"],
            &["status", "--help"],
            &["wake-up", "--help"],
            &["search", "--help"],
            &["help", "--help"],
            &["companion", "--help"],
            &["companion", "list", "--help"],
        ],
        two_brains,
        usage_line,
    );
}

#[test]
fn usage_line_of_a_usage_error() {
    golden(&two_brains(), "usage_error_missing_query", &["search"]);
}
