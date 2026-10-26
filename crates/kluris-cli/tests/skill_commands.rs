//! Every `kluris ...` command the agent skill tells an agent to run exists,
//! with each `--flag` it names (SPEC items 49/50: the skill's write flow
//! calls `sync`, `commit` and `resolve`).

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

#[test]
fn every_command_the_skill_names_exists_with_its_flags() {
    let home = TempDir::new().unwrap();
    let skill = render_skill(&SkillInput {
        brain_name: "acme",
        brain_path: "/brains/acme",
        has_git: true,
        description: "",
        companions: &[],
        companion_home: "/home/u/.kluris/companions",
    });
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
    for command in ["sync", "commit", "resolve", "wake-up", "search", "dream"] {
        assert!(
            seen.iter().any(|c| c == command),
            "the skill never names {command}"
        );
    }
}
