//! SKILL.md rendering: the bytes of an earlier release's skill with exactly
//! the changes listed here. Every `fixtures/skills/earlier/*.md` file is an
//! earlier release's rendering for the inputs in `fixtures/skills/cases.json`
//! (frozen; never regenerated); the expected text is that file run through
//! [`earlier_to_current`], which applies these changes and nothing else:
//!
//! 1. the literal `{{brain, file, target}}` the earlier text holds becomes
//!    `{brain, file, target}`;
//! 2. a marker comment follows the frontmatter;
//! 3. the "Search" paragraph replaces the earlier search advice in "Query
//!    first -- never guess";
//! 4. the write flow: the bootstrap says every command keeps the brain in
//!    step and its `sync` key says what happened, a "Teammates' changes"
//!    section, dream's housekeeping commit and `kluris commit` after an
//!    approved write in place of the `git add -A && git commit` reminder,
//!    and `commit`/`resolve` in "CLI commands".
//!
//! Each change must match exactly once, so a template edit that moves or
//! drops one fails here instead of passing silently.

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::agents::render::{
    MARKER, SkillInput, is_generated_skill, is_generated_workflow, posix_path, render_skill,
    skill_name,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills")
}

struct Case {
    name: String,
    brain_name: String,
    brain_path: String,
    has_git: bool,
    description: String,
    companions: Vec<String>,
    companion_home: String,
}

impl Case {
    fn input(&self) -> SkillInput<'_> {
        SkillInput {
            brain_name: &self.brain_name,
            brain_path: &self.brain_path,
            has_git: self.has_git,
            description: &self.description,
            companions: &self.companions,
            companion_home: &self.companion_home,
        }
    }

    fn earlier(&self) -> String {
        read(&fixtures().join("earlier").join(format!("{}.md", self.name)))
    }

    fn earlier_workflow(&self) -> String {
        read(
            &fixtures()
                .join("earlier")
                .join(format!("{}.workflow.md", self.name)),
        )
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn cases() -> Vec<Case> {
    let text = read(&fixtures().join("cases.json"));
    let root: Value = serde_json::from_str(&text).expect("cases.json");
    let str_of = |v: &Value, k: &str| v[k].as_str().expect(k).to_string();
    root.as_object()
        .expect("object")
        .iter()
        .map(|(name, v)| Case {
            name: name.clone(),
            brain_name: str_of(v, "brain_name"),
            brain_path: str_of(v, "brain_path"),
            has_git: v["has_git"].as_bool().expect("has_git"),
            description: str_of(v, "brain_description"),
            companions: v["companions"]
                .as_array()
                .expect("companions")
                .iter()
                .map(|c| c.as_str().expect("companion").to_string())
                .collect(),
            companion_home: str_of(v, "companion_home"),
        })
        .collect()
}

/// The marker line, spelled out so a change to it shows up in review.
const MARKER_LINE: &str = "<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->";

/// The earlier search advice in "Query first -- never guess", with `{brain_flag_hint_inline}` filled in as ` --brain <name>`.
const EARLIER_SEARCH_ADVICE: &str = "\
Start with the cached wake-up snapshot. For targeted lookup, prefer
`kluris search \"<query>\"{flag} --json`; it ranks neurons,
glossary terms, and brain.md in one pass. Use `--lobe <name>`, `--tag <tag>`,
or `--limit 20` when useful. If search finds nothing, navigate manually:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files.

If nothing is documented, say so plainly: \"Nothing documented about X yet.\"
If a result is deprecated, prefer its replacement and mention that the old
knowledge was superseded.
";

/// The "Search" paragraph in its place: keywords, 2-3 parallel
/// phrasings, `title`/`heading`/`snippet` first and files opened at `line`,
/// low `coverage` is weak, "Nothing documented about X yet" instead of a
/// loose match, `related`, `replaced_by`. The `--lobe`/`--tag`/`--limit`
/// advice and the manual-navigation fallback stay.
const SEARCH_ADVICE: &str = "\
Start with the cached wake-up snapshot. For targeted lookup, search the brain.

### Search

Run `kluris search \"<2-5 distinctive words>\"{flag} --json`. Use keywords,
not a sentence. For a question that could be worded several ways, run 2-3
searches in parallel with different wording: synonyms, the team's own terms
from the wake-up glossary, and the language the brain is written in. Search
ranks neurons, glossary terms, and brain.md in one pass; use `--lobe <name>`,
`--tag <tag>`, or `--limit 20` when useful.

Read `title`, `heading` and `snippet` first, then open only the best files,
starting at `line`. Treat a top hit with low `coverage` (under about 0.5) and
no `title` or `heading` match as weak. Follow a hit's `related` neurons when
the answer spans topics. If a hit is `deprecated`, read its `replaced_by`
instead and mention that the old knowledge was superseded.

If every search is weak, navigate manually before answering:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files. If
nothing is documented, say so plainly: \"Nothing documented about X yet.\"
Do not stretch a loose match into an answer.
";

/// The earlier wake-up refresh rule and the fallback paragraph after it.
const EARLIER_REFRESH: &str = "\
this skill's remember/learn intents, `kluris dream{flag}`,
or direct file edits the user tells you about.

If the `kluris` CLI is unavailable";

/// A result that pulled refreshes the snapshot too; every command keeps the
/// brain in step, and its `sync` key says what happened.
const REFRESH: &str = "\
this skill's remember/learn intents, `kluris dream{flag}`,
a `kluris` result whose `sync.pulled` is above 0,
or direct file edits the user tells you about.

You never need to sync the brain yourself: every `kluris` command that opens
the brain keeps it in step with your teammates. `wake-up`, `search` and
`status` bring their commits in before they read (at most once a minute, so
a read right after another may not show the newest), and `kluris commit`
brings them in and pushes what you committed. Each `--json` result says what
happened in its `sync` key, which reads leave out when there is nothing to
say. Read it every time, as in \"Teammates' changes\" below.

If the `kluris` CLI is unavailable";

const EARLIER_WRITING_RULES: &str = "## Writing rules\n\n### Approval protocol\n";

/// "Teammates' changes", ahead of the approval protocol.
const WRITING_RULES: &str = "\
## Writing rules

### Teammates' changes

Read the `sync` key of every `kluris` result (`dream --json` lists it per
brain):
- `conflicts[]` is not empty: you and a teammate changed the same file, and
  a merge waits for the human. STOP before writing anything else. Show the
  human each conflict's `path` with its `ours` (this brain's text) and
  `theirs` (the teammate's text); `base` is the text both started from, and
  `null` means that side has no file there (or, with `binary` true, is not
  text). Ask which to keep, or propose a merged text under the approval
  protocol. Then settle it with the conflict's `merge` id:
  - A text conflict: write the chosen version to the path (delete the file
    to keep a deletion), then run
    `kluris commit{flag} --merge <merge> --paths <every conflicted path of that merge> --json`.
    It finishes the merge against the teammate's commit you showed. While
    the merge waits, `kluris commit` refuses any other path: commit those
    after it. Running `kluris dream` in between is fine.
  - A `structure` conflict (a file on one side, a folder on the other, or a
    rename) or a `binary` one: ask which side to keep, then run
    `kluris resolve{flag} <path> --ours|--theirs --merge <merge> --json`.
  - A merge that has both: settle each `structure` path with
    `kluris resolve` first, then write the text files and run
    `kluris commit{flag} --merge <merge> --paths <its text paths> --json`.
    It keeps the sides `kluris resolve` recorded; never give a `structure`
    path in `--paths`.
  Never write conflict markers into a brain file.
- `left_as_is` with `reason` `uncommitted`: teammates' commits did not come
  in because of the files in `paths`. Tell the human before you write one
  of those files. Any other `reason` (`offline`, `refused`, `busy`,
  `in_progress`, `hook`, `failed`, `no_upstream`): mention it once with its
  `detail`, then go on with the brain as it is.
- `to_push` above 0 with a `last_push` whose `result` is `offline`,
  `refused` or `failed`: tell the human, with its `detail`, that those
  commits are not shared yet; the next `kluris commit` or `kluris sync` shares them.
  `needs_pull`: nothing to do.

### Approval protocol
";

/// The earlier step 6 ends with a `git add -A && git commit` reminder.
const EARLIER_COMMIT_REMINDER: &str = "\
   on. Then remind them to commit with git (`cd {path} && git add -A && git commit`)
   if the brain uses git.
";

/// Step 6 notes dream's housekeeping commit; step 7: `kluris commit` the
/// approved files, which pulls and pushes.
const COMMIT_STEP: &str = "\
   on. `kluris dream` also commits the brain's own housekeeping, built from
   what is already committed (dates, a missing `parent`, reverse `related`
   links, maps, `brain.md`); `committed`, when present, lists those files.
   Mention them; there is nothing to approve. Fixes that your uncommitted
   files cause stay in the work tree for step 7.
7. **Commit and share what you wrote.** Once the human approved and the
   files are written, run
   `kluris commit{flag} --paths <every brain-relative path you wrote or deleted, comma-separated> --json`,
   with `KLURIS_AGENT` set to your agent's name (for example
   `KLURIS_AGENT=\"Claude Code\"`) so the commit credits you. It commits those
   files, the `map.md`/`brain.md` files they change and the reverse
   `related:` links they call for in neurons with no other change (listed in
   `linked`), under the human's git identity, and nothing else; every other
   uncommitted file stays as it is (`left_dirty`). Then it brings teammates'
   commits in and pushes. Report the `commit` and whether its `sync` says
   `pushed`. A commit that landed exits 0 whatever its sync did:
   - `left_as_is` `offline`: the commit is shared later, by a background
     push that a later command starts.
   - `left_as_is` `refused`: tell the human its `detail`.
   - `warning`: something failed after the commit landed; show it to the
     human, with the repair it gives.
   - `conflicts[]`: settle them as in \"Teammates' changes\".
   If a git hook stops the commit (`ok` false, nothing committed), show its
   output to the human and fix what it reports; never pass `--no-verify`
   unless the human asks for it. A brain inside a larger git repository
   gets no `sync` key: push it with git the way the team normally does. A
   brain without git: skip this step.
";

/// The earlier last "CLI commands" lines: sync and commit through plain git.
const EARLIER_CLI_TAIL: &str = "\
- `kluris mri{flag}` -- generate interactive visualization

Sync, commit, and branch operations go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} push / pull / status /
checkout` as you would with any other repo.
";

const CLI_TAIL: &str = "\
- `kluris mri{flag}` -- generate interactive visualization
- `kluris commit{flag} --paths <a.md,b.md> --json` -- commit exactly those files and the maps and links they change, bring in teammates' commits, and push; with `--merge <id>`, finish a waiting merge from the settled files (without `--paths` once every path has a choice)
- `kluris resolve{flag} <path> --ours|--theirs|--file <merged file> --merge <id> --json` -- settle a conflicted path of a waiting merge: a `structure` one with `--ours` or `--theirs`, a binary one or one `--paths` cannot name (a comma, or spaces at either end) with any of the three

Branch operations and anything else go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} status / log / checkout`
as you would with any other repo.
";

/// The brain path as the earlier text writes it (`The brain lives at `<path>` (`).
fn brain_path_of(earlier: &str) -> String {
    let start = earlier
        .find("The brain lives at `")
        .expect("brain path line")
        + "The brain lives at `".len();
    let end = start + earlier[start..].find('`').expect("closing backtick");
    earlier[start..end].to_string()
}

fn replace_once(text: &str, from: &str, to: &str, what: &str) -> String {
    let n = text.matches(from).count();
    assert_eq!(n, 1, "{what}: expected exactly one match, found {n}");
    text.replacen(from, to, 1)
}

/// An earlier release's SKILL.md with the listed changes applied.
fn earlier_to_current(earlier: &str, brain_name: &str) -> String {
    let flag = format!(" --brain {brain_name}");
    let out = replace_once(
        earlier,
        "a list of `{{brain, file, target}}` entries.",
        "a list of `{brain, file, target}` entries.",
        "brace fix",
    );
    let out = replace_once(
        &out,
        "\n---\n\n# Brain: ",
        &format!("\n---\n\n{MARKER_LINE}\n\n# Brain: "),
        "marker",
    );
    let out = replace_once(
        &out,
        &EARLIER_SEARCH_ADVICE.replace("{flag}", &flag),
        &SEARCH_ADVICE.replace("{flag}", &flag),
        "search advice",
    );
    let path = brain_path_of(earlier);
    let fill = |text: &str| text.replace("{flag}", &flag).replace("{path}", &path);
    let out = replace_once(
        &out,
        &fill(EARLIER_REFRESH),
        &fill(REFRESH),
        "the refresh rule",
    );
    let out = replace_once(
        &out,
        &fill(EARLIER_WRITING_RULES),
        &fill(WRITING_RULES),
        "teammates' changes",
    );
    let out = replace_once(
        &out,
        &fill(EARLIER_COMMIT_REMINDER),
        &fill(COMMIT_STEP),
        "commit step",
    );
    replace_once(
        &out,
        &fill(EARLIER_CLI_TAIL),
        &fill(CLI_TAIL),
        "CLI commands",
    )
}

/// The first line that differs, for a readable failure.
fn first_difference(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}:\n  got:      {x:?}\n  expected: {y:?}", i + 1);
        }
    }
    format!(
        "one is a prefix of the other ({} vs {} bytes)",
        a.len(),
        b.len()
    )
}

#[test]
fn fixture_cases_are_all_present() {
    let names: Vec<String> = cases().into_iter().map(|c| c.name).collect();
    assert_eq!(names.len(), 9, "{names:?}");
}

#[test]
fn every_case_is_the_earlier_rendering_with_exactly_the_listed_changes() {
    for case in cases() {
        let got = render_skill(&case.input());
        let expected = earlier_to_current(&case.earlier(), &case.brain_name);
        assert!(
            got == expected,
            "case {}: {}",
            case.name,
            first_difference(&got, &expected)
        );
    }
}

#[test]
fn rendered_skill_has_no_leaked_double_braces() {
    for case in cases() {
        let got = render_skill(&case.input());
        assert!(!got.contains("{{"), "case {}", case.name);
        assert!(!got.contains("}}"), "case {}", case.name);
        assert!(
            got.contains("`{brain, file, target}`"),
            "case {}",
            case.name
        );
    }
}

#[test]
fn marker_follows_the_frontmatter() {
    assert_eq!(MARKER, MARKER_LINE);
    for case in cases() {
        let got = render_skill(&case.input());
        let name = skill_name(&case.brain_name);
        let head = format!("---\nname: {name}\ndescription: \"");
        assert!(got.starts_with(&head), "case {}", case.name);
        let after = got.split_once("\n---\n\n").expect("frontmatter end").1;
        assert!(
            after.starts_with(&format!("{MARKER_LINE}\n\n# Brain: ")),
            "case {}",
            case.name
        );
    }
}

/// Every agent's frontmatter rule: `name` equals the folder
/// (`kluris-<brain>`), lowercase letters, digits and hyphens, at most 64
/// characters; the description fits the strictest limit (1,024 characters).
#[test]
fn frontmatter_fits_every_agent() {
    for case in cases() {
        let got = render_skill(&case.input());
        let mut lines = got.lines();
        assert_eq!(lines.next(), Some("---"));
        let name = lines
            .next()
            .and_then(|l| l.strip_prefix("name: "))
            .expect("name");
        assert_eq!(name, skill_name(&case.brain_name));
        assert!(name.len() <= 64);
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        );
        let desc = lines
            .next()
            .and_then(|l| l.strip_prefix("description: \""))
            .and_then(|l| l.strip_suffix('"'))
            .expect("description");
        assert!(desc.chars().count() <= 1024, "{}", desc.chars().count());
        assert_eq!(lines.next(), Some("---"));
    }
}

#[test]
fn search_paragraph_names_the_search_json_keys() {
    let case = &cases()[0];
    let got = render_skill(&case.input());
    for needle in [
        "Use keywords,\nnot a sentence.",
        "run 2-3\nsearches in parallel",
        "Read `title`, `heading` and `snippet` first",
        "starting at `line`",
        "low `coverage`",
        "Nothing documented about X yet.",
        "Do not stretch a loose match",
        "`related` neurons",
        "read its `replaced_by`",
    ] {
        assert!(got.contains(needle), "missing {needle:?}");
    }
}

/// The write flow: no sync call, `kluris commit` after an approved write,
/// conflicts settled with `kluris commit --merge` or `kluris resolve`, no
/// `git add -A`.
#[test]
fn the_write_flow_commits_and_settles_conflicts_without_a_sync() {
    for case in cases() {
        let got = render_skill(&case.input());
        let b = &case.brain_name;
        for needle in [
            format!("`kluris commit --brain {b} --paths "),
            format!("`kluris commit --brain {b} --merge <merge> --paths "),
            format!("`kluris resolve --brain {b} <path> --ours|--theirs --merge <merge> --json`"),
            "`KLURIS_AGENT=\"Claude Code\"`".to_string(),
            "never pass `--no-verify`\n   unless the human asks for it".to_string(),
            "Never write conflict markers into a brain file".to_string(),
            "the reverse\n   `related:` links they call for in neurons with no other change"
                .to_string(),
            "### Teammates' changes".to_string(),
        ] {
            assert!(
                got.contains(&needle),
                "case {}: missing {needle:?}",
                case.name
            );
        }
        for stale in [
            format!("`kluris sync --brain {b} --json`"),
            "### Sync before you write".to_string(),
            "git add -A".to_string(),
            "push / pull".to_string(),
        ] {
            assert!(!got.contains(&stale), "case {}: has {stale:?}", case.name);
        }
        // The approval protocol itself is unchanged.
        assert!(got.contains("5. STOP. NEVER write until the human explicitly approves."));
    }
}

/// Values are substituted in one pass: a description or path that contains
/// a placeholder name (`{brain_flag_hint}`, `{specmint_block}`, ...) is
/// written as it is.
#[test]
fn placeholders_inside_values_are_not_expanded() {
    let companions = vec!["specmint-core".to_string()];
    let input = SkillInput {
        brain_name: "tricky",
        brain_path: "/srv/{git_label}/{brain_name}",
        has_git: true,
        description: "Uses {brain_path}, {brain_flag_hint}, {specmint_block} and {skill_name}",
        companions: &companions,
        companion_home: "/h/{brain_description}/.kluris/companions",
    };
    let got = render_skill(&input);
    assert!(got.contains(
        "The brain lives at `/srv/{git_label}/{brain_name}` (git).\n\
         Description: Uses {brain_path}, {brain_flag_hint}, {specmint_block} and {skill_name}\n"
    ));
    assert!(
        got.contains("read `/h/{brain_description}/.kluris/companions/specmint-core/SKILL.md`")
    );
    // The description in the frontmatter is the fixed SKILL_DESCRIPTION.
    assert_eq!(got.matches("{brain_flag_hint}").count(), 1);
    assert_eq!(got.matches("{specmint_block}").count(), 1);
}

#[test]
fn default_description_and_git_label() {
    let input = SkillInput {
        brain_name: "plain",
        brain_path: "/b/plain",
        has_git: false,
        description: "",
        companions: &[],
        companion_home: "/h/.kluris/companions",
    };
    let got = render_skill(&input);
    assert!(got.contains("lives at `/b/plain` (no git).\nDescription: plain knowledge base\n"));
    assert!(!got.contains("## Spec-worthy work first"));
}

#[test]
fn posix_path_matches_the_recorded_paths() {
    let text = read(&fixtures().join("posix_paths.json"));
    let rows: Value = serde_json::from_str(&text).expect("posix_paths.json");
    let mut bad = Vec::new();
    for row in rows.as_array().expect("array") {
        let input = row[0].as_str().expect("input");
        let expected = row[1].as_str().expect("expected");
        let got = posix_path(input);
        if got != expected {
            bad.push(format!("{input:?}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn snapshot_basic() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "basic")
        .expect("basic");
    insta::assert_snapshot!("basic", render_skill(&case.input()));
}

#[test]
fn snapshot_companions_all_windows() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "companions-all")
        .expect("companions-all");
    insta::assert_snapshot!("companions_all", render_skill(&case.input()));
    let case = cases()
        .into_iter()
        .find(|c| c.name == "windows-path")
        .expect("windows-path");
    insta::assert_snapshot!("windows_path", render_skill(&case.input()));
}

// --- recognising Kluris-written skills --------------------------------------

#[test]
fn current_and_earlier_skills_are_recognised() {
    for case in cases() {
        let name = skill_name(&case.brain_name);
        assert!(
            is_generated_skill(&name, &render_skill(&case.input())),
            "current {}",
            case.name
        );
        assert!(
            is_generated_skill(&name, &case.earlier()),
            "earlier {}",
            case.name
        );
        assert!(
            is_generated_workflow(&format!("{name}.md"), &case.earlier_workflow()),
            "earlier workflow {}",
            case.name
        );
    }
}

/// Every SKILL.md and Windsurf workflow an earlier release wrote, as each
/// release rendered it (`fixtures/skills/legacy/`, frozen; never
/// regenerated): each file is named after the first release of its shape;
/// a `v1.*` file and a `-bare` file are the single-brain `kluris`, any
/// other is `kluris-acme`.
fn legacy_renderings() -> Vec<(String, String, String)> {
    let dir = fixtures().join("legacy");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).expect("legacy fixtures") {
        let path = entry.expect("entry").path();
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("name")
            .to_string();
        let stem = file.trim_end_matches(".md").trim_end_matches(".workflow");
        let bare = stem.ends_with("-bare") || stem.starts_with("v1.");
        let skill = if bare { "kluris" } else { "kluris-acme" };
        let entry_name = if file.ends_with(".workflow.md") {
            format!("{skill}.md")
        } else {
            skill.to_string()
        };
        out.push((file, entry_name, read(&path)));
    }
    out.sort();
    assert!(out.len() >= 30, "{}", out.len());
    out
}

#[test]
fn every_earlier_rendering_is_recognised() {
    for (file, name, text) in legacy_renderings() {
        let ok = if file.ends_with(".workflow.md") {
            is_generated_workflow(&name, &text)
        } else {
            is_generated_skill(&name, &text)
        };
        assert!(ok, "{file} as {name}");
        let crlf = text.replace('\n', "\r\n");
        let ok = if file.ends_with(".workflow.md") {
            is_generated_workflow(&name, &crlf)
        } else {
            is_generated_skill(&name, &crlf)
        };
        assert!(ok, "{file} as {name}, CRLF");
    }
}

/// A rendering is Kluris's only under its own name: a single-brain `kluris`
/// under `kluris-acme`, or a `kluris-acme` under another name, is not.
#[test]
fn renderings_are_recognised_only_under_their_own_name() {
    for (file, name, text) in legacy_renderings() {
        if file.ends_with(".workflow.md") {
            assert!(!is_generated_workflow("kluris-other.md", &text), "{file}");
        } else {
            assert!(!is_generated_skill("kluris-other", &text), "{file}");
            let swapped = if name == "kluris" {
                "kluris-acme"
            } else {
                "kluris"
            };
            assert!(!is_generated_skill(swapped, &text), "{file} as {swapped}");
        }
    }
}

/// The frontmatter must be the template's byte for byte: changing any one
/// character of it, quotes and the brain name included, makes the file
/// someone else's.
#[test]
fn any_change_to_the_frontmatter_is_not_recognised() {
    let mut texts: Vec<(String, String, String)> = legacy_renderings();
    let case = &cases()[0];
    texts.push((
        "earlier".into(),
        skill_name(&case.brain_name),
        case.earlier(),
    ));
    for (file, name, text) in texts {
        let end = text.find("\n---\n").expect("frontmatter end") + 1;
        let workflow = file.ends_with(".workflow.md");
        for (i, c) in text[..end].char_indices() {
            if c == '\n' && i + 1 == end {
                continue;
            }
            let other = if c == 'x' { 'y' } else { 'x' };
            let changed = format!("{}{other}{}", &text[..i], &text[i + c.len_utf8()..]);
            let ok = if workflow {
                is_generated_workflow(&name, &changed)
            } else {
                is_generated_skill(&name, &changed)
            };
            assert!(!ok, "{file}: byte {i} ({c:?}) changed is still recognised");
        }
    }
}

/// The body must carry the release's fixed headings in order: a missing or
/// reordered heading makes the file someone else's.
#[test]
fn a_missing_or_reordered_heading_is_not_recognised() {
    let case = &cases()[0];
    let name = skill_name(&case.brain_name);
    let earlier = case.earlier();
    assert!(is_generated_skill(&name, &earlier));
    for heading in ["## Bootstrap", "## Writing rules", "## CLI commands"] {
        let line = format!("\n{heading}\n");
        assert_eq!(earlier.matches(&line).count(), 1, "{heading}");
        let dropped = earlier.replace(&line, "\n");
        assert!(!is_generated_skill(&name, &dropped), "without {heading}");
    }
    let swapped = earlier
        .replace("\n## Bootstrap\n", "\n## @@\n")
        .replace("\n## CLI commands\n", "\n## Bootstrap\n")
        .replace("\n## @@\n", "\n## CLI commands\n");
    assert!(!is_generated_skill(&name, &swapped), "reordered");
    let brain = earlier.replace("\n# Brain: acme\n", "\n# Brain: other\n");
    assert!(
        !is_generated_skill(&name, &brain),
        "another brain's heading"
    );
}

/// The marker counts only on its own line right after the frontmatter and
/// its blank line. (The current rendering is an earlier release's template
/// with the listed changes, so it is recognised by that too: a user's file
/// shows the marker rule alone.)
#[test]
fn the_marker_counts_only_at_its_place() {
    let case = &cases()[0];
    assert!(is_generated_skill(
        &skill_name(&case.brain_name),
        &render_skill(&case.input())
    ));
    let marked =
        format!("---\nname: kluris-notes\ndescription: x\n---\n\n{MARKER_LINE}\n\n# Notes\n");
    // A user's skill that copies the marker below its own frontmatter is
    // marked: the marker is the promise.
    assert!(is_generated_skill("kluris-notes", &marked));
    let block = format!("\n---\n\n{MARKER_LINE}\n\n");
    for moved in [
        marked.replace(&block, &format!("\n---\n\n\n{MARKER_LINE}\n\n")),
        marked.replace(&block, &format!("\n---\n{MARKER_LINE}\n\n")),
        marked.replace(&block, &format!("\n---\n\n {MARKER_LINE}\n\n")),
        marked.replace(&block, &format!("\n---\n\n{MARKER_LINE} extra\n\n")),
        marked.replace(&block, "\n---\n\n<!-- kluris:generated -->\n\n"),
        marked.replace(&block, &format!("\n---\n\n# Notes\n{MARKER_LINE}\n\n")),
        marked.replace("description: x\n", "description: x\nmetadata: y\n"),
        marked.replace("description: x\n", "description:\n"),
    ] {
        assert_ne!(moved, marked);
        assert!(!is_generated_skill("kluris-notes", &moved), "{moved}");
    }
    assert!(!is_generated_skill("kluris-other", &marked));
}

/// A user's own `kluris-notes` whose
/// description merely opens like Kluris's is not Kluris's.
#[test]
fn a_description_that_only_opens_like_kluris_is_not_recognised() {
    for text in [
        "---\nname: kluris-notes\ndescription: \"You are the team's subject matter expert on my personal notes\"\n---\n\n# Notes\n",
        "---\nname: kluris-notes\ndescription: You are the team's subject matter expert on my personal notes\n---\n\n# Notes\n",
        "---\nname: kluris\ndescription: You are the team's subject matter expert. Internalize the brain\n---\n\n# Kluris\n",
        "---\nname: kluris\ndescription: \"You are the team's subject matter expert. Your knowledge comes from a brain\"\n---\n\nbody\n",
    ] {
        let name = if text.contains("name: kluris\n") {
            "kluris"
        } else {
            "kluris-notes"
        };
        assert!(!is_generated_skill(name, text), "{text}");
    }
    let wf =
        "---\ndescription: You are the team's subject matter expert. Your knowledge\n---\n\nbody\n";
    assert!(!is_generated_workflow("kluris.md", wf));
}

#[test]
fn foreign_skills_are_not_recognised() {
    let own = "---\nname: kluris-notes\ndescription: My own notes about kluris\n---\n\n# Notes\n";
    assert!(!is_generated_skill("kluris-notes", own));
    // An earlier SKILL.md copied under another folder name is not that folder's.
    let case = &cases()[0];
    assert!(!is_generated_skill("kluris-other", &case.earlier()));
    // A file that mentions the marker in its body is not marked.
    let quoted = format!("---\nname: kluris-notes\ndescription: x\n---\n\nSee `{MARKER_LINE}`.\n");
    assert!(!is_generated_skill("kluris-notes", &quoted));
    assert!(!is_generated_skill("kluris-notes", ""));
    assert!(!is_generated_workflow("kluris-notes.md", "# my workflow\n"));
    assert!(!is_generated_workflow(
        "kluris-notes.md",
        "---\ndescription: \"My workflow\"\n---\n"
    ));
}

/// CRLF copies (an install on Windows that a user's editor or git turned
/// into CRLF) are still recognised.
#[test]
fn crlf_copies_are_recognised() {
    let case = &cases()[0];
    let name = skill_name(&case.brain_name);
    let current = render_skill(&case.input()).replace('\n', "\r\n");
    assert!(is_generated_skill(&name, &current));
    assert!(is_generated_skill(
        &name,
        &case.earlier().replace('\n', "\r\n")
    ));
}
