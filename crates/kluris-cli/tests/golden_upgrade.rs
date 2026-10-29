//! Golden tests of Kluris on a home an earlier Kluris set up (the frozen
//! worlds of `common::upgrade`): the skill install, `doctor`, and the first
//! commands a user runs, with what each prints and every file it changes;
//! and the POSIX path rule the skills write paths with, against the frozen
//! rows of the earlier Kluris's answers.
//!
//! The worlds, each as the earlier Kluris made it on macOS and Linux (LF)
//! and on Windows (`-crlf`: its text files with CRLF line endings):
//! - `skill-install`: the brains `acme` (git, a companion) and `beta` (no
//!   git), a Hermes profile, the skills of every agent the earlier Kluris
//!   wrote and its Windsurf workflows, then the user's own `kluris-notes`
//!   skills and workflow, for the skill install and `doctor`;
//! - `migration`: the brains `prod` (product) and `team` (blank) with
//!   neurons an agent wrote, dreamed and committed, one neuron written after
//!   the last commit, two Hermes profiles, a companion, and the user's own
//!   skill files, for the first commands after the upgrade.
//!
//! Every scenario runs on both forms of its world, on every OS: reading the
//! earlier files must not depend on the OS Kluris runs on. An output of the
//! CRLF world equals the LF world's unless the test names it, and a named
//! one has its own `_crlf` snapshot.

mod common;

use std::fs;
use std::path::Path;

use common::shared::{skill_reference, vendored_marker};
use common::upgrade::{Brain, World, link_folder};
use common::{
    PINNED_GIT_DATE, Run, Tree, assert_unchanged, changes_dump, command_line, git_dump, normalize,
    slashed,
};
use kluris_core::agents::install::{
    DestinationRole, InstallReport, InstallRequest, install_skills, read_brain_configs,
};
use kluris_core::agents::render::posix_path;
use kluris_core::config::read_global_config;
use serde_json::json;

/// A frozen world in its two forms: as the earlier Kluris made it on macOS
/// and Linux, then on Windows (each with its brains, whose last commit ids
/// differ with the line endings).
struct Forms {
    lf: (&'static str, &'static [Brain]),
    crlf: (&'static str, &'static [Brain]),
}

const SKILL_INSTALL: Forms = Forms {
    lf: (
        "skill-install",
        &[
            Brain {
                name: "acme",
                commits: &["brain: initialize acme"],
                head: Some("ad53edffd4247465f0a37f404c9e18bfebce23c9"),
            },
            Brain {
                name: "beta",
                commits: &[],
                head: None,
            },
        ],
    ),
    crlf: (
        "skill-install-crlf",
        &[
            Brain {
                name: "acme",
                commits: &["brain: initialize acme"],
                head: Some("e4383c823590a753f5bb7b4f92f145964c293cb0"),
            },
            Brain {
                name: "beta",
                commits: &[],
                head: None,
            },
        ],
    ),
};

const MIGRATION: Forms = Forms {
    lf: (
        "migration",
        &[
            Brain {
                name: "prod",
                commits: &["brain: initialize prod", "brain: add prod neurons"],
                head: Some("49ae2c9000967bbcc47d71aa3ca274ce6d25b620"),
            },
            Brain {
                name: "team",
                commits: &["brain: initialize team", "brain: add team neurons"],
                head: Some("f3fbfbcb41eda46fe00c7c2b5ee00f1979fe901d"),
            },
        ],
    ),
    crlf: (
        "migration-crlf",
        &[
            Brain {
                name: "prod",
                commits: &["brain: initialize prod", "brain: add prod neurons"],
                head: Some("bcb427dcb89f9689d6b953e6bc745b3ee94b0011"),
            },
            Brain {
                name: "team",
                commits: &["brain: initialize team", "brain: add team neurons"],
                head: Some("f3ecd7a44f5f272d24db454a197e83e82ce22efe"),
            },
        ],
    ),
};

/// A scenario's outputs, each with its snapshot name.
type Outputs = Vec<(String, String)>;

/// Run `scenario` on both forms of `forms`: snapshot each output of the LF
/// world; each output of the CRLF world named in `crlf_differs` differs
/// from the LF one and is snapshotted with `_crlf` appended, and every
/// other one equals the LF one.
fn on_both_forms(forms: &Forms, crlf_differs: &[&str], scenario: impl Fn(&World) -> Outputs) {
    let lf = scenario(&World::frozen(forms.lf.0, forms.lf.1));
    let crlf = scenario(&World::frozen(forms.crlf.0, forms.crlf.1));
    assert_eq!(
        lf.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        crlf.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    for name in crlf_differs {
        assert!(lf.iter().any(|(n, _)| n == name), "no output {name}");
    }
    let mut differ = Vec::new();
    for ((name, lf), (_, crlf)) in lf.iter().zip(&crlf) {
        insta::assert_snapshot!(name.as_str(), lf);
        if crlf_differs.contains(&name.as_str()) {
            assert_ne!(
                crlf, lf,
                "{name}: the CRLF world's output is the LF world's"
            );
            insta::assert_snapshot!(format!("{name}_crlf"), crlf);
        } else if crlf != lf {
            differ.push(name.as_str());
        }
    }
    assert!(
        differ.is_empty(),
        "the CRLF world's outputs differ: {differ:?}"
    );
}

/// What changed under `work/` since `before`, as `changes_dump` writes it,
/// with a runtime companion file and a generated brain SKILL.md shown by
/// reference (`common::shared`): a SKILL.md as its shared skill snapshot,
/// which checks its bytes.
fn changes(world: &World, before: &Tree) -> String {
    changes_dump(
        before,
        &world.files(),
        |text| normalize(text, &world.root),
        |path, entry| {
            vendored_marker(path, entry)
                .or_else(|| skill_reference(path, entry, &world.root, |_| String::new()))
        },
    )
}

fn output(name: impl Into<String>, text: String) -> (String, String) {
    (name.into(), text)
}

/// The skill folders (relative to HOME) the earlier Kluris wrote and
/// Kluris empties: the folders stay.
const EMPTIED: &[&str] = &[
    ".cursor/skills",
    ".codex/skills",
    ".copilot/skills",
    ".gemini/skills",
    ".kilo/skills",
    ".junie/skills",
    ".codeium/windsurf/skills",
    ".codeium/windsurf/global_workflows",
];

fn assert_emptied_folders_stay(world: &World) {
    for rel in EMPTIED {
        assert!(world.home.join(rel).is_dir(), "{rel} is gone");
    }
}

/// Write `files` (relative to HOME).
fn add_files(world: &World, files: &[(&str, &[u8])]) {
    for (rel, bytes) in files {
        let path = world.home.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, bytes).expect("write");
    }
}

// --- the skill install --------------------------------------------------------

/// The install's report, with paths relative to HOME.
fn report_text(report: &InstallReport, world: &World) -> String {
    let rel = |path: &Path| {
        path.strip_prefix(&world.home)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };
    let destinations: Vec<_> = report
        .destinations
        .iter()
        .map(|d| {
            let role = match &d.role {
                DestinationRole::Universal => "universal".to_string(),
                DestinationRole::Agent(a) => format!("agent:{a}"),
                DestinationRole::Deselected(a) => format!("deselected:{a}"),
                DestinationRole::Migration => "migration".to_string(),
            };
            json!({
                "path": rel(&d.path),
                "role": role,
                "alias_of": d.alias_of.as_deref().map(rel),
                "written": d.written,
                "removed": d.removed,
                "kept": d.kept,
                "failed": d.failed,
            })
        })
        .collect();
    let doc = json!({
        "ok": report.ok(),
        "skills": report.skills,
        "agents": report.agents,
        "destinations": destinations,
        "brain_errors": report.brain_errors,
    });
    let text = serde_json::to_string_pretty(&doc).expect("json") + "\n";
    normalize(&text, &world.root)
}

/// Install every registered brain's skill into the world's HOME, as
/// `create`, `register`, `remove`, `companion add|remove` and `doctor` do.
fn install(world: &World, copilot_home: Option<&Path>) -> InstallReport {
    let config = read_global_config(&world.config).expect("config");
    let brains = read_brain_configs(&config);
    install_skills(&InstallRequest {
        config: &config,
        brains: &brains,
        home: &world.home,
        copilot_home,
    })
}

fn skill_install(world: &World, copilot_home: bool, name: &str) -> Outputs {
    let before = world.files();
    let copilot = world.home.join("copilot-home");
    let report = install(world, copilot_home.then_some(copilot.as_path()));
    let outputs = vec![
        output(format!("{name}_report"), report_text(&report, world)),
        output(format!("{name}_changes"), changes(world, &before)),
    ];
    assert_emptied_folders_stay(world);
    outputs
}

#[test]
fn skill_install_moves_the_earlier_skills() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        skill_install(world, false, "skill_install")
    });
}

#[test]
fn skill_install_with_copilot_home_keeps_the_copilot_folder() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        skill_install(world, true, "skill_install_copilot_home")
    });
}

// --- doctor -------------------------------------------------------------------

/// `doctor <args>` and what it changed.
fn doctor(world: &World, args: &[&str], name: &str) -> Outputs {
    let before = world.files();
    let out = world.kluris(args, &[]);
    vec![
        output(name, world.render(&out)),
        output(format!("{name}_changes"), changes(world, &before)),
    ]
}

#[test]
fn doctor_moves_the_earlier_skills() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        let outputs = doctor(world, &["doctor", "--json"], "doctor_json");
        assert_emptied_folders_stay(world);
        outputs
    });
}

#[test]
fn doctor_text_output() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        let out = world.kluris(&["doctor"], &[]);
        vec![output("doctor_text", world.render(&out))]
    });
}

#[test]
fn doctor_no_refresh_writes_nothing() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        let before = world.files();
        let out = world.kluris(&["doctor", "--no-refresh", "--json"], &[]);
        assert_unchanged(&before, &world.files(), "doctor --no-refresh");
        vec![output("doctor_no_refresh", world.render(&out))]
    });
}

/// No git on PATH: the `git` check fails and doctor exits 1.
#[test]
fn doctor_a_failed_check_exits_1() {
    let world = World::empty();
    let empty = world.root.join("empty-path");
    fs::create_dir(&empty).expect("empty");
    let bin = common::upgrade::kluris_bin();
    let path = std::env::join_paths([bin.parent().expect("bin dir"), &empty]).expect("PATH");
    let out = world.kluris_on(path, &["doctor", "--no-refresh", "--json"], &[]);
    insta::assert_snapshot!("doctor_failed_check", world.render(&out));
}

/// A user's own skill whose description opens like Kluris's.
const PERSONAL_SKILL: &[u8] = b"---\nname: kluris-personal\ndescription: \"You are the team's subject matter expert on my personal notes\"\n---\n\n# Notes\n";

#[test]
fn doctor_keeps_entries_that_only_look_like_kluris() {
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        let earlier =
            fs::read(world.home.join(".claude/skills/kluris-acme/SKILL.md")).expect("skill");
        add_files(
            world,
            &[
                (".agents/skills/kluris-personal/SKILL.md", PERSONAL_SKILL),
                (".agents/skills/kluris-personal/private.md", b"my notes"),
                (".codex/skills/kluris-personal/SKILL.md", PERSONAL_SKILL),
                // The earlier Kluris's own staging names.
                (".agents/skills/.kluris-acme.tmp/SKILL.md", &earlier),
                (".agents/skills/.kluris-acme.old/SKILL.md", b"my draft"),
                (".agents/skills/.kluris-notes.old/SKILL.md", &earlier),
                // A skill folder with more than its SKILL.md.
                (".cursor/skills/kluris-acme/extra.txt", b"left by someone"),
            ],
        );
        doctor(world, &["doctor", "--json"], "doctor_look_alikes")
    });
}

/// `~/.codex/skills` is a link to `../.agents/skills` (a symlink, or on
/// Windows a junction where the machine makes no symlink): doctor writes
/// the universal folder once, reports the link as its alias and leaves the
/// link as it was. Skipped only on a machine that can make neither.
#[test]
fn doctor_a_skill_folder_linked_to_the_universal_one() {
    let target = Path::new("..").join(".agents").join("skills");
    let probe = World::empty();
    fs::create_dir_all(probe.home.join(".agents/skills")).expect("mkdir");
    fs::create_dir_all(probe.home.join(".codex")).expect("mkdir");
    let kind = match link_folder(&target, &probe.home.join(".codex/skills")) {
        Ok(kind) => kind,
        Err(why) => {
            eprintln!("skipped: this machine makes no folder link ({why})");
            return;
        }
    };
    eprintln!("~/.codex/skills is a {kind}");
    on_both_forms(&SKILL_INSTALL, &[], |world| {
        let codex = world.home.join(".codex/skills");
        fs::remove_dir_all(&codex).expect("rm codex");
        assert_eq!(link_folder(&target, &codex), Ok(kind));
        let outputs = doctor(world, &["doctor", "--json"], "doctor_linked_folder");
        assert!(
            fs::symlink_metadata(&codex).expect("codex").is_symlink(),
            "the link is gone"
        );
        for brain in ["acme", "beta"] {
            let skill = format!("kluris-{brain}/SKILL.md");
            assert_eq!(
                fs::read(codex.join(&skill)).expect("through the link"),
                fs::read(world.home.join(".agents/skills").join(&skill)).expect("universal"),
                "{skill} through the link"
            );
        }
        outputs
    });
}

// --- the first commands on a migrated machine ---------------------------------

const QUERIES: &[(&str, &[&str])] = &[
    ("prod", &["access token", "sessions"]),
    ("team", &["standup", "blockers"]),
];

/// A `search --json` run as the migration snapshots hold it: the shared
/// [`common::search::search_slice`], its brain and query those the run was
/// given.
fn search_slice(run: &Run, world: &World, brain: &str, query: &str) -> String {
    let (doc, slice) = common::search::search_slice(run);
    assert_eq!(doc["brain"], brain, "{}", world.render(run));
    assert_eq!(doc["query"], query, "{}", world.render(run));
    slice
}

/// The user's own files in the migration world (relative to HOME): no
/// command may change a byte of them.
const USER_FILES: &[&str] = &[
    ".agents/skills/README.txt",
    ".agents/skills/my-tool/SKILL.md",
    ".agents/skills/my-tool/run.sh",
    ".cursor/skills/kluris-notes/SKILL.md",
];

/// `tree`'s entries whose path starts with `prefix`.
fn under(tree: &Tree, prefix: &str) -> Tree {
    tree.iter()
        .filter(|(path, _)| path.starts_with(prefix))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect()
}

/// Fails the test, naming `what`, unless `now` holds the config and every
/// [`USER_FILES`] entry with the bytes the world was frozen with.
fn assert_user_files_kept(world: &World, frozen: &Tree, now: &Tree, what: &str) {
    let config = world
        .config
        .strip_prefix(&world.home)
        .expect("config under HOME");
    let config = format!("home/{}", slashed(config));
    let paths = std::iter::once(config).chain(USER_FILES.iter().map(|rel| format!("home/{rel}")));
    for path in paths {
        assert!(frozen.contains_key(&path), "the frozen world has no {path}");
        assert_eq!(now.get(&path), frozen.get(&path), "{what} changed {path}");
    }
}

/// `doctor`, then `wake-up` and `search` on each brain, `dream` on each,
/// and `doctor` again. The brain commands' snapshots are shared by both
/// runs: `COPILOT_HOME` changes only where the skills go.
///
/// One expected state is carried through the whole sequence: the world as
/// the first doctor leaves it (its brains, config and user files as
/// frozen), each dream's changes to its own brain added. A read changes
/// nothing in it, a dream changes nothing outside its brain, and after the
/// second doctor the world is that state, its config and user files as
/// frozen, and every brain's refs and history as frozen.
fn first_commands(world: &World, copilot_home: bool) -> Outputs {
    let copilot = world.home.join("copilot-home");
    let extra: Vec<(&str, &Path)> = if copilot_home {
        vec![("COPILOT_HOME", copilot.as_path())]
    } else {
        Vec::new()
    };
    let variant = if copilot_home { "_copilot_home" } else { "" };
    let mut outputs = Vec::new();

    let frozen = world.files();
    let histories = world.histories();
    let doctor = world.kluris(&["doctor", "--json"], &extra);
    outputs.push(output(
        format!("migration_doctor{variant}"),
        world.render(&doctor),
    ));
    outputs.push(output(
        format!("migration_doctor{variant}_changes"),
        changes(world, &frozen),
    ));
    let mut expected = world.files();
    assert_user_files_kept(world, &frozen, &expected, "the first doctor");
    assert_unchanged(
        &under(&frozen, "brains/"),
        &under(&expected, "brains/"),
        "the first doctor",
    );

    let read = |args: &[&str], expected: &Tree| {
        let out = world.kluris(args, &extra);
        let what = command_line(args);
        assert_unchanged(expected, &world.files(), &what);
        assert_eq!(
            world.histories(),
            histories,
            "{what} changed a brain's history"
        );
        out
    };
    for (brain, _) in QUERIES {
        let out = read(&["wake-up", "--brain", brain, "--json"], &expected);
        outputs.push(output(
            format!("migration_wake_up_{brain}"),
            world.render(&out),
        ));
    }
    for (brain, queries) in QUERIES {
        for query in *queries {
            let args = [
                "search", "--brain", brain, "--json", "--limit", "1000", "--", query,
            ];
            let out = read(&args, &expected);
            let name = format!("migration_search_{brain}_{}", query.replace(' ', "_"));
            outputs.push(output(name, search_slice(&out, world, brain, query)));
        }
    }

    let mut statuses = Vec::new();
    for (brain, _) in QUERIES {
        let prefix = format!("brains/{brain}/");
        let before = world.files();
        let what = format!("the commands before `dream --brain {brain}`");
        assert_unchanged(&expected, &before, &what);
        assert_unchanged(&under(&frozen, &prefix), &under(&before, &prefix), &what);
        let out = world.kluris(&["dream", "--brain", brain, "--json"], &extra);
        outputs.push(output(
            format!("migration_dream_{brain}"),
            world.render(&out),
        ));
        outputs.push(output(
            format!("migration_dream_{brain}_changes"),
            changes(world, &expected),
        ));
        let after = world.files();
        expected.retain(|path, _| !path.starts_with(&prefix));
        expected.extend(under(&after, &prefix));
        assert_unchanged(&expected, &after, &format!("dream --brain {brain}"));
        let repo = world.brain(brain);
        let status = world.git(
            &repo,
            &["-c", "core.quotePath=false", "status", "--porcelain"],
            PINNED_GIT_DATE,
        );
        outputs.push(output(
            format!("migration_dream_{brain}_git"),
            format!("== git status --porcelain\n{status}{}", git_dump(&repo)),
        ));
        statuses.push((repo, status));
    }

    let doctor = world.kluris(&["doctor", "--json"], &extra);
    outputs.push(output(
        format!("migration_second_doctor{variant}"),
        world.render(&doctor),
    ));
    let last = world.files();
    assert_unchanged(&expected, &last, "the commands after the upgrade");
    assert_user_files_kept(world, &frozen, &last, "the commands after the upgrade");
    assert_eq!(
        world.histories(),
        histories,
        "the commands after the upgrade changed a brain's history"
    );
    for (repo, status) in statuses {
        let now = world.git(
            &repo,
            &["-c", "core.quotePath=false", "status", "--porcelain"],
            PINNED_GIT_DATE,
        );
        assert_eq!(now, status, "{}: the work tree's state", repo.display());
    }
    assert_emptied_folders_stay(world);
    outputs
}

/// The outputs of [`first_commands`] that differ on the CRLF world: dream
/// keeps a CRLF file CRLF when it rewrites it, and the brains' commits hold
/// the CRLF bytes, so their ids differ.
const FIRST_COMMANDS_CRLF: &[&str] = &[
    "migration_dream_prod_changes",
    "migration_dream_prod_git",
    "migration_dream_team_git",
];

#[test]
fn first_commands_after_the_upgrade() {
    on_both_forms(&MIGRATION, FIRST_COMMANDS_CRLF, |world| {
        first_commands(world, false)
    });
}

#[test]
fn first_commands_after_the_upgrade_with_copilot_home() {
    on_both_forms(&MIGRATION, FIRST_COMMANDS_CRLF, |world| {
        first_commands(world, true)
    });
}

// --- the POSIX path rule ------------------------------------------------------

/// `[path, answer, the answer's anchor]`: the earlier Kluris's POSIX form of
/// each path (every piece of the path alphabet below, every ordered pair of
/// pieces, odd singles, and a thousand seeded random joins of 3 to 10
/// pieces), and the drive and root it parsed in it, POSIX-formed. The
/// alphabet: `/`, `\`, `//`, `\\`, `:`, `C:`, `c:`, `1:`, `?`, `\\?\`,
/// `UNC`, `\\?\UNC\`, `.`, `..`, `a`, `brain`, `Users`, a space, `é`,
/// `日本`, `-`, `~`, `server`, `share`.
fn posix_rows() -> Vec<(String, String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/posix-path-rows.jsonl");
    let text = fs::read_to_string(&path).expect("rows");
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}")))
        .collect()
}

#[test]
fn posix_path_gives_the_frozen_answers() {
    let wrong: Vec<String> = posix_rows()
        .iter()
        .filter(|(path, answer, _)| posix_path(path) != *answer)
        .map(|(path, answer, _)| format!("{path:?}: {:?}, expected {answer:?}", posix_path(path)))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Whether the rule reads `path` as a Windows path (a backslash, or a drive
/// letter's colon second).
fn read_as_windows(path: &str) -> bool {
    path.contains('\\') || path.chars().nth(1) == Some(':')
}

/// `text` read as a POSIX path: two leading slashes kept, every other empty
/// or `.` part dropped.
fn as_posix(text: &str) -> String {
    let rest = text.trim_start_matches('/');
    let root = match text.len() - rest.len() {
        0 => "",
        2 => "//",
        _ => "/",
    };
    let parts: Vec<&str> = rest
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    format!("{root}{}", parts.join("/"))
}

/// Every answer is `.` or its anchor and then parts that are neither `.`
/// nor empty. Every answer is its own answer, except one whose anchor is a
/// UNC or `\\?\` drive (a Windows path that opens with two separators):
/// it holds no backslash or drive colon, so it reads back as a POSIX path,
/// which drops its trailing and doubled `/`.
#[test]
fn posix_path_has_no_dot_or_empty_parts_and_is_idempotent() {
    let mut unc = 0;
    for (path, _, anchor) in posix_rows() {
        let answer = posix_path(&path);
        if read_as_windows(&path) && anchor.starts_with("//") {
            assert_eq!(posix_path(&answer), as_posix(&answer), "{path:?}");
            unc += usize::from(posix_path(&answer) != answer);
        } else {
            assert_eq!(posix_path(&answer), answer, "{path:?}: not idempotent");
        }
        if answer == "." {
            assert_eq!(anchor, "", "{path:?}");
            continue;
        }
        let rest = answer
            .strip_prefix(anchor.as_str())
            .unwrap_or_else(|| panic!("{path:?}: {answer:?} does not start with {anchor:?}"));
        if !rest.is_empty() {
            assert!(
                rest.split('/').all(|part| !part.is_empty() && part != "."),
                "{path:?}: {answer:?} has a `.` or empty part after {anchor:?}"
            );
        }
    }
    assert_eq!(
        unc, 36,
        "the UNC and `\\\\?\\` answers that read back differently"
    );
}
