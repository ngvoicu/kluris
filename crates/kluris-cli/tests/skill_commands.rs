//! Every `kluris ...` command the agent skill tells an agent to run exists,
//! with each `--flag` it names, and the skill's CLI list is the agent's:
//! `commit` and `resolve` settle and share its writes, while `sync` is left
//! to people, since every command keeps the brain in step.

use std::path::Path;

use assert_cmd::Command;
use assert_fs::TempDir;
use kluris_core::agents::render::{SkillInput, render_skill};

fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    cmd
}

/// The inline code spans of `text` that start with `kluris `.
fn kluris_spans(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("kluris "))
        .map(str::to_string)
        .collect()
}

fn skill() -> String {
    render_skill(&SkillInput {
        brain_name: "acme",
        brain_path: "/brains/acme",
        has_git: true,
        description: "",
        companions: &[],
        companion_home: "/home/u/.kluris/companions",
    })
}

/// The `kluris` command each bullet of the skill's `## CLI commands` list
/// names, with the bullet's first code span.
fn cli_list(skill: &str) -> Vec<(String, String)> {
    let start = skill
        .find("\n## CLI commands\n")
        .expect("a CLI commands section");
    let list = &skill[start + 1..];
    let list = &list[..list[3..].find("\n## ").map_or(list.len(), |at| at + 3)];
    list.lines()
        .filter(|line| line.starts_with("- `kluris "))
        .map(|line| {
            let span = kluris_spans(line).remove(0);
            let command = span.split_whitespace().nth(1).unwrap().to_string();
            (command, span)
        })
        .collect()
}

#[test]
fn every_command_the_skill_names_exists_with_its_flags() {
    let home = TempDir::new().unwrap();
    let skill = skill();
    let spans = kluris_spans(&skill);
    let mut seen = Vec::new();
    for span in &spans {
        let words: Vec<&str> = span
            .split(|c: char| c.is_whitespace() || c == '|')
            .filter(|w| !w.is_empty())
            .collect();
        let command = words[1];
        let out = kluris(home.path())
            .args([command, "--help"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{span}: `kluris {command}` is not a command"
        );
        let help = String::from_utf8(out.stdout).unwrap();
        for flag in words.iter().filter(|w| w.starts_with("--")) {
            assert!(
                help.contains(flag),
                "{span}: `kluris {command}` has no {flag}"
            );
        }
        seen.push(command.to_string());
    }
    for command in ["commit", "resolve", "wake-up", "search", "dream"] {
        assert!(
            seen.iter().any(|c| c == command),
            "the skill never names {command}"
        );
    }
}

#[test]
fn the_cli_list_has_commit_and_resolve_with_merge_and_no_sync() {
    let list = cli_list(&skill());
    let commands: Vec<&str> = list.iter().map(|(c, _)| c.as_str()).collect();
    assert!(
        !commands.contains(&"sync"),
        "the agent's CLI list names sync: {commands:?}"
    );
    for command in [
        "commit", "resolve", "wake-up", "search", "dream", "status", "mri",
    ] {
        assert!(
            commands.contains(&command),
            "the agent's CLI list lacks {command}: {commands:?}"
        );
    }
    let resolve = &list.iter().find(|(c, _)| c == "resolve").unwrap().1;
    assert!(resolve.contains("--merge"), "{resolve}");
    let commit = &list.iter().find(|(c, _)| c == "commit").unwrap().1;
    assert!(commit.contains("--paths"), "{commit}");
}
