//! `--help` (and `-h`) must never print an implementation note. clap renders
//! a field's `///` doc comment as the argument's help (its later paragraphs
//! as the long help), so a note meant for readers of the source leaks into
//! the binary's output when it is written as a doc comment.

use std::path::Path;

use assert_cmd::Command;
use assert_fs::TempDir;

fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    cmd
}

/// Words the source may say but a user's help output must not.
const INTERNAL_WORDS: &[&str] = &[
    "Kept as given",
    "[`",
    "v2",
    "PAR-",
    "Click",
    "clap",
    "parity",
];

/// The top-level commands, in the order the root's `--help` lists them.
const TOP_LEVEL: &[&str] = &[
    "create",
    "register",
    "list",
    "wake-up",
    "search",
    "status",
    "companion",
    "dream",
    "mri",
    "remove",
    "doctor",
    "commit",
    "sync",
    "resolve",
    "self-update",
    "self-uninstall",
    "help",
];

/// Every command path, found by walking each `--help` text's `Commands:`
/// table (the root first, then each group's table in order), so a new
/// command or subcommand joins the sweep without editing this list.
fn command_paths(home: &Path) -> Vec<Vec<String>> {
    let mut paths = vec![Vec::new()];
    let mut at = 0;
    while at < paths.len() {
        let out = kluris(home)
            .args(&paths[at])
            .arg("--help")
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        for name in commands_section(&stdout) {
            let mut sub = paths[at].clone();
            sub.push(name.to_string());
            paths.push(sub);
        }
        at += 1;
    }
    paths
}

/// The command names in a help text's `Commands:` table
/// (`  list            List registered brains.`).
fn commands_section(help: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut section = false;
    for line in help.lines() {
        if section {
            if line.trim().is_empty() {
                break;
            }
            if let Some(name) = line.split_whitespace().next() {
                names.push(name);
            }
        }
        section = section || line == "Commands:";
    }
    names
}

/// The commands that run git hooks name each one in their `--help`, and
/// which of them `--no-verify` skips.
#[test]
fn help_names_the_hooks_a_command_runs() {
    let home = TempDir::new().unwrap();
    // Whitespace goes first: a wrapped help line must not hide a hook name.
    let flat = |args: &[&str]| {
        let out = kluris(home.path()).args(args).output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    for command in ["sync", "resolve"] {
        let help = flat(&[command, "--help"]);
        for text in [
            "pre-merge-commit",
            "prepare-commit-msg",
            "post-merge",
            "pre-push",
            "reference-transaction",
            "post-index-change",
            "Skip the pre-merge-commit and commit-msg hooks",
            "--no-verify skips pre-merge-commit and commit-msg",
        ] {
            assert!(
                help.contains(text),
                "kluris {command} --help lacks {text:?}:\n{help}"
            );
        }
    }
    let help = flat(&["commit", "--help"]);
    for text in [
        "pre-commit",
        "prepare-commit-msg",
        "post-commit",
        "reference-transaction",
        "post-index-change",
        "--no-verify skips pre-commit and commit-msg",
    ] {
        assert!(
            help.contains(text),
            "kluris commit --help lacks {text:?}:\n{help}"
        );
    }
}

/// `doctor --help` and `sync --help` say what the commands do with the
/// remote: doctor's checks read every brain through its sync, asking its
/// remote each time, so `--no-refresh` only skips the refresh, and a
/// conflict `sync` leaves is settled either way there is.
#[test]
fn help_says_doctor_syncs_and_both_ways_to_settle_a_conflict() {
    let home = TempDir::new().unwrap();
    let flat = |args: &[&str]| {
        let out = kluris(home.path()).args(args).output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let doctor = flat(&["doctor", "--help"]);
    assert!(!doctor.contains("read-only"), "{doctor}");
    assert!(
        doctor.contains("Doctor asks every brain's remote each time"),
        "{doctor}"
    );
    assert!(
        doctor.contains("--no-refresh skips refreshing the companions and agent skills"),
        "{doctor}"
    );
    let sync = flat(&["sync", "--help"]);
    for text in ["`kluris commit --merge <id>`", "`kluris resolve`"] {
        assert!(
            sync.contains(text),
            "kluris sync --help lacks {text:?}:\n{sync}"
        );
    }
}

#[test]
fn help_prints_no_implementation_notes() {
    let home = TempDir::new().unwrap();
    let paths = command_paths(home.path());
    let top: Vec<&str> = paths
        .iter()
        .filter(|p| p.len() == 1)
        .map(|p| p[0].as_str())
        .collect();
    assert_eq!(top, TOP_LEVEL, "the sweep must cover every command");
    for path in &paths {
        for flag in ["--help", "-h"] {
            let out = kluris(home.path()).args(path).arg(flag).output().unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            for word in INTERNAL_WORDS {
                assert!(
                    !text.contains(word),
                    "kluris {} {flag} printed {word:?}:\n{text}",
                    path.join(" ")
                );
            }
        }
    }
}
