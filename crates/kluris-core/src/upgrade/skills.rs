//! The agent skills earlier releases wrote: the SKILL.md (and Windsurf
//! workflow) shapes they rendered, so an install removes their copies and
//! never a user's own `kluris-*` folder, and the folders those copies are
//! in, which every agent now reads through `~/.agents/skills` or its own
//! folder (Codex would list two enabled entries, Devin an `agents:` and a
//! `claude:` copy).
//!
//! The shapes are [`ERAS`], one [`Era`] per group of releases that render
//! alike, generated from each release's own `agents.py`; frozen, do not
//! edit.

use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::render::lf;
use crate::scaffold::is_valid_v2_name;

/// The skill folders earlier releases wrote to, relative to HOME.
/// `~/.copilot/skills` is one only while `COPILOT_HOME` is unset. The
/// Windsurf folders are found under `~/.codeium` ([`folders`]).
pub const SKILL_FOLDERS: [&str; 7] = [
    ".cursor/skills",
    ".codex/skills",
    ".copilot/skills",
    ".gemini/skills",
    ".kilo/skills",
    ".kilocode/skills",
    ".junie/skills",
];

/// A folder earlier releases wrote into, by what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Folder {
    /// Skill folders, each with a SKILL.md.
    Skills(PathBuf),
    /// Windsurf workflow files.
    Workflows(PathBuf),
}

/// Every folder earlier releases wrote skills (or Windsurf workflows) to,
/// for `home` and an optional `COPILOT_HOME`, in the order they are
/// cleared: the skills, then the workflows, of every `~/.codeium/windsurf*`
/// folder (Windsurf, Windsurf Next, Windsurf Insiders), sorted by name;
/// then [`SKILL_FOLDERS`], without `~/.copilot/skills` while `COPILOT_HOME`
/// is set.
pub fn folders(home: &Path, copilot_home: Option<&Path>) -> Vec<Folder> {
    let roots: Vec<PathBuf> = sorted_dirs(&home.join(".codeium"))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("windsurf"))
        })
        .collect();
    let mut out: Vec<Folder> = roots
        .iter()
        .map(|r| Folder::Skills(r.join("skills")))
        .collect();
    out.extend(
        roots
            .iter()
            .map(|r| Folder::Workflows(r.join("global_workflows"))),
    );
    for rel in SKILL_FOLDERS {
        if rel == ".copilot/skills" && copilot_home.is_some() {
            continue;
        }
        out.push(Folder::Skills(
            rel.split('/')
                .fold(home.to_path_buf(), |p, part| p.join(part)),
        ));
    }
    out
}

/// The directories directly inside `dir` (a symlink to one counts), sorted
/// by name; none when `dir` cannot be read.
fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// How every earlier release's description opens.
const DESCRIPTION_OPENINGS: [&str; 2] = [
    "You are the team's subject matter expert",
    "Your team's shared AI brain",
];

/// Whether `text` holds an earlier release's description opening anywhere:
/// with [`is_earlier_skill`] false, a near miss.
pub fn has_earlier_trait(text: &str) -> bool {
    DESCRIPTION_OPENINGS.iter().any(|o| text.contains(o))
}

/// Whether `skill_md`, the SKILL.md of the folder `dir_name` (its name as
/// the folder listing gives it), is one an earlier release wrote: that
/// release's frontmatter byte for byte, with only the brain name varying
/// (`kluris-<brain>`, or the single-brain `kluris`), and its headings in
/// order in the body. A user's `kluris-notes` whose description merely
/// opens alike is not. CRLF reads as LF.
pub fn is_earlier_skill(dir_name: &str, skill_md: &str) -> bool {
    let text = lf(skill_md);
    ERAS.iter().any(|era| {
        let frontmatter = |desc: &str| skill_frontmatter(era.style, dir_name, desc);
        matches_era(era, dir_name, &text, frontmatter)
    })
}

/// Whether `text`, the file `file_name` in a Windsurf `global_workflows`
/// folder, is a workflow an earlier release wrote (`kluris-<brain>.md` or
/// `kluris.md`): that release's frontmatter byte for byte and its headings
/// in order.
pub fn is_earlier_workflow(file_name: &str, text: &str) -> bool {
    let Some(stem) = file_name.strip_suffix(".md") else {
        return false;
    };
    let text = lf(text);
    ERAS.iter().any(|era| {
        era.workflow != Workflow::None
            && matches_era(era, stem, &text, |desc| {
                workflow_frontmatter(era.workflow, desc)
            })
    })
}

/// The brain an `era` file named `name` (`kluris-<brain>` or `kluris`)
/// would be for, then whether `text` is that file: `frontmatter(desc)`
/// built from the era's description with that brain, then its headings.
fn matches_era(era: &Era, name: &str, text: &str, frontmatter: impl Fn(&str) -> String) -> bool {
    let slotted = era.description.contains(BRAIN_SLOT);
    let brain = match name.strip_prefix("kluris-") {
        // A brain's skill is `kluris-<brain>`; the first releases wrote
        // only `kluris`.
        Some(brain) if slotted => brain.to_string(),
        Some(_) => return false,
        None if name != "kluris" => return false,
        None if !slotted => String::new(),
        // The single-brain `kluris` of a slotted era: the brain is the one
        // its description names.
        None => {
            let template = frontmatter(era.description);
            let Some(slot) = template.find(BRAIN_SLOT) else {
                return false;
            };
            let Some(rest) = text.strip_prefix(&template[..slot]) else {
                return false;
            };
            rest.chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                .collect()
        }
    };
    if slotted && !is_valid_v2_name(&brain) {
        return false;
    }
    let expected = frontmatter(&era.description.replace(BRAIN_SLOT, &brain));
    text.strip_prefix(&expected)
        .is_some_and(|body| headings_in_order(body, era.headings, &brain))
}

/// The one placeholder of an earlier description and heading.
const BRAIN_SLOT: &str = "{brain_name}";

/// An earlier SKILL.md's frontmatter and the blank line after it.
fn skill_frontmatter(style: Style, name: &str, description: &str) -> String {
    match style {
        Style::Folded => format!(
            "---\nname: {name}\ndescription: >\n  {description}\n\
             allowed-tools: Read, Write, Bash, Glob, Grep\n---\n\n"
        ),
        Style::Plain => format!("---\nname: {name}\ndescription: {description}\n---\n\n"),
        Style::Quoted => format!(
            "---\nname: {name}\ndescription: \"{}\"\n---\n\n",
            description.replace('"', "\\\"")
        ),
    }
}

/// An earlier Windsurf workflow's frontmatter: the description's first 200
/// characters (Python's `[:200]`).
fn workflow_frontmatter(style: Workflow, description: &str) -> String {
    let cut: String = description.chars().take(200).collect();
    match style {
        Workflow::Quoted => format!(
            "---\ndescription: \"{}\"\n---\n\n",
            cut.replace('"', "\\\"")
        ),
        _ => format!("---\ndescription: {cut}\n---\n\n"),
    }
}

/// Every one of `headings` (with `brain` in its slot) is a whole line of
/// `body`, in that order.
fn headings_in_order(body: &str, headings: &[&str], brain: &str) -> bool {
    let mut lines = body.split('\n');
    headings.iter().all(|heading| {
        let want = heading.replace(BRAIN_SLOT, brain);
        lines.any(|line| line == want)
    })
}

/// What every SKILL.md (and Windsurf workflow) of a group of releases
/// shares.
struct Era {
    /// The releases, `first..=last`.
    #[allow(dead_code)]
    releases: &'static str,
    /// `SKILL_DESCRIPTION`; `{brain_name}` is the only slot.
    description: &'static str,
    /// How the SKILL.md frontmatter writes the description.
    style: Style,
    /// The heading lines (`#` to `###`) of `SKILL_BODY`, in order.
    headings: &'static [&'static str],
    /// How the release wrote Windsurf workflows.
    workflow: Workflow,
}

/// The frontmatter of an era's SKILL.md: `name: <folder>` and the
/// description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `description: >` folded, then `allowed-tools: ...` (the first
    /// releases).
    Folded,
    /// `description: <text>` unquoted.
    Plain,
    /// `description: "<text>"`, `"` escaped.
    Quoted,
}

/// The frontmatter of an era's Windsurf workflow: `description:` and the
/// first 200 characters of the description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Workflow {
    /// The release wrote none.
    None,
    /// Unquoted (the first releases).
    Plain,
    /// Quoted, `"` escaped.
    Quoted,
}

/// The releases' shapes, oldest first.
const ERAS: &[Era] = &[
    Era {
        releases: "v1.0.0..=v1.0.1",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to recall, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Folded,
        headings: &[
            "## You are the team's subject matter expert",
            "## Reading protocol",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::None,
    },
    Era {
        releases: "v1.0.2..=v1.0.3",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to recall, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Folded,
        headings: &[
            "## You are the team's subject matter expert",
            "## Reading protocol",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Plain,
    },
    Era {
        releases: "v1.0.4",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to recall, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Plain,
        headings: &[
            "## You are the team's subject matter expert",
            "## Reading protocol",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Plain,
    },
    Era {
        releases: "v1.0.5..=v1.0.9",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to recall, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## You are the team's subject matter expert",
            "## Reading protocol",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v1.1.0..=v1.2.1",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to recall, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v1.2.2..=v1.4.10",
        description: "Your team's shared AI brain -- a git-backed knowledge base of architecture decisions, service docs, API specs, conventions, and learnings. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", architecture decisions, service documentation, API endpoints, coding standards, deployment docs, \"remember this\", \"store this decision\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks about how services work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v1.5.0",
        description: "You are the team's subject matter expert. Your knowledge comes from a shared SME knowledge -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v1.5.1..=v1.5.6",
        description: "You are the team's subject matter expert. Your knowledge comes from a a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v1.6.2",
        description: "You are the team's subject matter expert. Your knowledge comes from a a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain paths are baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "## Bootstrap",
            "## Query first -- never guess",
            "## Brain selection",
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v2.0.0",
        description: "You are the team's subject matter expert for the {brain_name} brain. Your knowledge comes from a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain path is baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "# Brain: {brain_name}",
            "## Bootstrap",
            "## Query first -- never guess",
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v2.1.0..=v2.2.9",
        description: "You are the team's subject matter expert for the {brain_name} brain. Your knowledge comes from a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain path is baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "# Brain: {brain_name}",
            "## Bootstrap",
            "## Query first -- never guess",
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "## Writing rules",
            "# Auth flow",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v2.3.0..=v2.10.2",
        description: "You are the team's subject matter expert for the {brain_name} brain. Your knowledge comes from a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain path is baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "# Brain: {brain_name}",
            "## Bootstrap",
            "## Query first -- never guess",
            "## You are the team's subject matter expert",
            "## How the brain is structured",
            "## Intent detection",
            "# parent: ./map.md",
            "# related: [./auth.md]",
            "# tags: [api, openapi]",
            "# title: Payments API",
            "# updated: 2026-04-09",
            "## Writing rules",
            "# Auth flow",
            "## CLI commands (for mechanical operations)",
        ],
        workflow: Workflow::Quoted,
    },
    Era {
        releases: "v2.11.0..=v2.28.11",
        description: "You are the team's subject matter expert for the {brain_name} brain. Your knowledge comes from a brain -- a git-backed repo of architecture, decisions, conventions, and learnings curated by humans. Use this skill whenever the user mentions: brain, team knowledge, \"what do we know about\", decisions, documentation, API endpoints, conventions, deployment, \"remember this\", \"store this\", or wants to search, learn, document, or work with shared project knowledge. Also trigger when the user asks how things work, why decisions were made, or needs context from other projects. The brain path is baked in below -- do not search for config files.",
        style: Style::Quoted,
        headings: &[
            "# Brain: {brain_name}",
            "## Bootstrap",
            "## Query first -- never guess",
            "### When NOT to check the brain",
            "## Brain vs current project",
            "## How the brain is structured",
            "## Intent detection",
            "# parent: ./map.md",
            "# related: [./auth.md]",
            "# tags: [api, openapi]",
            "# title: Payments API",
            "# updated: 2026-04-09",
            "## Writing rules",
            "### Approval protocol",
            "# Auth flow",
            "## CLI commands",
        ],
        workflow: Workflow::Quoted,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every era's description opens with one of the openings a near miss
    /// is told by, so no era can slip past `kept`.
    #[test]
    fn every_era_opens_with_a_known_description_opening() {
        for era in ERAS {
            assert!(
                DESCRIPTION_OPENINGS
                    .iter()
                    .any(|o| era.description.starts_with(o)),
                "{}",
                era.releases
            );
        }
    }
}
