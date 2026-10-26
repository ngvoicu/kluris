//! Each brain's SKILL.md, rendered as v2.28.11's `agents.render_skill` does
//! with the decided changes (`parity/allowlist.md`): the `{brain, file,
//! target}` braces no longer leak, values are substituted in one pass, a
//! marker comment follows the frontmatter, and the search advice is
//! research-05's "Search" paragraph.
//!
//! Also here: recognising the skills and Windsurf workflows Kluris wrote
//! (v3's marker at its place, or the full template of a v1/v2 release,
//! [`super::legacy`]), so installs remove only those and never a user's own
//! `kluris-*` folder.
//!
//! `windows_splitroot` below is translated from CPython 3.11
//! `Lib/pathlib.py` (`_WindowsFlavour.splitroot`); see LICENSE-PSF.

use super::legacy::{ERAS, Era, Style, Workflow};
use crate::companions;
use crate::scaffold::is_valid_v2_name;

/// The SKILL.md template: frontmatter, marker and v2's `SKILL_BODY` with the
/// decided changes. Placeholders are `{name}` (see [`fill`]).
const SKILL_TEMPLATE: &str = include_str!("../../templates/skill.md");
/// v2's `SKILL_DESCRIPTION` (one line; its `\` continuations joined).
const SKILL_DESCRIPTION: &str = include_str!("../../templates/skill_description.txt");
/// v2's `_FLAG_HINT_BLOCK` (it starts with three newlines).
const FLAG_HINT: &str = include_str!("../../templates/flag_hint.md");
/// v2's `_build_specmint_block` text around the routing lines.
const COMPANION_BLOCK: &str = include_str!("../../templates/companion_block.md");

/// The marker comment v3 writes right after the frontmatter; a SKILL.md is
/// v3's when this whole line is in that place.
pub const MARKER: &str = "<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->";

/// What a brain's skill is rendered from.
#[derive(Debug, Clone, Copy)]
pub struct SkillInput<'a> {
    pub brain_name: &'a str,
    /// The brain path as stored in config.yml; written in POSIX form.
    pub brain_path: &'a str,
    /// `<brain>/.git` exists.
    pub has_git: bool,
    /// The config entry's description; empty means `<name> knowledge base`.
    pub description: &'a str,
    /// The brain's companions as listed; unknown names are ignored.
    pub companions: &'a [String],
    /// `~/.kluris/companions`; written in POSIX form.
    pub companion_home: &'a str,
}

/// The skill (and folder) name of a brain: `kluris-<name>`.
pub fn skill_name(brain_name: &str) -> String {
    format!("kluris-{brain_name}")
}

/// The SKILL.md of one brain. Every agent folder gets these same bytes.
pub fn render_skill(input: &SkillInput) -> String {
    let name = input.brain_name;
    let skill = skill_name(name);
    let description = if input.description.is_empty() {
        format!("{name} knowledge base")
    } else {
        input.description.to_string()
    };
    let flag_hint = fill(FLAG_HINT, &[("brain_name", name), ("skill_name", &skill)]);
    let flag_inline = format!(" --brain {name}");
    let skill_description = fill(SKILL_DESCRIPTION, &[("brain_name", name)]).replace('"', "\\\"");
    let companion_block = companion_block(input.companions, input.companion_home);
    let brain_path = posix_path(input.brain_path);
    fill(
        SKILL_TEMPLATE,
        &[
            ("skill_name", &skill),
            ("skill_description", &skill_description),
            ("brain_name", name),
            ("brain_path", &brain_path),
            ("git_label", if input.has_git { "git" } else { "no git" }),
            ("brain_description", &description),
            ("brain_flag_hint", &flag_hint),
            ("brain_flag_hint_inline", &flag_inline),
            ("specmint_block", &companion_block),
        ],
    )
}

/// Replace each `{key}` of `template` that names one of `values`, in one
/// pass: text that comes from a value is never looked at again, and any
/// other `{...}` stays as written.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 4096);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let found = after.find('}').and_then(|close| {
            let key = &after[..close];
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v, close))
        });
        match found {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What each companion is for, in the routing lines (v2 `_COMPANION_LABELS`).
fn companion_label(name: &str) -> &'static str {
    match name {
        "specmint-core" => "general spec-driven work with markdown specs",
        "specmint-tdd" => "TDD-heavy work with markdown specs",
        "specmint-core-html" => "general spec-driven work with canonical SPEC.html files",
        _ => "TDD-heavy spec-driven work with canonical SPEC.html files",
    }
}

/// The "Spec-worthy work first" block for the brain's known companions, in
/// canonical order; empty without one (v2 `_build_specmint_block`).
fn companion_block(names: &[String], companion_home: &str) -> String {
    let selected = companions::normalize(names);
    let Some(first) = selected.first() else {
        return String::new();
    };
    let home = posix_path(companion_home);
    let mut routing = selected
        .iter()
        .map(|n| format!("- {} -> read `{home}/{n}/SKILL.md`", companion_label(n)))
        .collect::<Vec<_>>()
        .join("\n");
    if !selected
        .iter()
        .any(|n| n == "specmint-tdd" || n == "specmint-tdd-html")
    {
        routing.push_str(
            "\n- If the user explicitly wants strict TDD, suggest adding \
             `specmint-tdd` or `specmint-tdd-html` for this brain.",
        );
    }
    let first_dir = format!("{home}/{first}");
    fill(
        COMPANION_BLOCK,
        &[("routing", &routing), ("first_companion_dir", &first_dir)],
    )
}

// --- paths --------------------------------------------------------------------

/// `p` in POSIX form, as v2's `agents._posix_path` writes brain and companion
/// paths into SKILL.md: a path with a backslash or a second character `:` is
/// read as Python 3.11's `PureWindowsPath` (`C:\Users\x` becomes
/// `C:/Users/x`), any other as `PurePosixPath`; both drop empty and `.`
/// parts and a trailing separator, and never resolve `..`.
pub fn posix_path(p: &str) -> String {
    if p.contains('\\') || p.chars().nth(1) == Some(':') {
        windows_as_posix(p)
    } else {
        posix_as_posix(p)
    }
}

/// `str(PurePosixPath(p))`: two leading slashes are kept, three or more
/// read as one.
fn posix_as_posix(p: &str) -> String {
    let rel = p.trim_start_matches('/');
    let root = match p.len() - rel.len() {
        0 => "",
        2 => "//",
        _ => "/",
    };
    let parts: Vec<&str> = rel
        .split('/')
        .filter(|x| !x.is_empty() && *x != ".")
        .collect();
    let out = format!("{root}{}", parts.join("/"));
    if out.is_empty() { ".".to_string() } else { out }
}

/// `PureWindowsPath(p).as_posix()`.
fn windows_as_posix(p: &str) -> String {
    let part = p.replace('/', "\\");
    let (drive, root, rel) = windows_splitroot(&part);
    let parts: Vec<&str> = rel
        .split('\\')
        .filter(|x| !x.is_empty() && *x != ".")
        .collect();
    let out = format!("{drive}{root}{}", parts.join("\\"));
    if out.is_empty() {
        ".".to_string()
    } else {
        out.replace('\\', "/")
    }
}

/// Python 3.11's `_WindowsFlavour.splitroot` (separators already `\`):
/// `(drive, root, rest)`. The drive is `X:`, a UNC `\\server\share`, or
/// either behind an extended `\\?\` prefix.
fn windows_splitroot(part: &str) -> (String, &'static str, String) {
    let sep = b'\\';
    let mut part = part.to_string();
    let mut prefix = String::new();
    if let Some(s) = part.strip_prefix("\\\\?\\") {
        prefix.push_str("\\\\?\\");
        let mut s = s.to_string();
        if let Some(unc) = s.strip_prefix("UNC\\") {
            prefix.push_str("UNC");
            s = format!("\\\\{unc}");
        }
        part = s;
    }
    let b = part.as_bytes();
    let (first, second, third) = (b.first().copied(), b.get(1).copied(), b.get(2).copied());
    // A UNC drive `\\server\share` (no second `\` straight after the server).
    if first == Some(sep)
        && second == Some(sep)
        && third != Some(sep)
        && let Some(index) = find_from(&part, 2)
        && find_from(&part, index + 1) != Some(index + 1)
    {
        let end = find_from(&part, index + 1).unwrap_or(part.len());
        let drive = if prefix.is_empty() {
            part[..end].to_string()
        } else {
            format!("{prefix}{}", &part[1..end])
        };
        let rest = part.get(end + 1..).unwrap_or("").to_string();
        return (drive, "\\", rest);
    }
    let mut rest = part.as_str();
    let mut first = first;
    if second == Some(b':') && first.is_some_and(|c| c.is_ascii_alphabetic()) {
        prefix.push_str(&rest[..2]);
        rest = &rest[2..];
        first = third;
    }
    if first == Some(sep) {
        return (prefix, "\\", rest.trim_start_matches('\\').to_string());
    }
    (prefix, "", rest.to_string())
}

/// The byte index of the first `\` in `s` at or after `from`.
fn find_from(s: &str, from: usize) -> Option<usize> {
    s.get(from..)?.find('\\').map(|i| i + from)
}

// --- recognising Kluris-written files -----------------------------------------

/// The text with CRLF read as LF.
fn lf(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        text.replace("\r\n", "\n").into()
    } else {
        text.into()
    }
}

/// How every v1/v2 description opens.
pub(super) const DESCRIPTION_OPENINGS: [&str; 2] = [
    "You are the team's subject matter expert",
    "Your team's shared AI brain",
];

/// How v3's marker opens.
const MARKER_OPENING: &str = "<!-- kluris:generated";

/// Whether `text` (a SKILL.md or workflow) carries a trait of Kluris's:
/// the marker's opening or a v1/v2 description opening, anywhere. A file
/// with one that [`is_generated_skill`] does not recognise is a near miss.
pub fn has_kluris_trait(text: &str) -> bool {
    text.contains(MARKER_OPENING) || DESCRIPTION_OPENINGS.iter().any(|o| text.contains(o))
}

/// Whether `skill_md`, the SKILL.md of the folder `dir_name` (its name as
/// the folder listing gives it), was written by Kluris:
///
/// - v3: the frontmatter is `name: <dir_name>` and a one-line
///   `description:`, then a blank line and [`MARKER`] as a line of its own;
/// - v1/v2: the frontmatter is a release's own, byte for byte, with only
///   the brain name varying (`kluris-<brain>`, or v1/v2.0's bare `kluris`),
///   and the body has that release's headings in order.
///
/// Anything else, a user's `kluris-notes` whose description merely opens
/// like Kluris's included, is not. CRLF reads as LF.
pub fn is_generated_skill(dir_name: &str, skill_md: &str) -> bool {
    let text = lf(skill_md);
    has_marker(dir_name, &text)
        || ERAS.iter().any(|era| {
            let frontmatter = |desc: &str| skill_frontmatter(era.style, dir_name, desc);
            matches_era(era, dir_name, &text, frontmatter)
        })
}

/// Whether `text`, the file `file_name` in a Windsurf `global_workflows`
/// folder, is a workflow a v1/v2 release wrote (`kluris-<brain>.md` or
/// `kluris.md`): that release's frontmatter byte for byte and its headings
/// in order.
pub fn is_generated_workflow(file_name: &str, text: &str) -> bool {
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

/// v3's frontmatter and marker: `---`, `name: <dir_name>`, a non-empty
/// `description: ...` line, `---`, a blank line, the marker line.
fn has_marker(dir_name: &str, text: &str) -> bool {
    let head = format!("---\nname: {dir_name}\ndescription: ");
    let Some((description, rest)) = text.strip_prefix(&head).and_then(|r| r.split_once('\n'))
    else {
        return false;
    };
    !description.is_empty()
        && rest
            .strip_prefix("---\n\n")
            .and_then(|r| r.strip_prefix(MARKER))
            .is_some_and(|r| r.starts_with('\n'))
}

/// The brain an `era` file named `name` (`kluris-<brain>` or `kluris`)
/// would be for, then whether `text` is that file: `frontmatter(desc)`
/// built from the era's description with that brain, then its headings.
fn matches_era(era: &Era, name: &str, text: &str, frontmatter: impl Fn(&str) -> String) -> bool {
    let slotted = era.description.contains(BRAIN_SLOT);
    let brain = match name.strip_prefix("kluris-") {
        // v2 names a brain's skill `kluris-<brain>`; v1 wrote only `kluris`.
        Some(brain) if slotted => brain.to_string(),
        Some(_) => return false,
        None if name != "kluris" => return false,
        None if !slotted => String::new(),
        // v2.0's single-brain `kluris`: the brain is the one its
        // description names.
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

/// The one placeholder of a v1/v2 description and heading.
const BRAIN_SLOT: &str = "{brain_name}";

/// A v1/v2 SKILL.md's frontmatter and the blank line after it.
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

/// A v1/v2 Windsurf workflow's frontmatter: the description's first 200
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_leaves_unknown_and_unclosed_braces() {
        let out = fill("{a} {b} {{a}} {a", &[("a", "{b}"), ("b", "B")]);
        assert_eq!(out, "{b} B {{b}} {a");
    }

    /// Every v1/v2 description opens with one of the openings a near miss
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

    #[test]
    fn templates_are_lf_and_ascii_where_v2_is() {
        for t in [
            SKILL_TEMPLATE,
            SKILL_DESCRIPTION,
            FLAG_HINT,
            COMPANION_BLOCK,
        ] {
            assert!(!t.contains('\r'));
        }
        assert!(SKILL_TEMPLATE.ends_with("repo.\n\n"));
        assert!(FLAG_HINT.starts_with("\n\n\nWhen invoking"));
        assert!(!SKILL_DESCRIPTION.contains('\n'));
    }
}
