//! Golden tests of `kluris companion list|add|remove` and of the brain
//! resolver: each run's exit code and streams and, for a command that
//! writes, what it changed under the sandbox's `work/`.
//!
//! The companion scenarios start from a state in `tests/fixtures/companions/`,
//! a home as the companion commands find it before they migrate it: `base/`
//! holds brains `a` and `b` (their `kluris.yml` lists the `windsurf` and
//! `gemini` agents), the config, one SKILL.md per brain and the Windsurf
//! workflows; each other folder holds the files that differ once brains
//! opted into companions. Names that start with `.` are stored as
//! `dot-<name>` (see [`unstored`]). [`state`] fills the paths in, copies each
//! SKILL.md into every per-agent folder of that home, installs the
//! opted-in companions from `vendored/` and makes `a` a git repository
//! with one commit.
//!
//! The brain and companion pickers prompt only when stdin is a terminal:
//! [`pickers`] runs them with stdin on a pseudo-terminal (Unix only).

mod common;

use std::fs;
use std::path::Path;

use common::shared::{COMPANIONS, skill_reference, vendored, vendored_marker};
use common::{
    Entry, Form, PINNED_GIT_DATE, Run, Sandbox, Tree, changes_dump, normalize, run_end, section,
    tree_files, unstored, windows_variant, write,
};

/// The fixture states: `base`, or `base` with the overlay of that name, and
/// the companions its brains opted into.
const STATES: &[(&str, &[&str])] = &[
    ("base", &[]),
    ("a-core", &["specmint-core"]),
    ("a-tdd", &["specmint-tdd"]),
    ("a-core-tdd", &["specmint-core", "specmint-tdd"]),
    ("a-tdd-html-core", &["specmint-tdd-html", "specmint-core"]),
    ("b-core-html", &["specmint-core-html"]),
    ("a-core-b-tdd-html", &["specmint-core", "specmint-tdd-html"]),
];

/// The per-agent folders that hold the same SKILL.md as `.claude/skills`
/// in every fixture state.
const SKILL_COPIES: [&str; 9] = [
    ".agents/skills",
    ".codeium/windsurf/skills",
    ".codex/skills",
    ".copilot/skills",
    ".cursor/skills",
    ".gemini/skills",
    ".hermes/skills",
    ".junie/skills",
    ".kilo/skills",
];

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A sandbox in the fixture state `name` (see the module docs).
fn state(name: &str) -> Sandbox {
    let companions = STATES
        .iter()
        .find(|(state, _)| *state == name)
        .map(|(_, companions)| *companions)
        .unwrap_or_else(|| panic!("no fixture state {name}"));
    let sb = Sandbox::new();
    let brain = |b: &str| sb.work.join("brains").join(b).display().to_string();
    let fill = |bytes: Vec<u8>| {
        let text = String::from_utf8(bytes).unwrap_or_else(|e| panic!("fixture: {e}"));
        text.replace("<BRAIN a>", &brain("a"))
            .replace("<BRAIN b>", &brain("b"))
            .replace("<HOME>", &sb.home.display().to_string())
    };
    let fixtures = manifest_dir().join("tests/fixtures/companions");
    let mut layers = vec!["base"];
    if name != "base" {
        layers.push(name);
    }
    for layer in layers {
        for (rel, entry) in tree_files(&fixtures.join(layer)) {
            write(
                &sb.work.join(unstored(&rel)),
                fill(entry.file().to_vec()).as_bytes(),
            );
        }
    }
    for b in ["a", "b"] {
        let skill = format!("kluris-{b}/SKILL.md");
        let text = read(&sb.home.join(".claude/skills").join(&skill));
        for folder in SKILL_COPIES {
            write(&sb.home.join(folder).join(&skill), &text);
        }
    }
    for companion in companions {
        let installed = sb.home.join(".kluris/companions").join(companion);
        for (rel, entry) in tree_files(&vendored().join(companion)) {
            write(&installed.join(rel), entry.file());
        }
    }
    let a = sb.work.join("brains").join("a");
    sb.git(&a, &["init", "-q", "-b", "main"], PINNED_GIT_DATE);
    sb.git(&a, &["add", "-A"], PINNED_GIT_DATE);
    sb.git(
        &a,
        &["commit", "-q", "-m", "brain: initialize a"],
        PINNED_GIT_DATE,
    );
    sb
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The snapshots with a Windows variant: a missing `kluris.yml` is
/// reported with the OS's own error text ("The system cannot find the file
/// specified." on Windows).
const WINDOWS_VARIANTS: &[&str] = &[
    "add_kluris_yml_missing_json",
    "add_kluris_yml_missing_text",
    "remove_kluris_yml_missing_json",
    "remove_kluris_yml_missing_text",
];

/// Snapshot `run` as `name` (its Windows variant when [`WINDOWS_VARIANTS`]
/// names it), the configured paths it prints checked in the OS's form and
/// shown `/`-joined ([`Sandbox::shown`]).
fn check(sb: &Sandbox, name: &str, run: &Run) {
    let text = sb.shown(run).render(&sb.root);
    let check = || insta::assert_snapshot!(name, text);
    if WINDOWS_VARIANTS.contains(&name) {
        windows_variant(check);
    } else {
        check();
    }
}

/// Snapshot what a run changed under `work/` as `name`, as `changes_dump`
/// writes it. A runtime companion file that holds its vendored bytes
/// reads `<vendored/NAME/PATH>`; a brain's SKILL.md reads `<snapshot
/// skill_BRAIN_COMPANIONS>`, its text checked against that snapshot (one
/// per brain and companion set, shared by every scenario that writes it).
fn check_changes(sb: &Sandbox, name: &str, before: &Tree) {
    let out = changes_dump(
        before,
        &tree_files(&sb.work),
        |text| normalize(text, &sb.root, Form::Dumped),
        |path, entry| reference(sb, path, entry),
    );
    insta::assert_snapshot!(name, out);
}

fn reference(sb: &Sandbox, path: &str, entry: &Entry) -> Option<common::Reference> {
    vendored_marker(path, entry)
        .or_else(|| skill_reference(path, entry, &sb.root, |_| String::new()))
}

fn assert_unchanged(sb: &Sandbox, before: &Tree) {
    common::assert_unchanged(before, &tree_files(&sb.work), "the run");
}

/// A snapshot name for `args`: their letters and digits, each run of
/// anything else as one `_`.
fn slug(args: &[&str]) -> String {
    args.join(" ")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

// --- companion list -----------------------------------------------------------

#[test]
fn companion_list_without_brains() {
    let sb = Sandbox::new();
    for (name, args) in [
        (
            "list_without_brains_json",
            &["companion", "list", "--json"][..],
        ),
        ("list_without_brains_text", &["companion", "list"][..]),
    ] {
        check(&sb, name, &sb.kluris(args));
    }
}

/// `a` with two companions, `b` without, `c` with no `kluris.yml`, `d` with
/// one that does not parse, and a stray file at a companion's path.
fn list_scenario() -> Sandbox {
    let sb = state("a-tdd-html-core");
    let brains = sb.work.join("brains");
    fs::create_dir(brains.join("c")).unwrap_or_else(|e| panic!("{e}"));
    write(&brains.join("d").join("kluris.yml"), b"name: [broken\n");
    let mut config = String::from_utf8(read(&sb.config)).unwrap_or_else(|e| panic!("{e}"));
    for name in ["c", "d"] {
        let path = brains.join(name).display().to_string();
        let path = serde_json::to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        config.push_str(&format!(
            "  {name}:\n    path: {path}\n    description: ''\n"
        ));
    }
    write(&sb.config, config.as_bytes());
    write(&sb.home.join(".kluris/companions/specmint-tdd"), b"stray");
    sb
}

#[test]
fn companion_list_json() {
    let sb = list_scenario();
    let before = tree_files(&sb.work);
    for (name, brain) in [
        ("list_json", &[][..]),
        ("list_json_all", &["--brain", "all"]),
        ("list_json_a", &["--brain", "a"]),
        ("list_json_c", &["--brain", "c"]),
    ] {
        let args = [&["companion", "list"][..], brain, &["--json"]].concat();
        check(&sb, name, &sb.kluris(&args));
        assert_unchanged(&sb, &before);
    }
}

#[test]
fn companion_list_text() {
    let sb = list_scenario();
    check(&sb, "list_text", &sb.kluris(&["companion", "list"]));
}

#[test]
fn companion_list_with_an_unknown_brain() {
    let sb = state("base");
    for (name, json) in [("json", &["--json"][..]), ("text", &[])] {
        let args = [&["companion", "list", "--brain", "nope"][..], json].concat();
        check(
            &sb,
            &format!("list_unknown_brain_{name}"),
            &sb.kluris(&args),
        );
    }
}

// --- companion add ------------------------------------------------------------

#[test]
fn companion_add_json() {
    for brain in ["a", "b", "all"] {
        for companion in COMPANIONS {
            let sb = state("base");
            let before = tree_files(&sb.work);
            let run = sb.kluris(&["companion", "add", companion, "--brain", brain, "--json"]);
            let name = format!("add_json_{companion}_{brain}");
            check(&sb, &name, &run);
            check_changes(&sb, &format!("{name}_changes"), &before);
        }
    }
}

#[test]
fn companion_add_text_with_an_existing_opt_in() {
    let sb = state("a-tdd");
    let before = tree_files(&sb.work);
    let run = sb.kluris(&["companion", "add", "specmint-tdd", "--brain", "all"]);
    check(&sb, "add_text_existing_opt_in", &run);
    check_changes(&sb, "add_text_existing_opt_in_changes", &before);
}

#[test]
fn companion_add_refreshes_an_old_single_file_install() {
    let sb = state("base");
    let companions = sb.home.join(".kluris/companions");
    write(
        &companions.join("specmint-core/SKILL.md"),
        b"old single-file playbook",
    );
    write(&companions.join(".specmint-core.old"), b"stray");
    let before = tree_files(&sb.work);
    let run = sb.kluris(&[
        "companion",
        "add",
        "specmint-core",
        "--brain",
        "a",
        "--json",
    ]);
    check(&sb, "add_refreshes_old_install", &run);
    check_changes(&sb, "add_refreshes_old_install_changes", &before);
}

// --- companion remove ---------------------------------------------------------

#[test]
fn companion_remove_json_from_the_brain_that_has_it() {
    let sb = state("a-core-tdd");
    let before = tree_files(&sb.work);
    let run = sb.kluris(&[
        "companion",
        "remove",
        "specmint-core",
        "--brain",
        "a",
        "--json",
    ]);
    check(&sb, "remove_json", &run);
    check_changes(&sb, "remove_json_changes", &before);
}

#[test]
fn companion_remove_text() {
    let sb = state("b-core-html");
    let before = tree_files(&sb.work);
    let run = sb.kluris(&["companion", "remove", "specmint-core-html", "--brain", "b"]);
    check(&sb, "remove_text", &run);
    check_changes(&sb, "remove_text_changes", &before);
}

/// `--brain all` rewrites only the brains that had the companion: `b`'s
/// `kluris.yml`, with its comment, stays as it was.
#[test]
fn companion_remove_all_touches_only_brains_that_had_it() {
    for (name, args) in [
        (
            "remove_all_had_it",
            &[
                "companion",
                "remove",
                "specmint-core",
                "--brain",
                "all",
                "--json",
            ][..],
        ),
        (
            "remove_all_none_had_it",
            &["companion", "remove", "specmint-tdd", "--brain", "all"][..],
        ),
    ] {
        let sb = state("a-core");
        let yml = sb.work.join("brains/b/kluris.yml");
        let text = [&b"# my notes\n"[..], &read(&yml)].concat();
        write(&yml, &text);
        let before = tree_files(&sb.work);
        check(&sb, name, &sb.kluris(args));
        check_changes(&sb, &format!("{name}_changes"), &before);
    }
}

// --- companion errors ---------------------------------------------------------

#[test]
fn companion_name_required_when_not_interactive() {
    let sb = state("base");
    let before = tree_files(&sb.work);
    for command in ["add", "remove"] {
        for (flag, json) in [("json", &["--json"][..]), ("text", &[])] {
            let args = [&["companion", command, "--brain", "a"][..], json].concat();
            check(
                &sb,
                &format!("{command}_name_required_{flag}"),
                &sb.kluris(&args),
            );
            assert_unchanged(&sb, &before);
        }
    }
}

#[test]
fn companion_invalid_name() {
    let sb = state("base");
    for command in ["add", "remove"] {
        for (flag, json) in [("json", &["--json"][..]), ("text", &[])] {
            let args = [&["companion", command, "bogus", "--brain", "a"][..], json].concat();
            check(
                &sb,
                &format!("{command}_invalid_name_{flag}"),
                &sb.kluris(&args),
            );
        }
    }
}

const USAGE_ERRORS_JSON: &[&[&str]] = &[
    &[
        "companion",
        "add",
        "--brain",
        "a",
        "specmint-core",
        "extra",
        "--json",
    ],
    &["companion", "list", "x", "--json"],
    &["companion", "bogus", "--json"],
    &["companion", "--json"],
    &["companion", "add", "--brain", "--json"],
    &["companion", "add", "specmint-core", "--json"],
    &[
        "companion",
        "remove",
        "specmint-core",
        "--brain",
        "all",
        "--brain",
        "nope",
        "--json",
    ],
];

const USAGE_ERRORS: &[&[&str]] = &[
    &["companion", "bogus"],
    &["companion", "list", "x"],
    &["companion", "add", "--brain"],
    &["companion", "add", "specmint-core"],
];

#[test]
fn companion_usage_and_resolver_errors() {
    let sb = state("base");
    let before = tree_files(&sb.work);
    for args in USAGE_ERRORS_JSON {
        check(&sb, &format!("usage_{}", slug(args)), &sb.kluris(args));
        assert_unchanged(&sb, &before);
    }
    for args in USAGE_ERRORS {
        check(&sb, &format!("usage_{}", slug(args)), &sb.kluris(args));
    }
}

#[test]
fn bare_companion_prints_its_help_and_exits_2() {
    let sb = Sandbox::new();
    check(&sb, "bare_companion", &sb.kluris(&["companion"]));
}

#[test]
fn companion_help() {
    let sb = Sandbox::new();
    for sub in [&[][..], &["list"], &["add"], &["remove"]] {
        let args = [&["companion"][..], sub, &["--help"]].concat();
        check(&sb, &format!("help_{}", slug(&args)), &sb.kluris(&args));
    }
}

/// `b`'s `kluris.yml` is missing or does not parse: the command stops with
/// an error before it writes anything.
#[test]
fn companion_a_brain_whose_kluris_yml_does_not_read() {
    for damage in ["missing", "invalid"] {
        let sb = state("a-core");
        let yml = sb.work.join("brains/b/kluris.yml");
        if damage == "missing" {
            fs::remove_file(&yml).unwrap_or_else(|e| panic!("{e}"));
        } else {
            write(&yml, b"name: [broken\n");
        }
        let before = tree_files(&sb.work);
        for command in ["add", "remove"] {
            for (flag, json) in [("json", &["--json"][..]), ("text", &[])] {
                let args = [
                    &["companion", command, "specmint-tdd-html", "--brain", "all"][..],
                    json,
                ]
                .concat();
                let run = sb.kluris(&args);
                check(&sb, &format!("{command}_kluris_yml_{damage}_{flag}"), &run);
                assert_unchanged(&sb, &before);
            }
        }
    }
}

// --- the resolver ---------------------------------------------------------------

/// What a registered brain's path holds.
#[derive(Clone, Copy)]
enum Kind {
    Dir,
    File,
    Missing,
}

use Kind::{Dir, File, Missing};

/// The brains a resolver case starts from.
#[derive(Clone, Copy)]
enum Brains {
    /// No config file at all.
    NoConfig,
    /// A config with these `(name, kind)` brains, in order, at
    /// `work/brains/brain-<index>`, each described as `<name> brain`.
    Registered(&'static [(&'static str, Kind)]),
    /// `café` (an existing folder `work/brains/café`, described `déjà`) and
    /// `b` (`work/brains/nope`, not there).
    Unicode,
}

/// What a resolver case's snapshot holds of a run.
#[derive(Clone, Copy)]
enum Out {
    /// The exit code and both streams.
    Whole,
    /// The exit code and the `name` of the JSON on stdout.
    Name,
}

/// Environment changes: a value, or `None` to remove the variable.
type Env = &'static [(&'static str, Option<&'static str>)];

struct Case {
    name: String,
    brains: Brains,
    args: Vec<&'static str>,
    env: Env,
    out: Out,
}

impl Case {
    fn new(name: impl Into<String>, brains: Brains, args: &[&'static str]) -> Self {
        Self {
            name: name.into(),
            brains,
            args: args.to_vec(),
            env: &[],
            out: Out::Whole,
        }
    }

    fn env(mut self, env: Env) -> Self {
        self.env = env;
        self
    }

    fn name_only(mut self) -> Self {
        self.out = Out::Name;
        self
    }
}

const NONE: Brains = Brains::Registered(&[]);
const A: Brains = Brains::Registered(&[("a", Dir)]);
const AB: Brains = Brains::Registered(&[("a", Dir), ("b", Dir)]);
const NO_PROMPT_UNSET: Env = &[("KLURIS_NO_PROMPT", None)];

/// A single-brain command (`wake-up`) and an `--brain all` one (`status`).
const SINGLE: &str = "wake-up";
const FAN: &str = "status";

fn with(command: &'static str, args: &[&'static str]) -> Vec<&'static str> {
    [&[command][..], args].concat()
}

fn resolver_cases() -> Vec<Case> {
    let mut cases = vec![
        Case::new(
            "all_refused_on_single_json",
            AB,
            &[SINGLE, "--brain", "all", "--json"],
        ),
        Case::new(
            "all_refused_on_single_text",
            AB,
            &[SINGLE, "--brain", "all"],
        ),
        Case::new(
            "all_refused_before_counting",
            NONE,
            &[SINGLE, "--brain", "all", "--json"],
        ),
        Case::new(
            "all_with_zero_brains_json",
            NONE,
            &[FAN, "--brain", "all", "--json"],
        ),
        Case::new("all_with_zero_brains_text", NONE, &[FAN, "--brain", "all"]),
        Case::new(
            "all_in_config_order",
            Brains::Registered(&[("zeta", Dir), ("alpha", Dir), ("mid", Dir)]),
            &[FAN, "--brain", "all", "--json"],
        ),
    ];
    const BAD_PATHS: Brains = Brains::Registered(&[("ok", Dir), ("gone", Missing), ("file", File)]);
    cases.push(Case::new(
        "all_with_bad_paths_json",
        BAD_PATHS,
        &[FAN, "--brain", "all", "--json"],
    ));
    cases.push(Case::new(
        "all_with_bad_paths_text",
        BAD_PATHS,
        &[FAN, "--brain", "all"],
    ));

    const COUNTS: [(&str, Brains); 3] = [("0", NONE), ("1", A), ("2", AB)];
    for (count, brains) in COUNTS {
        for (kind, command) in [("single", SINGLE), ("fan", FAN)] {
            for (flag, json) in [("json", &["--json"][..]), ("text", &[])] {
                let args = [&[command, "--brain", "nope"][..], json].concat();
                let name = format!("unknown_name_{count}_brains_{kind}_{flag}");
                cases.push(Case::new(name, brains, &args));
            }
        }
    }

    const ABC: Brains = Brains::Registered(&[("a", Dir), ("b", Dir), ("c", Dir)]);
    const ONLY: Brains = Brains::Registered(&[("only", Dir)]);
    cases.extend([
        Case::new(
            "names_match_exactly",
            Brains::Registered(&[("Demo", Dir)]),
            &[SINGLE, "--brain", "demo", "--json"],
        ),
        Case::new("named_brain_fan", ABC, &[FAN, "--brain", "b", "--json"]),
        Case::new(
            "named_brain_single",
            ABC,
            &[SINGLE, "--brain", "b", "--json"],
        )
        .name_only(),
        Case::new(
            "last_brain_option_wins_fan",
            AB,
            &[FAN, "--brain", "a", "--brain=b", "--json", "--json"],
        ),
        Case::new(
            "last_brain_option_wins_single",
            AB,
            &[SINGLE, "--brain", "nope", "--brain", "a", "--json"],
        )
        .name_only(),
        Case::new(
            "empty_name_one_brain",
            ONLY,
            &[FAN, "--brain", "", "--json"],
        ),
        Case::new(
            "empty_name_one_brain_equals",
            ONLY,
            &[FAN, "--brain=", "--json"],
        ),
        Case::new("empty_name_two_brains", AB, &[FAN, "--brain", "", "--json"]),
        // The next token is the value, even `--json`; the envelope still
        // applies because `--json` is on the command line.
        Case::new("option_like_value", A, &[SINGLE, "--brain", "--json"]),
        Case::new(
            "brain_without_value_json",
            A,
            &[SINGLE, "--json", "--brain"],
        ),
        Case::new("brain_without_value_text", A, &[SINGLE, "--brain"]),
    ]);

    for (kind, command) in [("single", SINGLE), ("fan", FAN)] {
        for (flag, json) in [("json", &["--json"][..]), ("text", &[])] {
            let name = format!("zero_brains_{kind}_{flag}");
            cases.push(Case::new(name, NONE, &with(command, json)));
        }
    }
    cases.push(Case::new(
        "no_config_file",
        Brains::NoConfig,
        &[SINGLE, "--json"],
    ));

    // Even with stdin a pipe and prompting allowed.
    const SOLO: Brains = Brains::Registered(&[("solo", Dir)]);
    cases.push(Case::new("exactly_one_brain_fan", SOLO, &[FAN, "--json"]).env(NO_PROMPT_UNSET));
    cases.push(
        Case::new("exactly_one_brain_single", SOLO, &[SINGLE, "--json"])
            .env(NO_PROMPT_UNSET)
            .name_only(),
    );

    const SOLO_MISSING: Brains = Brains::Registered(&[("solo", Missing)]);
    const SOLO_FILE: Brains = Brains::Registered(&[("solo", File)]);
    for (kind, brains) in [("missing", SOLO_MISSING), ("file", SOLO_FILE)] {
        for (how, args) in [
            ("single_json", &[SINGLE, "--json"][..]),
            ("single_text", &[SINGLE]),
            ("fan_named_json", &[FAN, "--brain", "solo", "--json"]),
        ] {
            cases.push(Case::new(format!("one_brain_{kind}_{how}"), brains, args));
        }
    }

    const SEVERAL: Brains = Brains::Registered(&[("b", Dir), ("a", Missing), ("c", Dir)]);
    const HOWS: [(&str, &[&str], Env); 6] = [
        ("json", &["--json"], &[]),
        ("json_prompt_allowed", &["--json"], NO_PROMPT_UNSET),
        ("no_prompt", &[], &[]),
        ("pipe", &[], NO_PROMPT_UNSET),
        ("no_prompt_true", &[], &[("KLURIS_NO_PROMPT", Some("true"))]),
        ("no_prompt_0", &[], &[("KLURIS_NO_PROMPT", Some("0"))]),
    ];
    for (kind, command) in [("single", SINGLE), ("fan", FAN)] {
        for (how, args, env) in HOWS {
            let name = format!("several_brains_{kind}_{how}");
            cases.push(Case::new(name, SEVERAL, &with(command, args)).env(env));
        }
    }

    cases.push(Case::new(
        "unicode_several",
        Brains::Unicode,
        &[SINGLE, "--json"],
    ));
    cases.push(Case::new(
        "unicode_named",
        Brains::Unicode,
        &[SINGLE, "--brain", "café", "--json"],
    ));
    cases
}

/// A sandbox with `brains` registered.
fn resolver_sandbox(brains: Brains) -> Sandbox {
    let sb = Sandbox::new();
    let root = sb.work.join("brains");
    match brains {
        Brains::NoConfig => {}
        Brains::Registered(spec) => {
            fs::create_dir_all(&root).unwrap_or_else(|e| panic!("{e}"));
            let mut rows = Vec::new();
            for (index, (name, kind)) in spec.iter().enumerate() {
                let path = root.join(format!("brain-{index}"));
                match kind {
                    Dir => fs::create_dir(&path).unwrap_or_else(|e| panic!("{e}")),
                    File => write(&path, b"x"),
                    Missing => {}
                }
                rows.push((*name, path, format!("{name} brain")));
            }
            let rows: Vec<(&str, &Path, &str)> = rows
                .iter()
                .map(|(n, p, d)| (*n, p.as_path(), d.as_str()))
                .collect();
            sb.write_config(&rows);
        }
        Brains::Unicode => {
            let cafe = root.join("café");
            fs::create_dir_all(&cafe).unwrap_or_else(|e| panic!("{e}"));
            sb.write_config(&[("café", &cafe, "déjà"), ("b", &root.join("nope"), "")]);
        }
    }
    sb
}

#[test]
fn resolver_table() {
    for case in resolver_cases() {
        let sb = resolver_sandbox(case.brains);
        let mut cmd = sb.command(&case.args);
        for (key, value) in case.env {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
        let run = Run::of(cmd);
        let name = format!("resolver_{}", case.name);
        match case.out {
            Out::Whole => check(&sb, &name, &run),
            Out::Name => {
                let doc: serde_json::Value = serde_json::from_str(&run.stdout)
                    .unwrap_or_else(|e| panic!("{name}: stdout is not JSON ({e}): {run:?}"));
                let brain = doc["name"].as_str().unwrap_or("<no name>");
                insta::assert_snapshot!(
                    name.as_str(),
                    format!(
                        "{}{}",
                        section("name", &format!("{brain}\n")),
                        run_end(run.code)
                    )
                );
            }
        }
    }
}

/// Stdin is the null device (`/dev/null`, `NUL` on Windows) and prompting
/// is allowed: it is not a terminal on any OS, so this is the
/// non-interactive error.
#[test]
fn resolver_null_stdin() {
    for (kind, command) in [("single", SINGLE), ("fan", FAN)] {
        let sb = resolver_sandbox(AB);
        let mut cmd = sb.command(&[command]);
        cmd.env_remove("KLURIS_NO_PROMPT")
            .stdin(std::process::Stdio::null());
        check(&sb, &format!("resolver_null_stdin_{kind}"), &Run::of(cmd));
    }
}

// --- the pickers (stdin on a terminal) ------------------------------------------

#[cfg(unix)]
mod pickers {
    use super::*;
    use common::pty::{End, run_on_terminal};

    /// Where a picker case starts.
    enum Start {
        Brains(Brains),
        State(&'static str),
    }

    /// What a picker case checks of the sandbox after the run.
    enum Tree {
        /// Nothing: the command only reads.
        Unchecked,
        /// The run changed nothing.
        Unchanged,
        /// The run's changes, as a snapshot `<name>_changes`.
        Changes,
    }

    struct Picker {
        name: &'static str,
        start: Start,
        args: &'static [&'static str],
        lines: &'static [&'static str],
        end: End,
        env: Env,
        tree: Tree,
    }

    const AB_MISSING: Brains = Brains::Registered(&[("a", Dir), ("b", Missing)]);
    const ADD_A: &[&str] = &["companion", "add", "--brain", "a"];
    const REMOVE_A: &[&str] = &["companion", "remove", "--brain", "a"];
    const REMOVE_ALL: &[&str] = &["companion", "remove", "--brain", "all"];

    /// Each picker case: prompting allowed (`KLURIS_NO_PROMPT` unset) unless
    /// `env` sets it.
    #[rustfmt::skip]
    const PICKERS: &[Picker] = &[
        // The brain picker.
        Picker { name: "brain_picker_wake_up_picks_b", start: Start::Brains(AB), args: &[SINGLE], lines: &["2"], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker {
            name: "brain_picker_reprompts_on_invalid_input",
            start: Start::Brains(Brains::Registered(&[("a", Dir), ("b", Missing), ("c", File)])),
            args: &[FAN],
            lines: &["abc", "", " 'x", "0", "9", "1_0", "1__0", " ", "１"],
            end: End::Open, env: &[], tree: Tree::Unchecked,
        },
        Picker { name: "brain_picker_all_is_every_brain", start: Start::Brains(AB), args: &[FAN, "--brain", ""], lines: &["+3"], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_choice_is_path_checked_missing", start: Start::Brains(AB_MISSING), args: &[FAN], lines: &["2"], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_choice_is_path_checked_all", start: Start::Brains(AB_MISSING), args: &[FAN], lines: &["3"], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_wake_up_picks_a_missing_brain", start: Start::Brains(AB_MISSING), args: &[SINGLE], lines: &["2"], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_end_of_input", start: Start::Brains(AB), args: &[SINGLE], lines: &[], end: End::Eof, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_end_of_input_after_input", start: Start::Brains(AB), args: &[SINGLE], lines: &["x"], end: End::Eof, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_interrupt", start: Start::Brains(AB), args: &[SINGLE], lines: &[], end: End::Interrupt, env: &[], tree: Tree::Unchecked },
        Picker { name: "brain_picker_no_prompt_yes_prompts", start: Start::Brains(AB), args: &[FAN], lines: &["1"], end: End::Open, env: &[("KLURIS_NO_PROMPT", Some("yes"))], tree: Tree::Unchecked },
        Picker { name: "terminal_with_json_lists_the_brains", start: Start::Brains(AB), args: &[FAN, "--json"], lines: &[], end: End::Open, env: &[], tree: Tree::Unchecked },
        Picker { name: "terminal_with_no_prompt_lists_the_brains", start: Start::Brains(AB), args: &[FAN], lines: &[], end: End::Open, env: &[("KLURIS_NO_PROMPT", Some("1"))], tree: Tree::Unchecked },
        // The companion pickers.
        Picker { name: "add_picker_core_and_tdd", start: Start::State("base"), args: ADD_A, lines: &["3"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "add_picker_reprompts_then_default", start: Start::State("base"), args: ADD_A, lines: &["9", "", "x"], end: End::Open, env: &[], tree: Tree::Unchanged },
        Picker { name: "add_picker_default", start: Start::State("base"), args: ADD_A, lines: &["", "7"], end: End::Open, env: &[], tree: Tree::Unchanged },
        Picker { name: "add_picker_all", start: Start::State("base"), args: ADD_A, lines: &["7"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "add_picker_tdd_html", start: Start::State("base"), args: ADD_A, lines: &["6"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "add_picker_space_then_core_html", start: Start::State("base"), args: ADD_A, lines: &[" 3", "5"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "add_picker_after_the_brain_picker", start: Start::State("base"), args: &["companion", "add"], lines: &["3", "2"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "add_picker_end_of_input", start: Start::State("base"), args: ADD_A, lines: &[], end: End::Eof, env: &[], tree: Tree::Unchanged },
        Picker { name: "remove_confirm_default_yes", start: Start::State("a-tdd"), args: REMOVE_A, lines: &[""], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "remove_confirm_y", start: Start::State("a-tdd"), args: REMOVE_A, lines: &["y"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "remove_confirm_junk_then_yes", start: Start::State("a-tdd"), args: REMOVE_A, lines: &["junk", " YES "], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "remove_confirm_n", start: Start::State("a-tdd"), args: REMOVE_A, lines: &["n"], end: End::Open, env: &[], tree: Tree::Unchanged },
        Picker { name: "remove_confirm_no", start: Start::State("a-tdd"), args: REMOVE_A, lines: &["No"], end: End::Open, env: &[], tree: Tree::Unchanged },
        Picker { name: "remove_picker_tdd_html", start: Start::State("a-core-b-tdd-html"), args: REMOVE_ALL, lines: &["2"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "remove_picker_reprompts_then_all", start: Start::State("a-core-b-tdd-html"), args: REMOVE_ALL, lines: &["0", "x", "", "3"], end: End::Open, env: &[], tree: Tree::Changes },
        Picker { name: "remove_picker_cancel", start: Start::State("a-core-b-tdd-html"), args: REMOVE_ALL, lines: &["4"], end: End::Open, env: &[], tree: Tree::Unchanged },
        Picker { name: "remove_picker_nothing_enabled", start: Start::State("base"), args: REMOVE_ALL, lines: &[], end: End::Open, env: &[], tree: Tree::Unchanged },
    ];

    #[test]
    fn pickers_on_a_terminal() {
        for case in PICKERS {
            let sb = match case.start {
                Start::Brains(brains) => resolver_sandbox(brains),
                Start::State(name) => state(name),
            };
            let before = tree_files(&sb.work);
            let mut cmd = sb.command(case.args);
            cmd.env_remove("KLURIS_NO_PROMPT");
            for (key, value) in case.env {
                match value {
                    Some(value) => cmd.env(key, value),
                    None => cmd.env_remove(key),
                };
            }
            let run = run_on_terminal(cmd, case.lines, case.end);
            let name = format!("picker_{}", case.name);
            check(&sb, &name, &run);
            match case.tree {
                Tree::Unchecked => {}
                Tree::Unchanged => assert_unchanged(&sb, &before),
                Tree::Changes => check_changes(&sb, &format!("{name}_changes"), &before),
            }
        }
    }
}
