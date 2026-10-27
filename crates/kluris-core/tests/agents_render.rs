//! SKILL.md rendering: v3 writes v2.28.11's bytes with exactly the decided
//! changes. Every `fixtures/skills/v2/*.md` file is v2's own
//! `kluris.core.agents.render_skill` output for the inputs in
//! `fixtures/skills/cases.json` (`parity/tools/gen_skill_fixtures.py`); the
//! expected v3 text is that file run through [`v2_to_v3`], which applies the
//! decided changes and nothing else:
//!
//! 1. the literal `{{brain, file, target}}` v2 leaks becomes `{brain, file,
//!    target}` (interview-01 #4, PAR entry "SKILL.md placeholder fixes");
//! 2. a marker comment follows the frontmatter (interview-01 #6);
//! 3. the "Search" paragraph of research-05 §6 replaces v2's search advice
//!    in "Query first -- never guess" (SPEC decision 2026-10-24, search);
//! 4. v3.1's write flow (SPEC Phase 7, items 49/50): reads note the fresh
//!    reads, a "Sync before you write" section, `kluris commit` and `kluris
//!    sync` after an approved write instead of v2's `git add -A && git
//!    commit` reminder, and `sync`/`commit`/`resolve` in "CLI commands".
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

    fn v2(&self) -> String {
        read(&fixtures().join("v2").join(format!("{}.md", self.name)))
    }

    fn v2_workflow(&self) -> String {
        read(
            &fixtures()
                .join("v2")
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

/// v2's search advice in "Query first -- never guess" (`agents.py:38-46`),
/// with `{brain_flag_hint_inline}` filled in as ` --brain <name>`.
const V2_SEARCH_ADVICE: &str = "\
Start with the cached wake-up snapshot. For targeted lookup, prefer
`kluris search \"<query>\"{flag} --json`; it ranks neurons,
glossary terms, and brain.md in one pass. Use `--lobe <name>`, `--tag <tag>`,
or `--limit 20` when useful. If search finds nothing, navigate manually:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files.

If nothing is documented, say so plainly: \"Nothing documented about X yet.\"
If a result is deprecated, prefer its replacement and mention that the old
knowledge was superseded.
";

/// research-05 §6's "Search" paragraph in its place: keywords, 2-3 parallel
/// phrasings, `title`/`heading`/`snippet` first and files opened at `line`,
/// low `coverage` is weak, "Nothing documented about X yet" instead of a
/// loose match, `related`, `replaced_by`. v2's `--lobe`/`--tag`/`--limit`
/// advice and the manual-navigation fallback stay.
const V3_SEARCH_ADVICE: &str = "\
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

/// v2's wake-up refresh rule and the fallback paragraph after it.
const V2_REFRESH: &str = "\
this skill's remember/learn intents, `kluris dream{flag}`,
or direct file edits the user tells you about.

If the `kluris` CLI is unavailable";

/// A merging sync refreshes the snapshot too; reads pull on their own.
const V3_REFRESH: &str = "\
this skill's remember/learn intents, `kluris dream{flag}`,
a `kluris sync{flag}` that reports `merged`,
or direct file edits the user tells you about.

You do not need to pull the brain: `kluris wake-up` and `kluris search` fetch
the teammates' pushed changes on their own (at most once a minute) and bring
the brain up to date when nothing local is in the way. Setting
`KLURIS_NO_AUTO_SYNC=1` turns this off.

If the `kluris` CLI is unavailable";

const V2_WRITING_RULES: &str = "## Writing rules\n\n### Approval protocol\n";

/// "Sync before you write", ahead of the approval protocol.
const V3_WRITING_RULES: &str = "\
## Writing rules

### Sync before you write

Before you write, change, or delete any brain file, run
`kluris sync{flag} --json` and read the brain's entry in `brains[]`:
- `conflicts[]` is not empty: this brain and a teammate's push changed the
  same thing (or a conflict is already open: then `refused` says so too).
  STOP and show the human each conflict: its `path`, its `kind` (`content`,
  `delete_edit`, `parent`, `frontmatter`, `structure`, or `ambiguous_base`),
  and what each side changed. The conflict's `base`, `ours`, and `theirs` are blob ids: read
  them with `git -C {path} show <id>`, or compare with
  `git -C {path} diff <base> <ours>` and `git -C {path} diff <base> <theirs>`
  (a `null` side deleted the file). Ask which version to keep, or propose a
  merged text and apply the approval protocol to it. Then settle each path
  with `kluris resolve{flag} <path> --ours`, `--theirs`, or
  `--file <merged file>`; the last one commits the merge and pushes it. A
  `structure` conflict (a file on one side where the other has a directory,
  or a rename) is settled with `--ours` or `--theirs`, which keep that side's
  whole structure.
  Brain files never contain conflict markers, so never edit a file to
  resolve a conflict. If `kluris resolve` says the conflict is stale, run
  `kluris resolve{flag} --abort` and sync again.
- `blocked[]` is not empty: those files have uncommitted changes that the
  incoming changes would overwrite. Ask the human to commit or stash them,
  then sync again.
- `refused` is set with no `conflicts[]`, or `offline` is true: tell the
  human, then go on with the local brain.

### Approval protocol
";

/// v2's step 6 ends with a `git add -A && git commit` reminder.
const V2_COMMIT_REMINDER: &str = "\
   on. Then remind them to commit with git (`cd {path} && git add -A && git commit`)
   if the brain uses git.
";

/// Step 7: `kluris commit` the approved files, then `kluris sync`.
const V3_COMMIT_STEP: &str = "\
   on.
7. **Commit and share what you wrote.** Once the human approved and the
   files are written, commit exactly those files with
   `kluris commit{flag} --paths <every brain-relative path you wrote or deleted, comma-separated> --json`,
   with `KLURIS_AGENT` set to your agent's name (for example
   `KLURIS_AGENT=\"Claude Code\"`) so the commit credits you. It commits those
   files and the `map.md`/`brain.md` files they change, under the human's git
   identity, and nothing else. If its `left_dirty` lists files that
   `kluris dream` changed because of this write (a reverse `related:` link,
   a refreshed date), show them to the human and, once they agree, commit
   them the same way; leave every other uncommitted file alone. Then run
   `kluris sync{flag} --json` and report
   the `commit` and whether it was `pushed`. If sync reports `conflicts[]` or
   `blocked[]`, handle them as in \"Sync before you write\". If a git hook
   stops the commit, show its output to the human and fix what it reports;
   never pass `--no-verify` unless the human asks for it. A brain inside a
   larger git repository (sync refuses it): commit with `kluris commit`, then
   push with git the way the team normally does. A brain without git: skip
   this step.
";

/// v2's last "CLI commands" lines: sync and commit through plain git.
const V2_CLI_TAIL: &str = "\
- `kluris mri{flag}` -- generate interactive visualization

Sync, commit, and branch operations go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} push / pull / status /
checkout` as you would with any other repo.
";

const V3_CLI_TAIL: &str = "\
- `kluris mri{flag}` -- generate interactive visualization
- `kluris sync{flag} --json` -- fetch, merge, and push the brain; reports `conflicts[]`, `blocked[]`, `refused`, and `offline`
- `kluris commit{flag} --paths <a.md,b.md> --json` -- commit exactly those files and the maps they change
- `kluris resolve{flag} <path> --ours|--theirs|--file <merged file> --json` -- settle a sync conflict path by path (`--abort` drops it)

Branch operations and anything else go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} status / log / checkout`
as you would with any other repo.
";

/// The brain path as v2 wrote it (`The brain lives at `<path>` (`).
fn brain_path_of(v2: &str) -> String {
    let start =
        v2.find("The brain lives at `").expect("brain path line") + "The brain lives at `".len();
    let end = start + v2[start..].find('`').expect("closing backtick");
    v2[start..end].to_string()
}

fn replace_once(text: &str, from: &str, to: &str, what: &str) -> String {
    let n = text.matches(from).count();
    assert_eq!(n, 1, "{what}: expected exactly one match, found {n}");
    text.replacen(from, to, 1)
}

/// v2's SKILL.md with the three decided changes applied.
fn v2_to_v3(v2: &str, brain_name: &str) -> String {
    let flag = format!(" --brain {brain_name}");
    let out = replace_once(
        v2,
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
        &V2_SEARCH_ADVICE.replace("{flag}", &flag),
        &V3_SEARCH_ADVICE.replace("{flag}", &flag),
        "search advice",
    );
    let path = brain_path_of(v2);
    let fill = |text: &str| text.replace("{flag}", &flag).replace("{path}", &path);
    let out = replace_once(&out, &fill(V2_REFRESH), &fill(V3_REFRESH), "fresh reads");
    let out = replace_once(
        &out,
        &fill(V2_WRITING_RULES),
        &fill(V3_WRITING_RULES),
        "sync before you write",
    );
    let out = replace_once(
        &out,
        &fill(V2_COMMIT_REMINDER),
        &fill(V3_COMMIT_STEP),
        "commit step",
    );
    replace_once(&out, &fill(V2_CLI_TAIL), &fill(V3_CLI_TAIL), "CLI commands")
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
fn every_case_is_v2_with_exactly_the_decided_changes() {
    for case in cases() {
        let got = render_skill(&case.input());
        let expected = v2_to_v3(&case.v2(), &case.brain_name);
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

/// Every agent's frontmatter rule (research-04): `name` equals the folder
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

/// v3.1's write flow: sync before writing, `kluris commit` and `kluris
/// sync` after, conflicts settled with `kluris resolve`, no `git add -A`.
#[test]
fn the_write_flow_syncs_commits_and_resolves() {
    for case in cases() {
        let got = render_skill(&case.input());
        let b = &case.brain_name;
        for needle in [
            format!("`kluris sync --brain {b} --json`"),
            format!("`kluris commit --brain {b} --paths "),
            format!("`kluris resolve --brain {b} <path> --ours`"),
            format!("`kluris resolve --brain {b} --abort`"),
            "`KLURIS_AGENT=\"Claude Code\"`".to_string(),
            "`KLURIS_NO_AUTO_SYNC=1`".to_string(),
            "never pass `--no-verify` unless the human asks for it".to_string(),
            "Brain files never contain conflict markers".to_string(),
            "If its `left_dirty` lists files that\n   `kluris dream` changed".to_string(),
            "### Sync before you write".to_string(),
            "a `kluris sync --brain ".to_string(),
        ] {
            assert!(
                got.contains(&needle),
                "case {}: missing {needle:?}",
                case.name
            );
        }
        assert!(!got.contains("git add -A"), "case {}", case.name);
        assert!(!got.contains("push / pull"), "case {}", case.name);
        // The approval protocol itself is unchanged.
        assert!(got.contains("5. STOP. NEVER write until the human explicitly approves."));
    }
}

/// Values are substituted in one pass: a description or path that contains
/// a placeholder name is written as it is (v2 expands `{brain_flag_hint}`,
/// `{specmint_block}`, ... found inside an earlier value).
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
fn posix_path_matches_v2() {
    let text = read(&fixtures().join("posix_paths.json"));
    let rows: Value = serde_json::from_str(&text).expect("posix_paths.json");
    let mut bad = Vec::new();
    for row in rows.as_array().expect("array") {
        let input = row[0].as_str().expect("input");
        let expected = row[1].as_str().expect("expected");
        let got = posix_path(input);
        if got != expected {
            bad.push(format!("{input:?}: got {got:?}, v2 {expected:?}"));
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
fn v3_and_v2_skills_are_recognised() {
    for case in cases() {
        let name = skill_name(&case.brain_name);
        assert!(
            is_generated_skill(&name, &render_skill(&case.input())),
            "v3 {}",
            case.name
        );
        assert!(is_generated_skill(&name, &case.v2()), "v2 {}", case.name);
        assert!(
            is_generated_workflow(&format!("{name}.md"), &case.v2_workflow()),
            "v2 workflow {}",
            case.name
        );
    }
}

/// Every SKILL.md and Windsurf workflow v1.0.0 to v2.28.11 wrote, as each
/// release's own `agents.py` renders it (`fixtures/skills/legacy/`, from
/// `parity/tools/gen_legacy_skills.py`): `<first release>.md` is
/// `kluris-acme` (v2) or `kluris` (v1), `-bare` is v2.0's single-brain
/// `kluris`.
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
fn every_v1_and_v2_rendering_is_recognised() {
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

/// A rendering is Kluris's only under its own name: a bare v1/v2.0 `kluris`
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
    texts.push(("v2.28.11".into(), skill_name(&case.brain_name), case.v2()));
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
    let v2 = case.v2();
    assert!(is_generated_skill(&name, &v2));
    for heading in ["## Bootstrap", "## Writing rules", "## CLI commands"] {
        let line = format!("\n{heading}\n");
        assert_eq!(v2.matches(&line).count(), 1, "{heading}");
        let dropped = v2.replace(&line, "\n");
        assert!(!is_generated_skill(&name, &dropped), "without {heading}");
    }
    let swapped = v2
        .replace("\n## Bootstrap\n", "\n## @@\n")
        .replace("\n## CLI commands\n", "\n## Bootstrap\n")
        .replace("\n## @@\n", "\n## CLI commands\n");
    assert!(!is_generated_skill(&name, &swapped), "reordered");
    let brain = v2.replace("\n# Brain: acme\n", "\n# Brain: other\n");
    assert!(
        !is_generated_skill(&name, &brain),
        "another brain's heading"
    );
}

/// v3's marker counts only on its own line right after the frontmatter
/// and its blank line. (v3's own rendering is v2.11's template with the
/// decided changes, so it is recognised by that too: a user's file shows
/// the marker rule alone.)
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

/// T-103 `description-prefix`: a user's own `kluris-notes` whose
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
    // A v2 SKILL.md copied under another folder name is not that folder's.
    let case = &cases()[0];
    assert!(!is_generated_skill("kluris-other", &case.v2()));
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

/// CRLF copies (a v2 install on Windows that a user's editor or git turned
/// into CRLF) are still recognised.
#[test]
fn crlf_copies_are_recognised() {
    let case = &cases()[0];
    let name = skill_name(&case.brain_name);
    let v3 = render_skill(&case.input()).replace('\n', "\r\n");
    assert!(is_generated_skill(&name, &v3));
    assert!(is_generated_skill(&name, &case.v2().replace('\n', "\r\n")));
}
