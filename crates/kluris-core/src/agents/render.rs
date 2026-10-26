//! Each brain's SKILL.md, rendered as v2.28.11's `agents.render_skill` does
//! with the decided changes (`parity/allowlist.md`): the `{brain, file,
//! target}` braces no longer leak, values are substituted in one pass, a
//! marker comment follows the frontmatter, and the search advice is
//! research-05's "Search" paragraph.
//!
//! Also here: recognising the skills and Windsurf workflows Kluris wrote
//! (v3's marker, or the shape of every v1/v2 rendering), so installs remove
//! only those and never a user's own `kluris-*` folder.
//!
//! `windows_splitroot` below is translated from CPython 3.11
//! `Lib/pathlib.py` (`_WindowsFlavour.splitroot`); see LICENSE-PSF.

use crate::companions;

/// The SKILL.md template: frontmatter, marker and v2's `SKILL_BODY` with the
/// decided changes. Placeholders are `{name}` (see [`fill`]).
const SKILL_TEMPLATE: &str = include_str!("../../templates/skill.md");
/// v2's `SKILL_DESCRIPTION` (one line; its `\` continuations joined).
const SKILL_DESCRIPTION: &str = include_str!("../../templates/skill_description.txt");
/// v2's `_FLAG_HINT_BLOCK` (it starts with three newlines).
const FLAG_HINT: &str = include_str!("../../templates/flag_hint.md");
/// v2's `_build_specmint_block` text around the routing lines.
const COMPANION_BLOCK: &str = include_str!("../../templates/companion_block.md");

/// The marker comment v3 writes right after the frontmatter. Only the
/// prefix up to [`MARKER_PREFIX`] is matched, so a later wording change
/// still recognises older v3 skills.
pub const MARKER: &str = "<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->";
const MARKER_PREFIX: &str = "<!-- kluris:generated";

/// The opening words of every description Kluris has written (v1.0 on).
const V2_DESCRIPTION_START: &str = "You are the team's subject matter expert";

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

/// Whether `skill_md`, the SKILL.md of the folder `dir_name`, was written by
/// Kluris: its frontmatter opens with `name: <dir_name>` and `description:`,
/// and either v3's marker is the first line after the frontmatter or the
/// description is Kluris's (every v1/v2 rendering opens with "You are the
/// team's subject matter expert"). A user's own `kluris-notes` skill is
/// neither.
pub fn is_generated_skill(dir_name: &str, skill_md: &str) -> bool {
    let text = lf(skill_md);
    let head = format!("---\nname: {dir_name}\ndescription: ");
    let Some(description) = text.strip_prefix(&head) else {
        return false;
    };
    if is_kluris_description(description) {
        return true;
    }
    match description.find("\n---\n") {
        Some(end) => description[end + "\n---\n".len()..]
            .trim_start_matches('\n')
            .starts_with(MARKER_PREFIX),
        None => false,
    }
}

/// Whether `text`, the file `file_name` in a Windsurf `global_workflows`
/// folder, is a workflow v2 wrote: `kluris*.md` opening with
/// `description:` and Kluris's description.
pub fn is_generated_workflow(file_name: &str, text: &str) -> bool {
    if !file_name.starts_with("kluris") || !file_name.ends_with(".md") {
        return false;
    }
    let text = lf(text);
    text.strip_prefix("---\ndescription: ")
        .is_some_and(is_kluris_description)
}

/// A frontmatter description value (quoted or not) Kluris wrote.
fn is_kluris_description(value: &str) -> bool {
    value
        .strip_prefix('"')
        .unwrap_or(value)
        .starts_with(V2_DESCRIPTION_START)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_leaves_unknown_and_unclosed_braces() {
        let out = fill("{a} {b} {{a}} {a", &[("a", "{b}"), ("b", "B")]);
        assert_eq!(out, "{b} B {{b}} {a");
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
