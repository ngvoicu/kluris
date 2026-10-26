//! Steps 5 and 6 of dream: every `map.md` and `brain.md` (v2 `core/maps.py`).
//!
//! As v2, except for the directories the walker prunes (dot directories,
//! `node_modules`, `__pycache__` and the rest of `SKIP_DIRS`): they get no
//! `map.md` and are never listed as a sub-lobe or sibling (interview-01 #4,
//! PAR-036). A lobe's neurons are its directory listing, as in v2 (a suffix
//! compared case-insensitively, links followed), not the neuron index.

use super::workspace::{Read, Workspace};
use crate::brain::store::{BrainStore, DirEntry};
use crate::brain::walk::is_pruned_dir;
use crate::frontmatter::{has_yaml_opt_in_head, py_splitlines, universal_newlines};
use crate::pycase;
use crate::yaml::{Date, Map, Value, date_iso, is_py_space};

/// The files a map never lists (v2 maps' `SKIP_FILES`).
const SKIP_FILES: [&str; 7] = [
    "map.md",
    "brain.md",
    "index.md",
    "glossary.md",
    "README.md",
    ".gitignore",
    "kluris.yml",
];

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// Python's `PurePath.suffix` and `.stem`.
fn split_suffix(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_dir(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// `name.replace('-', ' ').title()`.
fn title_of(name: &str) -> String {
    pycase::title(&name.replace('-', " "))
}

/// The map description (`_read_map_description`): the frontmatter
/// `description` when it is a non-empty string, else the first body line
/// after the `# ` heading that is not an `up`, `sideways`, `## ` or `- [`
/// line.
fn map_description<S: BrainStore>(ws: &Workspace<'_, S>, rel: &str) -> String {
    if !ws.exists(rel) {
        return String::new();
    }
    let content = match ws.read(rel, Read::Lenient) {
        Ok(fm) => {
            if let Some(Value::Str(text)) = fm.meta.get("description") {
                let text = py_strip(text);
                if !text.is_empty() {
                    return text.to_string();
                }
            }
            fm.body
        }
        Err(_) => match ws.text(rel) {
            Some(text) => text,
            None => return String::new(),
        },
    };
    let mut title_seen = false;
    for line in py_splitlines(&content) {
        let line = py_strip(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with("# ") {
            title_seen = true;
            continue;
        }
        if !title_seen {
            continue;
        }
        if ["up ", "sideways ", "## ", "- ["]
            .iter()
            .any(|p| line.starts_with(p))
        {
            continue;
        }
        return line.to_string();
    }
    String::new()
}

/// A listed neuron's title (`_get_neurons`): a markdown file's first `# `
/// line, a YAML neuron's `title`, else the stem in title case.
fn neuron_title<S: BrainStore>(ws: &Workspace<'_, S>, rel: &str, yaml: bool) -> String {
    let fallback = title_of(split_suffix(base_name(rel)).0);
    let Ok(fm) = ws.read(rel, Read::Lenient) else {
        return fallback;
    };
    if yaml {
        return match fm.meta.get("title") {
            Some(Value::Str(title)) if !py_strip(title).is_empty() => py_strip(title).to_string(),
            _ => fallback,
        };
    }
    // python-frontmatter hands over the stripped body.
    let body = universal_newlines(&fm.body);
    py_strip(&body)
        .split('\n')
        .find_map(|line| line.strip_prefix("# "))
        .map_or(fallback, |title| py_strip(title).to_string())
}

fn is_opted_in<S: BrainStore>(ws: &Workspace<'_, S>, rel: &str) -> bool {
    ws.text(rel)
        .is_some_and(|text| has_yaml_opt_in_head(text.as_bytes()))
}

/// The child directories listed in maps: not pruned, with a `map.md`.
fn lobe_dirs<'e, S: BrainStore>(
    ws: &Workspace<'_, S>,
    dir: &str,
    entries: &'e [DirEntry],
) -> impl Iterator<Item = &'e str> {
    entries
        .iter()
        .filter(move |e| {
            e.is_dir && !is_pruned_dir(&e.name) && ws.exists(&join(&join(dir, &e.name), "map.md"))
        })
        .map(|e| e.name.as_str())
}

/// Write `content` to `rel` with `meta` unless the body there is already
/// the same (`_write_if_changed`); `updated` is then today.
fn write_if_changed<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    rel: &str,
    mut meta: Map,
    content: &str,
    today: Date,
) {
    let old = if ws.exists(rel) {
        ws.read(rel, Read::Lenient).ok()
    } else {
        None
    };
    if let Some(fm) = &old
        && py_strip(&universal_newlines(&fm.body)) == py_strip(content)
    {
        return;
    }
    // A file that exists but does not read keeps its line endings too.
    let endings = match &old {
        Some(fm) => Some(fm.clone()),
        None if ws.exists(rel) => ws.text(rel).map(|text| crate::frontmatter::Frontmatter {
            meta: Map::new(),
            body: String::new(),
            line_ending: crate::frontmatter::line_ending(&text),
            had_bom: text.starts_with('\u{feff}'),
            body_line: 0,
        }),
        None => None,
    };
    meta.insert("updated".into(), Value::Str(date_iso(&today)));
    ws.write(rel, endings.as_ref(), &meta, content);
}

/// `generate_map_md` for one directory.
fn generate_map<S: BrainStore>(ws: &mut Workspace<'_, S>, dir: &str, today: Date) {
    let entries = ws.store().entries(dir);
    let mut neurons = Vec::new();
    for entry in entries.iter().filter(|e| e.is_file) {
        if SKIP_FILES.contains(&entry.name.as_str()) {
            continue;
        }
        let suffix = split_suffix(&entry.name).1.to_lowercase();
        let yaml = suffix == ".yml" || suffix == ".yaml";
        if suffix != ".md" && !yaml {
            continue;
        }
        let rel = join(dir, &entry.name);
        if yaml && !is_opted_in(ws, &rel) {
            continue;
        }
        neurons.push((entry.name.clone(), neuron_title(ws, &rel, yaml)));
    }
    let sub_lobes: Vec<&str> = lobe_dirs(ws, dir, &entries).collect();

    let name = base_name(dir);
    let up = parent_dir(dir);
    let up_entries = ws.store().entries(up);
    let siblings: Vec<&str> = lobe_dirs(ws, up, &up_entries)
        .filter(|sibling| *sibling != name)
        .collect();
    let description = map_description(ws, &join(dir, "map.md"));
    let (parent_path, parent_name) = if up.is_empty() {
        ("../brain.md", "brain.md")
    } else {
        ("../map.md", base_name(up))
    };

    let mut contents: Vec<String> = sub_lobes
        .iter()
        .map(|sub| format!("- [{sub}/](./{sub}/map.md)"))
        .collect();
    contents.extend(
        neurons
            .iter()
            .map(|(file, title)| format!("- [{file}](./{file}) — {title}")),
    );
    let contents = if contents.is_empty() {
        "(empty)".to_string()
    } else {
        contents.join("\n")
    };
    let mut content = format!("# {}\n\n", title_of(name));
    if !description.is_empty() {
        content.push_str(&format!("{description}\n\n"));
    }
    content.push_str(&format!("up [{parent_name}]({parent_path})\n"));
    if !siblings.is_empty() {
        let links: Vec<String> = siblings
            .iter()
            .map(|s| format!("[{s}](../{s}/map.md)"))
            .collect();
        content.push_str(&format!("sideways {}\n", links.join(" | ")));
    }
    content.push_str(&format!("\n## Contents\n\n{contents}\n"));

    let mut meta = Map::new();
    meta.insert("auto_generated".into(), Value::Bool(true));
    meta.insert("parent".into(), Value::Str(parent_path.into()));
    meta.insert(
        "siblings".into(),
        Value::Seq(
            siblings
                .iter()
                .map(|s| Value::Str(format!("../{s}/map.md")))
                .collect(),
        ),
    );
    if !description.is_empty() {
        meta.insert("description".into(), Value::Str(description));
    }
    write_if_changed(ws, &join(dir, "map.md"), meta, &content, today);
}

/// The directories in v2's map order: deepest first, then by path text
/// (native separators, as v2 compares `str(path)`).
pub(crate) fn map_order(dirs: &[String]) -> Vec<String> {
    let mut dirs = dirs.to_vec();
    let native = |rel: &str| rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    dirs.sort_by_cached_key(|rel| (std::cmp::Reverse(rel.split('/').count()), native(rel)));
    dirs
}

/// Both passes of `generate_map_md` over `dirs` (in [`map_order`]).
pub(crate) fn generate_maps<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    dirs: &[String],
    today: Date,
) {
    for _pass in 0..2 {
        for dir in dirs {
            generate_map(ws, dir, today);
        }
    }
}

/// `generate_brain_md`: the root lobes with their map descriptions and the
/// glossary link.
pub(crate) fn generate_brain_md<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    name: &str,
    description: &str,
    today: Date,
) {
    let links: Vec<String> = ws
        .store()
        .lobe_names()
        .iter()
        .map(|lobe| {
            let description = map_description(ws, &join(lobe, "map.md"));
            format!("- [{lobe}/](./{lobe}/map.md) — {description}")
        })
        .collect();
    let content = format!(
        "# {name}\n\n{description}\n\n## Lobes\n\n{}\n\n## Reference\n\n- [glossary.md](./glossary.md) — Domain-specific terms, acronyms, and conventions\n",
        links.join("\n")
    );
    let mut meta = Map::new();
    meta.insert("auto_generated".into(), Value::Bool(true));
    write_if_changed(ws, "brain.md", meta, &content, today);
}
