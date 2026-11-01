//! The agent skill and the brain README tell agents and people how syncing
//! works: every command that opens a brain keeps it in step with the team,
//! `kluris commit` commits and pushes, and the `sync` key says what
//! happened. Each test reads one part of a template as it is rendered.

use kluris_core::agents::render::{SkillInput, render_skill};

const BRAIN_README: &str = include_str!("../templates/brain_readme.md");

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

/// The part of `text` from the heading line `heading` to the next heading
/// of the same level or above.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let start = text
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("no {heading:?} heading"))
        + 1;
    let body = &text[start + heading.len()..];
    let end = body
        .match_indices("\n#")
        .find(|(at, _)| {
            let hashes = body[at + 1..].chars().take_while(|c| *c == '#').count();
            hashes <= level && body[at + 1 + hashes..].starts_with(' ')
        })
        .map_or(body.len(), |(at, _)| at + 1);
    &text[start..start + heading.len() + end]
}

/// Step `n.` of the approval protocol, up to the next step or the end of
/// the list.
fn step(skill: &str, n: usize) -> &str {
    let protocol = section(skill, "### Approval protocol");
    let start = protocol
        .find(&format!("\n{n}. "))
        .unwrap_or_else(|| panic!("no step {n}"));
    let rest = &protocol[start + 1..];
    let end = rest
        .find(&format!("\n{}. ", n + 1))
        .or_else(|| rest.find("\n\n"))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// `text` with every run of whitespace one space, so a phrase reads the
/// same wherever the template wraps its lines.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_says(part: &str, which: &str, phrases: &[&str]) {
    let text = flat(part);
    let missing: Vec<_> = phrases.iter().filter(|p| !text.contains(**p)).collect();
    assert!(missing.is_empty(), "{which} lacks {missing:?}:\n{part}");
}

fn assert_never_says(part: &str, which: &str, phrases: &[&str]) {
    let text = flat(part);
    let found: Vec<_> = phrases.iter().filter(|p| text.contains(**p)).collect();
    assert!(found.is_empty(), "{which} says {found:?}:\n{part}");
}

#[test]
fn the_bootstrap_says_every_command_keeps_the_brain_in_step() {
    let skill = skill();
    let bootstrap = section(&skill, "## Bootstrap");
    assert_says(
        bootstrap,
        "Bootstrap",
        &[
            "only after the brain changes",
            "keeps it in step with your teammates",
            "You never need to sync the brain yourself",
            "`sync` key",
            "\"Teammates' changes\"",
        ],
    );
    assert_never_says(bootstrap, "Bootstrap", &["kluris sync", "merged"]);
}

#[test]
fn teammates_changes_replaces_sync_before_you_write() {
    let skill = skill();
    assert_never_says(
        &skill,
        "the skill",
        &[
            "### Sync before you write",
            "KLURIS_NO_AUTO_SYNC",
            "blocked[]",
        ],
    );
    let part = section(&skill, "### Teammates' changes");
    assert_says(
        part,
        "Teammates' changes",
        &[
            // Conflicts: stop, show both texts, ask, settle by kind.
            "`conflicts[]`",
            "STOP before writing anything else",
            "`ours`",
            "`theirs`",
            "Ask which to keep",
            "write the chosen version to the path (delete the file",
            "`kluris commit --brain acme --merge <merge> --paths <every conflicted path of that merge> --json`",
            "refuses any other path",
            "Running `kluris dream` in between is fine",
            "`structure`",
            "`binary`",
            "`kluris resolve --brain acme <path> --ours|--theirs --merge <merge> --json`",
            "`--file <merged file>`",
            "outside the brain",
            // What was left as it is.
            "`left_as_is`",
            "`uncommitted`",
            "before you write one of those files",
            "mention it once with its `detail`",
            // Commits not shared yet.
            "`to_push`",
            "`last_push`",
            "the next `kluris commit` or `kluris sync` shares them",
            "`needs_pull`: nothing to do",
        ],
    );
}

#[test]
fn step_6_says_dream_commits_the_brains_own_housekeeping() {
    let skill = skill();
    let step = step(&skill, 6);
    assert_says(
        step,
        "step 6",
        &["`committed`", "housekeeping", "nothing to approve"],
    );
}

#[test]
fn step_7_says_what_commit_does_and_reports() {
    let skill = skill();
    let step = step(&skill, 7);
    assert_says(
        step,
        "step 7",
        &[
            "`kluris commit --brain acme --paths <every brain-relative path you wrote or deleted, comma-separated> --json`",
            "`KLURIS_AGENT`",
            "`linked`",
            "brings teammates' commits in and pushes",
            "`pushed`",
            "`offline`",
            "`refused`",
            "`warning`",
            "`conflicts[]`",
            "never pass `--no-verify` unless the human asks",
            "A brain without git: skip this step.",
        ],
    );
    assert_never_says(step, "step 7", &["kluris sync"]);
}

#[test]
fn the_brain_readme_says_how_teammates_work_with_the_brain() {
    let team = section(BRAIN_README, "## Working with your team");
    assert_says(
        team,
        "the brain README's team section",
        &[
            "git clone <this-repo-url>",
            "kluris register",
            "keeps it in step",
            "kluris commit --brain {name} --paths",
            "conflict",
            "kluris commit --brain {name} --merge",
            "kluris resolve --brain {name}",
        ],
    );
    assert_never_says(
        BRAIN_README,
        "the brain README",
        &[
            "Sync (commit, push, pull) goes through `git` directly",
            "Sync and branch with git directly",
            "Use git directly to commit and sync",
            "KLURIS_NO_AUTO_SYNC",
        ],
    );
}
