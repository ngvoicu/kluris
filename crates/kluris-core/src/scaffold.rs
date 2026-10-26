//! A new brain's files, v2's `core/brain.py` (research-03 §2.2): the brain
//! types, the name rules, the scaffold `create` writes, the `--from-config`
//! structure, and the identity `register` reads from `brain.md`.
//!
//! The scaffold is v2's bytes: each lobe's `map.md`, `kluris.yml`,
//! `brain.md`, `glossary.md`, `README.md` and `.gitignore`, written as one
//! change set. Differences from v2, each allowlisted: new brain names follow
//! the stricter rule ([`is_valid_new_name`]), and a `--from-config` lobe name
//! that is a path or a lobe with a nested value is refused
//! ([`structure_from_config`]).

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::brain::change::{ChangeSet, FileWrite};
use crate::brain::store::{BrainStore, LocalGitStore};
use crate::config::{BrainConfig, brain_config_to_yaml};
use crate::fileops::under;
use crate::frontmatter::{self, py_splitlines};
use crate::pycase;
use crate::pyrepr::{py_repr_str, py_str};
use crate::yaml::{self, Date, Value, date_iso, is_py_space};

/// The brain types and their lobes, in v2's order (`BRAIN_TYPES`).
pub const BRAIN_TYPES: [(&str, &[(&str, &str)]); 5] = [
    (
        "product-group",
        &[
            (
                "projects",
                "Per-project sub-folders with APIs, data models, setup, conventions",
            ),
            (
                "infrastructure",
                "Hosting, CI/CD, Docker, deployment, environments, env vars",
            ),
            (
                "knowledge",
                "Decisions, learnings, troubleshooting tips, domain expertise",
            ),
        ],
    ),
    (
        "personal",
        &[
            (
                "projects",
                "Sub-folder per project: branches, status, TODOs",
            ),
            ("tasks", "Current priorities, blockers, in-progress work"),
            ("notes", "Daily notes, ideas, learnings"),
        ],
    ),
    (
        "product",
        &[
            ("prd", "Requirements, user stories, acceptance criteria"),
            (
                "features",
                "Sub-folder per feature: specs, status, feedback",
            ),
            ("ux", "User research, personas, journey maps, wireframes"),
            ("analytics", "Metrics, KPIs, experiment results"),
            ("competitors", "Competitive analysis, market positioning"),
            ("decisions", "Product decisions and rationale"),
        ],
    ),
    (
        "research",
        &[
            ("literature", "Papers, articles, summaries, key findings"),
            ("experiments", "Hypotheses, methodology, results"),
            ("findings", "Synthesized insights, conclusions"),
            ("datasets", "Data sources, schemas, access notes"),
            ("tools", "Research tools, scripts, environments"),
            ("questions", "Open questions, hypotheses to test"),
        ],
    ),
    ("blank", &[]),
];

/// The default brain type.
pub const DEFAULT_BRAIN_TYPE: &str = "product-group";

/// The longest brain name (in characters).
pub const NAME_MAX_LENGTH: usize = 48;

/// The name `--brain all` takes.
pub const RESERVED_NAME: &str = "all";

/// A lobe: its directory name and one-line description.
pub type Lobe = (String, String);

/// Whether `name` is one of the brain types.
pub fn is_brain_type(name: &str) -> bool {
    BRAIN_TYPES.iter().any(|(t, _)| *t == name)
}

/// `click.Choice`'s message for a value that is not a brain type.
pub fn not_a_brain_type(value: &str) -> String {
    let choices: Vec<String> = BRAIN_TYPES.iter().map(|(t, _)| format!("'{t}'")).collect();
    format!(
        "{} is not one of {}.",
        py_repr_str(value),
        choices.join(", ")
    )
}

/// The lobes of a brain type; an unknown type is `blank` (v2's
/// `get_type_defaults`).
pub fn type_lobes(brain_type: &str) -> Vec<Lobe> {
    BRAIN_TYPES
        .iter()
        .find(|(t, _)| *t == brain_type)
        .map_or(&[][..], |(_, lobes)| lobes)
        .iter()
        .map(|(name, description)| (name.to_string(), description.to_string()))
        .collect()
}

// --- names ------------------------------------------------------------------------

/// v2's `BRAIN_NAME_PATTERN.match`: `^[a-z][a-z0-9-]*$`, where Python's `$`
/// also matches before a final `\n`.
fn matches_v2_pattern(name: &str) -> bool {
    let name = name.strip_suffix('\n').unwrap_or(name);
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// v2's `validate_brain_name`: non-empty, not `all`, at most 48 characters,
/// no `..`, no leading `.`, and v2's pattern. The rule for names that already
/// exist: registered brains and `register`'s identity.
pub fn is_valid_v2_name(name: &str) -> bool {
    !name.is_empty()
        && name != RESERVED_NAME
        && name.chars().count() <= NAME_MAX_LENGTH
        && !name.contains("..")
        && !name.starts_with('.')
        && matches_v2_pattern(name)
}

/// The rule for a brain `create` makes: `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`, at
/// most 48 characters, not `all`. v2 also took `--` and a final `-`, which
/// VS Code and OpenCode drop as skill names (research-04).
pub fn is_valid_new_name(name: &str) -> bool {
    name != RESERVED_NAME
        && name.chars().count() <= NAME_MAX_LENGTH
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// Why `create` refuses `name`, in v2's order and words; `None` when it is
/// a valid new name.
pub fn new_name_error(name: &str) -> Option<String> {
    if name == RESERVED_NAME {
        return Some(format!(
            "'{name}' is a reserved brain name (used by --brain {name}). Pick a different name."
        ));
    }
    let length = name.chars().count();
    if length > NAME_MAX_LENGTH {
        return Some(format!(
            "Brain name '{name}' is too long ({length} chars). Maximum is {NAME_MAX_LENGTH}."
        ));
    }
    if !is_valid_v2_name(name) {
        return Some(format!(
            "Brain name '{name}' is invalid. Use lowercase letters, numbers, and hyphens only."
        ));
    }
    if !is_valid_new_name(name) {
        return Some(format!(
            "Brain name '{name}' is invalid. Use lowercase letters and numbers, with single \
             hyphens between them (no '--', no '-' at the end)."
        ));
    }
    None
}

// --- --from-config --------------------------------------------------------------------

/// Python's truth value of a loaded YAML value.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(i) => i.sign() != num_bigint::Sign::NoSign,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Seq(items) => !items.is_empty(),
        Value::Map(map) => !map.is_empty(),
        Value::Date(_) | Value::DateTime(_) => true,
    }
}

/// A lobe name that is one plain directory name: not empty, `.` or `..`, no
/// `/` or `\` (and on Windows no `:`), no NUL.
fn is_plain_lobe_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !(cfg!(windows) && name.contains(':'))
}

/// The lobes a `--from-config` file gives (`file` names it in messages):
/// `None` when it gives none, so the brain type's lobes are used. As v2: a
/// document that is empty or false-like and a mapping without `structure`
/// give none; each lobe's description is Python's `str()` of its value.
/// Refused (v2 writes outside the brain, a Python repr, or raises): a lobe
/// name that is not one plain directory name, a mapping or list value, a
/// `structure` that is not a mapping, a document that is not a mapping.
pub fn structure_from_config(text: &str, file: &str) -> Result<Option<Vec<Lobe>>, String> {
    let root = yaml::load(text).map_err(|e| format!("--from-config: cannot read {file}: {e}"))?;
    if !is_truthy(&root) {
        return Ok(None);
    }
    let Value::Map(root) = &root else {
        return Err(format!(
            "--from-config: {file} is not a mapping (expected `structure: {{lobe: description}}`)."
        ));
    };
    let Some(structure) = root.get("structure") else {
        return Ok(None);
    };
    let Value::Map(structure) = structure else {
        return Err(format!(
            "--from-config: `structure` in {file} must be a mapping of lobe names to descriptions."
        ));
    };
    let mut lobes = Vec::new();
    for (name, value) in structure {
        if !is_plain_lobe_name(name) {
            return Err(format!(
                "--from-config: lobe name {} is not a plain directory name \
                 (no '..', path separators or absolute paths).",
                py_repr_str(name)
            ));
        }
        let description = match value {
            Value::Map(_) | Value::Seq(_) => {
                return Err(format!(
                    "--from-config: lobe {} has a nested value; give each lobe a one-line \
                     description (`structure: {{lobe: description}}`).",
                    py_repr_str(name)
                ));
            }
            other => py_str(other),
        };
        lobes.push((name.clone(), description));
    }
    Ok(Some(lobes))
}

// --- the scaffold --------------------------------------------------------------------

const README: &str = include_str!("../templates/brain_readme.md");

/// v2's `GITIGNORE_CONTENT`.
pub const GITIGNORE: &str = include_str!("../templates/brain_gitignore");

/// A lobe's first `map.md` (dream rewrites it right after).
fn lobe_map(name: &str, description: &str, today: &str) -> String {
    format!(
        "---\nauto_generated: true\nparent: ../brain.md\ndescription: {description}\n\
         updated: {today}\n---\n# {}\n\n{description}\n",
        pycase::title(&name.replace('-', " "))
    )
}

fn brain_md(name: &str, description: &str, lobes: &[Lobe], today: &str) -> String {
    let links: Vec<String> = lobes
        .iter()
        .map(|(lobe, desc)| format!("- [{lobe}/](./{lobe}/map.md) — {desc}"))
        .collect();
    format!(
        "---\nauto_generated: true\nupdated: {today}\n---\n# {name}\n\n{description}\n\n\
         ## Lobes\n\n{}\n\n## Reference\n\n- [glossary.md](./glossary.md) — Domain-specific terms\n",
        links.join("\n")
    )
}

fn glossary_md(today: &str) -> String {
    format!(
        "---\nauto_generated: false\nupdated: {today}\n---\n# Glossary\n\n\
         Project-specific terms, acronyms, and conventions.\n\n| Term | Meaning |\n|------|---------|\n"
    )
}

/// v2's `_generate_readme`, filled in one pass.
pub fn readme(name: &str, description: &str) -> String {
    let mut out = String::with_capacity(README.len());
    let mut rest = README;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("{name}") {
            out.push_str(name);
            rest = after;
        } else if let Some(after) = tail.strip_prefix("{description}") {
            out.push_str(description);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// A scaffold write that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldError {
    pub path: PathBuf,
    pub reason: String,
}

impl fmt::Display for ScaffoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cannot write {}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for ScaffoldError {}

/// Write a new brain at `root` (created with its parents when missing; an
/// existing empty directory is used): the lobe directories, then as one
/// change set of new files (a file already there is an error and is left
/// alone) each lobe's `map.md`, `kluris.yml` (`config`), `brain.md`,
/// `glossary.md`, `README.md` and `.gitignore`, in v2's order.
pub fn scaffold_brain(
    root: &Path,
    config: &BrainConfig,
    lobes: &[Lobe],
    today: Date,
) -> Result<(), ScaffoldError> {
    let failed = |path: &Path, e: std::io::Error| ScaffoldError {
        path: path.to_path_buf(),
        reason: e.to_string(),
    };
    fs::create_dir_all(root).map_err(|e| failed(root, e))?;
    for (lobe, _) in lobes {
        let dir = root.join(lobe);
        match fs::create_dir(&dir) {
            Err(e) if !dir.is_dir() => return Err(failed(&dir, e)),
            _ => {}
        }
    }
    let today = date_iso(&today);
    let mut writes: Vec<FileWrite> = lobes
        .iter()
        .map(|(lobe, desc)| {
            FileWrite::create_new(format!("{lobe}/map.md"), lobe_map(lobe, desc, &today))
        })
        .collect();
    let mut file = |rel: &str, text: String| {
        writes.push(FileWrite::create_new(rel, text));
    };
    file("kluris.yml", brain_config_to_yaml(config));
    file(
        "brain.md",
        brain_md(&config.name, &config.description, lobes, &today),
    );
    file("glossary.md", glossary_md(&today));
    file("README.md", readme(&config.name, &config.description));
    file(".gitignore", GITIGNORE.to_string());
    let applied = LocalGitStore::open(root).apply(&ChangeSet {
        writes,
        ..ChangeSet::default()
    });
    match applied.failed.into_iter().next() {
        Some(write) => Err(ScaffoldError {
            path: under(root, &write.rel),
            reason: write.error,
        }),
        None => Ok(()),
    }
}

// --- register's identity --------------------------------------------------------------

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// The text v2's `_read_brain_identity` scans: `brain.md`'s body, or its whole
/// text when the frontmatter does not read; `None` when it cannot be read.
fn identity_text(brain_md: &Path) -> Option<String> {
    match frontmatter::read(brain_md) {
        Ok(fm) => Some(fm.body),
        Err(_) => fs::read_to_string(brain_md).ok(),
    }
}

/// v2's `_read_brain_identity`: the brain's name and description from
/// `brain.md` (its first `# ` heading when that is a valid v2 name, else
/// `fallback_name`; the first non-empty line after it that is not a `## `
/// heading, else `<fallback_name> knowledge base`).
pub fn read_brain_identity(brain_root: &Path, fallback_name: &str) -> (String, String) {
    let default_description = format!("{fallback_name} knowledge base");
    let brain_md = brain_root.join("brain.md");
    if !brain_md.exists() {
        return (fallback_name.to_string(), default_description);
    }
    let Some(content) = identity_text(&brain_md) else {
        return (fallback_name.to_string(), default_description);
    };
    let mut title = String::new();
    let mut description = String::new();
    let mut title_seen = false;
    for raw in py_splitlines(&content) {
        let line = py_strip(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ")
            && title.is_empty()
        {
            title = py_strip(rest).to_string();
            title_seen = true;
            continue;
        }
        if title_seen && !line.starts_with("## ") {
            description = line.to_string();
            break;
        }
    }
    let name = if is_valid_v2_name(&title) {
        title
    } else {
        fallback_name.to_string()
    };
    if description.is_empty() {
        description = default_description;
    }
    (name, description)
}

/// v2's fallback name for a brain directory: its name, lowercased with
/// spaces as `-` when it is not a valid name.
pub fn fallback_name(dir_name: &str) -> String {
    if is_valid_v2_name(dir_name) {
        dir_name.to_string()
    } else {
        pycase::lower(dir_name).replace(' ', "-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scaffold only creates files: one already in the target is an
    /// error and keeps its bytes (review T-103 finding 1).
    #[test]
    fn the_scaffold_never_overwrites_a_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("demo");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("README.md"), "my own readme\n").unwrap();
        let config = BrainConfig::new("demo");
        let today = Date::new(2026, 10, 26).unwrap();
        let err = scaffold_brain(&root, &config, &[], today).unwrap_err();
        assert_eq!(err.path, root.join("README.md"));
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "my own readme\n"
        );
    }

    #[test]
    fn v2_rule() {
        for ok in ["a", "a-b", "a--b", "a-", "abc\n", &"a".repeat(48)] {
            assert!(is_valid_v2_name(ok), "{ok:?}");
        }
        for bad in [
            "",
            "all",
            "A",
            "9a",
            ".a",
            "a..b",
            "a b",
            "abc\n\n",
            &"a".repeat(49),
        ] {
            assert!(!is_valid_v2_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn new_rule_is_the_strict_pattern() {
        for ok in ["a", "a-b", "a1-2b-c3", &"a".repeat(48)] {
            assert!(is_valid_new_name(ok), "{ok:?}");
        }
        for bad in [
            "",
            "all",
            "a--b",
            "a-",
            "-a",
            "abc\n",
            "a_b",
            "A",
            "9a",
            &"a".repeat(49),
        ] {
            assert!(!is_valid_new_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn short_names_agree_with_the_regex() {
        // Every name of up to 4 characters over a small alphabet.
        let alphabet = ['a', 'z', '0', '9', '-', '.', 'A', '_', '\n'];
        let mut names = vec![String::new()];
        for _ in 0..4 {
            let longer: Vec<String> = names
                .iter()
                .flat_map(|n| alphabet.iter().map(move |c| format!("{n}{c}")))
                .collect();
            names.extend(longer.into_iter().filter(|n| n.len() <= 4));
            names.sort();
            names.dedup();
        }
        for name in &names {
            let strict = !name.is_empty()
                && name.split('-').all(|p| {
                    !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                })
                && name.starts_with(|c: char| c.is_ascii_lowercase());
            assert_eq!(is_valid_new_name(name), strict, "{name:?}");
            assert!(
                !is_valid_new_name(name) || is_valid_v2_name(name),
                "{name:?}"
            );
            assert_eq!(new_name_error(name).is_none(), strict, "{name:?}");
        }
    }

    #[test]
    fn unknown_type_is_blank() {
        assert!(type_lobes("nope").is_empty());
        assert_eq!(type_lobes("product").len(), 6);
    }

    #[test]
    fn readme_fills_in_one_pass() {
        let text = readme("{description}", "{name}");
        assert!(text.starts_with("# {description}\n\n> {name}\n"));
    }

    #[test]
    fn lobe_map_title() {
        assert!(lobe_map("my-lobe", "d", "2026-01-01").contains("\n# My Lobe\n\nd\n"));
    }
}
