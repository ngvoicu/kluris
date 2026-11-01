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

/// Every `--flag` written after `kluris <command>` on a line of `text` that
/// the command's `--help` (a `companion` subcommand's own help) lacks.
fn unknown_flags(text: &str, home: &Path) -> Vec<String> {
    let names = commands(home);
    let mut helps: BTreeMap<String, String> = BTreeMap::new();
    let mut unknown = Vec::new();
    for line in text.lines() {
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
            .or_insert_with(|| stdout(kluris(home).args(&command).arg("--help")));
        for flag in words[at..].iter().filter(|w| w.starts_with("--")) {
            let flag = flag.split('=').next().unwrap_or(flag);
            let flag = flag.trim_end_matches(|c: char| !c.is_alphanumeric());
            if !help.contains(flag) {
                unknown.push(format!("kluris {key} {flag}"));
            }
        }
    }
    unknown
}

#[test]
fn readme_flags_exist() {
    let home = TempDir::new().unwrap();
    let unknown = unknown_flags(&readme(), home.path());
    assert!(
        unknown.is_empty(),
        "README uses flags the commands lack: {unknown:?}"
    );
}

/// The quick start runs as written in a fresh HOME: the folder it passes to
/// `--path` is made first.
#[test]
fn quick_start_makes_the_folder_it_creates_the_brain_in() {
    let readme = readme();
    let create = readme
        .find("kluris create acme --description \"Acme platform knowledge\" --path ~/brains")
        .expect("the non-interactive create example");
    let block = &readme[readme[..create].rfind("```").unwrap()..create];
    assert!(
        block.contains("mkdir -p ~/brains\n"),
        "no `mkdir -p ~/brains` before the create example: {block:?}"
    );
}

/// The fenced block that opens right after `marker`.
fn block_after<'a>(text: &'a str, marker: &str) -> &'a str {
    let at = text.find(marker).unwrap_or_else(|| panic!("no {marker:?}"));
    let open = at + text[at..].find("```").unwrap();
    let body = open + text[open..].find('\n').unwrap() + 1;
    &text[body..body + text[body..].find("```").unwrap()]
}

/// The `--json` search result the README shows is what `kluris search`
/// returns for the README's own example neuron (all but the corpus-dependent
/// score).
#[test]
fn search_json_example_matches_the_example_neuron() {
    let readme = readme();
    let neuron = block_after(&readme, "are required; `related` holds the synapses");
    let shown: serde_json::Value =
        serde_json::from_str(block_after(&readme, "date, and `deprecated` (with")).unwrap();

    let home = TempDir::new().unwrap();
    let brains = home.path().join("brains");
    std::fs::create_dir(&brains).unwrap();
    kluris(home.path())
        .args(["create", "acme", "--no-git", "--json", "--path"])
        .arg(&brains)
        .assert()
        .success();
    let file = brains.join("acme/projects/billing/auth-flow.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, neuron).unwrap();
    // The decision its `related` names, as in the README's brain.
    std::fs::write(
        brains.join("acme/knowledge/use-clerk.md"),
        "---\nparent: ./map.md\nrelated:\n  - ../projects/billing/auth-flow.md\n\
         created: 2026-09-01\nupdated: 2026-09-01\n---\n\n# Use Clerk for sign-in\n",
    )
    .unwrap();
    let out: serde_json::Value = serde_json::from_str(&stdout(kluris(home.path()).args([
        "search",
        "refreshing tokens",
        "--brain",
        "acme",
        "--json",
    ])))
    .unwrap();
    let mut got = out["results"][0].clone();
    let mut want = shown.clone();
    got.as_object_mut().unwrap().remove("score");
    want.as_object_mut().unwrap().remove("score");
    assert_eq!(got, want);
}

/// The README `kluris create` writes into every new brain.
fn brain_readme(home: &Path) -> String {
    let brains = home.join("brains");
    std::fs::create_dir(&brains).unwrap();
    kluris(home)
        .args(["create", "acme", "--no-git", "--json", "--path"])
        .arg(&brains)
        .assert()
        .success();
    std::fs::read_to_string(brains.join("acme/README.md")).unwrap()
}

#[test]
fn brain_readme_installs_todays_kluris() {
    let home = TempDir::new().unwrap();
    let readme = brain_readme(home.path());
    for line in [
        "curl -fsSL https://kluris.ngvoicu.dev/install.sh | sh",
        "irm https://kluris.ngvoicu.dev/install.ps1 | iex",
        "git clone <this-repo-url> ~/path/to/brain",
        "kluris register ~/path/to/brain",
    ] {
        assert!(readme.contains(line), "brain README has no {line:?}");
    }
    let banned = [
        "pipx", "PyPI", "Python", "Windsurf", "Kluris 2", "v2", "pack",
    ];
    let found: Vec<_> = banned
        .iter()
        .filter(|term| whole_word_hits(&readme, term) > 0)
        .collect();
    assert!(found.is_empty(), "brain README mentions {found:?}");
    assert!(
        !readme.contains("installed Kluris package"),
        "brain README says companions come from a package"
    );
}

#[test]
fn brain_readme_names_todays_agents() {
    let home = TempDir::new().unwrap();
    let readme = brain_readme(home.path());
    for agent in [
        "Claude Code",
        "Codex",
        "Cursor",
        "Devin",
        "Antigravity CLI",
        "Hermes Agent",
        "Kilo Code",
        "Junie",
        "OpenCode",
    ] {
        assert!(readme.contains(agent), "brain README does not name {agent}");
    }
}

#[test]
fn brain_readme_flags_exist() {
    let home = TempDir::new().unwrap();
    let readme = brain_readme(home.path());
    let unknown = unknown_flags(&readme, home.path());
    assert!(
        unknown.is_empty(),
        "brain README uses flags the commands lack: {unknown:?}"
    );
}

#[test]
fn readmes_name_antigravity_cli_and_never_gemini_cli() {
    let home = TempDir::new().unwrap();
    for (which, text) in [
        ("README", readme()),
        ("brain README", brain_readme(home.path())),
    ] {
        assert!(!text.contains("Gemini CLI"), "{which} names Gemini CLI");
        assert!(
            !text.contains("Gemini/"),
            "{which} names Gemini in the skill line"
        );
    }
    let readme = readme();
    assert!(
        readme.contains("| `~/.gemini/config/skills/` | Antigravity CLI (`agy`) |"),
        "README's Agents table has no Antigravity CLI row"
    );
    assert!(readme.contains("  Claude/Codex/Antigravity/etc: /kluris-acme\n"));
    // The kluris.yml example carries today's default list.
    assert!(
        !readme.contains("  - gemini\n"),
        "README lists gemini as an agent"
    );
}

/// `text` with every run of whitespace one space, so a phrase reads the
/// same wherever the text wraps its lines.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The README's sync section, from its heading to the next `## ` heading.
fn sync_section(readme: &str) -> &str {
    let start = readme
        .find("\n### Keep it in sync with your team\n")
        .expect("a sync section");
    let rest = &readme[start + 1..];
    &rest[..rest.find("\n## ").map_or(rest.len(), |at| at + 1)]
}

#[test]
fn readme_says_reads_pull_commit_pushes_and_a_background_push_shares_the_rest() {
    let readme = readme();
    let sync = flat(sync_section(&readme));
    for claim in [
        // What each command does with the remote.
        "`kluris wake-up`, `kluris search`, `kluris status`, `kluris register` and `kluris doctor` bring your teammates' commits in before they read",
        "at most once a minute, within 3 seconds",
        "within 10 seconds in all",
        "A read's exit code is never its sync's",
        "`kluris dream` and `kluris mri` pull, then commit the brain's own housekeeping",
        "`kluris commit` commits the files you name",
        "then pushes",
        // The background push and its record.
        "one background push",
        "`last_push`",
        "`needs_pull`",
        // Busy.
        "`busy`",
        // Every command syncs; nothing else does.
        "There is no switch that turns syncing off",
        "no watch mode and no service",
        // By hand.
        "`kluris sync`",
        "`kluris resolve`",
        "`kluris create --remote",
        "`kluris register`",
    ] {
        assert!(sync.contains(claim), "the sync section lacks {claim:?}");
    }
}

#[test]
fn readme_never_has_the_agent_sync_before_it_writes() {
    let readme = flat(&readme());
    for stale in [
        "KLURIS_NO_AUTO_SYNC",
        "auto_fetch",
        "kluris watch",
        "the agent runs it before every write",
        "Before it writes, the agent syncs",
        "commit, push and pull it with `git` like any",
    ] {
        assert!(!readme.contains(stale), "README says {stale:?}");
    }
}

#[test]
fn brain_readme_says_kluris_keeps_the_brain_in_step() {
    let home = TempDir::new().unwrap();
    let readme = flat(&brain_readme(home.path()));
    for claim in [
        "## Working with your team",
        "keeps it in step",
        "kluris commit --brain acme --paths",
        "kluris commit --brain acme --merge",
        "kluris resolve --brain acme",
    ] {
        assert!(readme.contains(claim), "brain README lacks {claim:?}");
    }
    for stale in [
        "Sync and branch with git directly",
        "Use git directly to commit and sync",
    ] {
        assert!(!readme.contains(stale), "brain README says {stale:?}");
    }
}
