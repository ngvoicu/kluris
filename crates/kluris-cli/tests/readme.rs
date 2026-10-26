//! The README describes the product as it is: every command `kluris help`
//! lists, the site's install commands, flags each command really takes, and
//! no history (earlier versions, dropped features, internal jargon).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_fs::TempDir;

fn readme() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    std::fs::read_to_string(&path).unwrap()
}

/// `kluris` with HOME and KLURIS_CONFIG inside `home`.
fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn stdout(cmd: &mut Command) -> String {
    String::from_utf8(cmd.output().unwrap().stdout).unwrap()
}

fn commands(home: &Path) -> Vec<String> {
    let help: serde_json::Value =
        serde_json::from_str(&stdout(kluris(home).args(["help", "--json"]))).unwrap();
    help["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

/// Case-insensitive occurrences of `term` in `text` that are whole words.
fn whole_word_hits(text: &str, term: &str) -> usize {
    let text = text.to_lowercase();
    let term = term.to_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(&term)
        .filter(|(at, _)| {
            !word(text[..*at].chars().next_back()) && !word(text[at + term.len()..].chars().next())
        })
        .count()
}

#[test]
fn readme_carries_no_history_or_internal_jargon() {
    let readme = readme();
    let banned = [
        "Kluris 2",
        "v2",
        "Python",
        "pipx",
        "PyPI",
        "migration",
        "migrate",
        "no longer",
        "removed",
        "used to",
        "pack",
        "Windsurf",
        "allowlist",
        "parity",
        "BrainStore",
        "change set",
    ];
    let found: Vec<_> = banned
        .iter()
        .filter(|term| whole_word_hits(&readme, term) > 0)
        .collect();
    assert!(found.is_empty(), "README mentions {found:?}");
    assert!(!readme.contains("PAR-"), "README mentions a PAR number");
}

#[test]
fn readme_names_every_command() {
    let home = TempDir::new().unwrap();
    let readme = readme();
    let missing: Vec<_> = commands(home.path())
        .into_iter()
        .filter(|c| !readme.contains(&format!("`kluris {c}")))
        .collect();
    assert!(
        missing.is_empty(),
        "README has no `kluris <command>` for {missing:?}"
    );
}

#[test]
fn readme_leads_with_the_site_installers() {
    let readme = readme();
    for line in [
        "curl -fsSL https://kluris.ngvoicu.dev/install.sh | sh",
        "irm https://kluris.ngvoicu.dev/install.ps1 | iex",
        "kluris self-update",
        "kluris self-uninstall",
    ] {
        assert!(readme.contains(line), "README has no {line:?}");
    }
}

/// Every `--flag` written after `kluris <command>` on a README line is in
/// that command's `--help` (a `companion` subcommand's own help).
#[test]
fn readme_flags_exist() {
    let home = TempDir::new().unwrap();
    let names = commands(home.path());
    let mut helps: BTreeMap<String, String> = BTreeMap::new();
    let mut unknown = Vec::new();
    for line in readme().lines() {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || "`|(),".contains(c))
            .filter(|w| !w.is_empty())
            .collect();
        let Some(at) = words
            .windows(2)
            .position(|w| w[0] == "kluris" && names.iter().any(|n| n == w[1]))
        else {
            continue;
        };
        let mut command = vec![words[at + 1]];
        if command[0] == "companion"
            && let Some(sub) = words
                .get(at + 2)
                .filter(|s| ["list", "add", "remove"].contains(s))
        {
            command.push(sub);
        }
        let key = command.join(" ");
        let help = helps
            .entry(key.clone())
            .or_insert_with(|| stdout(kluris(home.path()).args(&command).arg("--help")));
        for flag in words[at..].iter().filter(|w| w.starts_with("--")) {
            let flag = flag.split('=').next().unwrap_or(flag);
            let flag = flag.trim_end_matches(|c: char| !c.is_alphanumeric());
            if !help.contains(flag) {
                unknown.push(format!("kluris {key} {flag}"));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "README uses flags the commands lack: {unknown:?}"
    );
}
