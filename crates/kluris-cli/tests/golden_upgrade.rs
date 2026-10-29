//! Golden tests of Kluris on a home an earlier Kluris set up (the frozen
//! worlds of `common::upgrade`): the skill install, `doctor`, and the first
//! commands a user runs, with what each prints and every file it changes;
//! and the POSIX path rule the skills write paths with, against the frozen
//! rows of the earlier Kluris's answers.
//!
//! The worlds:
//! - `skill-install`: the brains `acme` (git, a companion) and `beta` (no
//!   git), a Hermes profile, the skills of every agent the earlier Kluris
//!   wrote and its Windsurf workflows, then the user's own `kluris-notes`
//!   skills and workflow (the parity scenarios of `test_skill_install.py`
//!   and `test_doctor.py`);
//! - `migration`: the brains `prod` (product) and `team` (blank) with
//!   neurons an agent wrote, dreamed and committed, one neuron written after
//!   the last commit, two Hermes profiles, a companion, and the user's own
//!   skill files (the parity scenario of `test_migration.py`).

mod common;

use std::fs;
use std::path::Path;

use common::upgrade::{Brain, World};
use common::{PINNED_GIT_DATE, git_dump, normalize, printed_paths};
use kluris_core::agents::install::{
    DestinationRole, InstallReport, InstallRequest, install_skills, read_brain_configs,
};
use kluris_core::agents::render::posix_path;
use kluris_core::config::read_global_config;
use serde_json::json;

const SKILL_INSTALL: &[Brain] = &[
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
];

const MIGRATION: &[Brain] = &[
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
];

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

#[test]
fn skill_install_moves_the_earlier_skills() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let before = world.files();
    let report = install(&world, None);
    insta::assert_snapshot!("skill_install_report", report_text(&report, &world));
    insta::assert_snapshot!("skill_install_changes", world.changes_since(&before));
    assert_emptied_folders_stay(&world);
}

#[test]
fn skill_install_with_copilot_home_keeps_the_copilot_folder() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let before = world.files();
    let report = install(&world, Some(&world.home.join("copilot-home")));
    insta::assert_snapshot!(
        "skill_install_copilot_home_report",
        report_text(&report, &world)
    );
    insta::assert_snapshot!(
        "skill_install_copilot_home_changes",
        world.changes_since(&before)
    );
}

// --- doctor -------------------------------------------------------------------

#[test]
fn doctor_moves_the_earlier_skills() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let before = world.files();
    let out = world.kluris(&["doctor", "--json"], &[]);
    printed_paths(|| insta::assert_snapshot!("doctor_json", world.render(&out)));
    insta::assert_snapshot!("doctor_json_changes", world.changes_since(&before));
    assert_emptied_folders_stay(&world);
}

#[test]
fn doctor_text_output() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let out = world.kluris(&["doctor"], &[]);
    printed_paths(|| insta::assert_snapshot!("doctor_text", world.render(&out)));
}

#[test]
fn doctor_no_refresh_writes_nothing() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let before = world.files();
    let out = world.kluris(&["doctor", "--no-refresh", "--json"], &[]);
    printed_paths(|| insta::assert_snapshot!("doctor_no_refresh", world.render(&out)));
    assert_eq!(world.changes_since(&before), "<nothing changed>\n");
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
    printed_paths(|| insta::assert_snapshot!("doctor_failed_check", world.render(&out)));
}

/// A user's own skill whose description opens like Kluris's.
const PERSONAL_SKILL: &[u8] = b"---\nname: kluris-personal\ndescription: \"You are the team's subject matter expert on my personal notes\"\n---\n\n# Notes\n";

#[test]
fn doctor_keeps_entries_that_only_look_like_kluris() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let earlier = fs::read(world.home.join(".claude/skills/kluris-acme/SKILL.md")).expect("skill");
    add_files(
        &world,
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
    let before = world.files();
    let out = world.kluris(&["doctor", "--json"], &[]);
    printed_paths(|| insta::assert_snapshot!("doctor_look_alikes", world.render(&out)));
    insta::assert_snapshot!("doctor_look_alikes_changes", world.changes_since(&before));
}

/// `~/.codex/skills` is a link to `../.agents/skills`: doctor writes the
/// universal folder once and reports the link as its alias.
#[cfg(unix)]
#[test]
fn doctor_a_skill_folder_linked_to_the_universal_one() {
    let world = World::frozen("skill-install", SKILL_INSTALL);
    let codex = world.home.join(".codex/skills");
    fs::remove_dir_all(&codex).expect("rm codex");
    std::os::unix::fs::symlink("../.agents/skills", &codex).expect("symlink");
    let before = world.files();
    let out = world.kluris(&["doctor", "--json"], &[]);
    insta::assert_snapshot!("doctor_linked_folder", world.render(&out));
    insta::assert_snapshot!("doctor_linked_folder_changes", world.changes_since(&before));
}

// --- the first commands on a migrated machine ---------------------------------

const QUERIES: &[(&str, &[&str])] = &[
    ("prod", &["access token", "sessions"]),
    ("team", &["standup", "blockers"]),
];

/// `doctor`, then `wake-up` and `search` on each brain, `dream` on each,
/// and `doctor` again. The brain commands' snapshots are shared by both
/// runs: `COPILOT_HOME` changes only where the skills go.
fn first_commands(copilot_home: bool) {
    let world = World::frozen("migration", MIGRATION);
    let copilot = world.home.join("copilot-home");
    let extra: Vec<(&str, &Path)> = if copilot_home {
        vec![("COPILOT_HOME", copilot.as_path())]
    } else {
        Vec::new()
    };
    let variant = if copilot_home { "_copilot_home" } else { "" };

    let frozen = world.files();
    let doctor = world.kluris(&["doctor", "--json"], &extra);
    printed_paths(|| {
        insta::assert_snapshot!(format!("migration_doctor{variant}"), world.render(&doctor))
    });
    insta::assert_snapshot!(
        format!("migration_doctor{variant}_changes"),
        world.changes_since(&frozen)
    );

    for (brain, _) in QUERIES {
        let out = world.kluris(&["wake-up", "--brain", brain, "--json"], &extra);
        printed_paths(|| {
            insta::assert_snapshot!(format!("migration_wake_up_{brain}"), world.render(&out))
        });
    }
    for (brain, queries) in QUERIES {
        for query in *queries {
            let args = [
                "search", "--brain", brain, "--json", "--limit", "1000", "--", query,
            ];
            let out = world.kluris(&args, &extra);
            let name = format!("migration_search_{brain}_{}", query.replace(' ', "_"));
            insta::assert_snapshot!(name, world.render(&out));
        }
    }
    for (brain, _) in QUERIES {
        let before = world.files();
        let out = world.kluris(&["dream", "--brain", brain, "--json"], &extra);
        insta::assert_snapshot!(format!("migration_dream_{brain}"), world.render(&out));
        insta::assert_snapshot!(
            format!("migration_dream_{brain}_changes"),
            world.changes_since(&before)
        );
        let repo = world.brain(brain);
        let status = world.git(
            &repo,
            &["-c", "core.quotePath=false", "status", "--porcelain"],
            PINNED_GIT_DATE,
        );
        insta::assert_snapshot!(
            format!("migration_dream_{brain}_git"),
            format!("== git status --porcelain\n{status}{}", git_dump(&repo))
        );
    }

    let before = world.files();
    let doctor = world.kluris(&["doctor", "--json"], &extra);
    printed_paths(|| {
        insta::assert_snapshot!(
            format!("migration_second_doctor{variant}"),
            world.render(&doctor)
        )
    });
    assert_eq!(world.changes_since(&before), "<nothing changed>\n");
    assert_emptied_folders_stay(&world);
}

#[test]
fn first_commands_after_the_upgrade() {
    first_commands(false);
}

#[test]
fn first_commands_after_the_upgrade_with_copilot_home() {
    first_commands(true);
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
