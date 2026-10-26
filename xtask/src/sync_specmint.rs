//! `sync-specmint`: re-vendor the four Specmint companion folders into
//! `vendored/`, a port of v2.28.11's `scripts/sync-specmint.py`.
//!
//! Each companion ships as a folder: `SKILL.md` plus `references/` and
//! `assets/` when the source has them. Everything else in a source repo
//! (.git, README.md, LICENSE, AGENTS.md, CLAUDE.md, evals/, skills/,
//! dotfiles) stays behind. Files are copied verbatim, except `SKILL.md`,
//! which gets the sanitizer rules below. Every rule must match exactly once:
//! a renamed heading or reworded sentence upstream aborts the sync instead
//! of silently shipping plugin text. Two checks always run on the result:
//! the sanitized `SKILL.md` must not mention any of [`SIDE_CAR_PATTERNS`],
//! and every `references/...` or `assets/...` path a shipped markdown file
//! mentions must exist in the folder.
//!
//! All four companions are built and checked in memory first; nothing under
//! `vendored/` is touched unless every one of them passes. `--check` writes
//! nothing and exits 1 when the vendored copy is stale; `--diff` prints the
//! full unified diff of every changed file. Running it twice is a no-op.

mod difflib;

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{fs, mem};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use regex::Regex;
use serde_json::{Value, json};

/// The companions, in v2's order, with their source flags.
pub const NAMES: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];
const FLAGS: [&str; 4] = ["--core", "--tdd", "--core-html", "--tdd-html"];

/// Top-level entries copied from a source repo; nothing else ships.
const ALLOWED_ENTRIES: [&str; 3] = ["SKILL.md", "references", "assets"];

/// Plugin/marketplace text that must not survive in a vendored SKILL.md.
pub const SIDE_CAR_PATTERNS: [&str; 7] = [
    "commands/",
    "agents/",
    ".claude-plugin",
    "plugin.json",
    "npx skills",
    "/plugin marketplace",
    "~/.claude/skills",
];

/// `{posix path: bytes}` of a companion folder.
pub type Tree = BTreeMap<String, Vec<u8>>;

/// `textwrap.fill(text, width, break_long_words=False,
/// break_on_hyphens=False)` for text whose only whitespace is single
/// spaces: greedy lines of at most `width` characters, a longer word alone.
fn fill(text: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let needed = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if !line.is_empty() && needed > width {
            lines.push(mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.join("\n")
}

/// The `## Kluris Companion Compatibility` section that replaces Specmint's
/// `## Cross-Tool Compatibility` (v2 `_compat_section`).
pub fn compat_section(name: &str) -> String {
    let home = format!("~/.kluris/companions/{name}");
    let (scope, format_note) = if name.ends_with("-html") {
        (
            format!(
                "Paths in this file that start with `references/` or `assets/` resolve against \
                 `{home}/`. `.specs/...` paths, and the relative `../assets/` links inside a \
                 `SPEC.html`, are project paths."
            ),
            "`SPEC.html` stays plain HTML with a JSON metadata blob: open it in any browser, \
             edit it in any text editor, and keep it diff-friendly with the conventions in \
             `references/spec-format.md`.",
        )
    } else {
        (
            format!(
                "Paths in this file that start with `references/` resolve against `{home}/`. \
                 `.specs/...` paths are project paths."
            ),
            "Specs remain plain markdown/YAML files that humans can edit and Git can diff.",
        )
    };
    let body = format!(
        "Kluris embeds this playbook as the `{name}` companion. Read and follow it directly \
         from `{home}/SKILL.md`. {scope} Do not ask the user to install specmint separately. \
         {format_note}"
    );
    format!(
        "\n## Kluris Companion Compatibility\n\n{}\n",
        fill(&body, 75)
    )
}

/// v3 `## Cross-Tool Compatibility` (the `npx skills add` install lines per
/// tool) up to `## Behavioral Notes`, replaced by [`compat_section`]:
/// Python's `re.subn(r"\n## Cross-Tool Compatibility\n.*?(?=\n## Behavioral
/// Notes\n)", ..., flags=re.S)`.
fn rule_compat_section(name: &str, content: &str) -> (String, usize) {
    const HEAD: &str = "\n## Cross-Tool Compatibility\n";
    const END: &str = "\n## Behavioral Notes\n";
    let mut out = String::new();
    let mut count = 0;
    let mut pos = 0;
    while let Some(found) = content[pos..].find(HEAD) {
        let start = pos + found;
        let Some(end) = content[start + HEAD.len()..].find(END) else {
            break;
        };
        let end = start + HEAD.len() + end;
        out.push_str(&content[pos..start]);
        out.push_str(&compat_section(name));
        count += 1;
        pos = end;
    }
    out.push_str(&content[pos..]);
    (out, count)
}

/// v3 html Step 1: "... (e.g. `~/.claude/skills/<name>/assets/`)." becomes
/// the Kluris path. Python's `\s` (`str.isspace`) is Rust's `\s` plus
/// `\x1c`-`\x1f`.
fn rule_asset_location(name: &str, content: &str) -> (String, usize) {
    let ws = r"[\s\x1c-\x1f]";
    let pattern = format!(
        r"(?:AI tools{ws}+should know the skill install location; for|For) skill installs it{ws}+is typically the installed{ws}+skill's assets/ directory \(e\.g\.{ws}+`~/\.claude/skills/{}/assets/`\)\.",
        regex::escape(name)
    );
    let Ok(re) = Regex::new(&pattern) else {
        return (content.to_string(), 0);
    };
    let count = re.find_iter(content).count();
    // Both matches sit in a numbered list item, so continuation lines indent 3.
    let replacement =
        format!("Under Kluris that directory is\n   `~/.kluris/companions/{name}/assets/`.");
    let out = re
        .replace_all(content, regex::NoExpand(&replacement))
        .into_owned();
    (out, count)
}

/// A sanitizer rule (v2 `Rule`): its label, whether it applies to a
/// companion, and the rewrite with its match count.
struct Rule {
    label: &'static str,
    applies: fn(&str) -> bool,
    apply: fn(&str, &str) -> (String, usize),
}

const RULES: [Rule; 2] = [
    Rule {
        label: "Cross-Tool Compatibility section",
        applies: |_| true,
        apply: rule_compat_section,
    },
    Rule {
        label: "~/.claude/skills asset location",
        applies: |name| name.ends_with("-html"),
        apply: rule_asset_location,
    },
];

/// Apply every rule for `name`; each must match exactly once.
pub fn sanitize_for_kluris(name: &str, content: &str) -> Result<String, String> {
    let mut content = content.to_string();
    for rule in RULES {
        if !(rule.applies)(name) {
            continue;
        }
        let (out, count) = (rule.apply)(name, &content);
        if count != 1 {
            return Err(format!(
                "{name}: sanitizer rule '{}' matched {count} times (expected 1). \
                 Specmint's SKILL.md changed; review the rule.",
                rule.label
            ));
        }
        content = out;
    }
    Ok(content)
}

/// The [`SIDE_CAR_PATTERNS`] `content` still mentions.
pub fn forbidden_hits(content: &str) -> Vec<&'static str> {
    SIDE_CAR_PATTERNS
        .iter()
        .copied()
        .filter(|p| content.contains(p))
        .collect()
}

/// v2's `_PATH_MENTION.findall(text)`:
/// `(?<![\w./-])((?:references|assets)/[A-Za-z0-9_.*-]+)`. Rust's regex has
/// no look-behind, so a match whose previous character is a word character,
/// `.`, `/` or `-` is dropped and the search goes on one character later.
/// Python's `\w` is approximated by `char::is_alphanumeric` and `_` (they
/// differ only for combining marks and symbol letters).
pub fn path_mentions(text: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r"(?:references|assets)/[A-Za-z0-9_.*-]+") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(m) = re.find_at(text, pos) {
        let before = text[..m.start()].chars().next_back();
        let blocked =
            before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'));
        if blocked {
            // `r` and `a` are one byte.
            pos = m.start() + 1;
        } else {
            found.push(m.as_str().to_string());
            pos = m.end();
        }
    }
    found
}

/// `fnmatch.fnmatchcase(name, pattern)` for patterns whose only wildcard is
/// `*` (the mention regex allows no other).
fn glob_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return false;
    };
    if parts.len() == 1 {
        return name == pattern;
    }
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

/// The `references/`/`assets/` paths mentioned in the tree's markdown files
/// that the tree lacks, as sorted `"<file>: <mention>"` (v2
/// `_missing_mentions`); a markdown file that is not UTF-8 is an error.
pub fn missing_mentions(tree: &Tree) -> Result<Vec<String>, String> {
    let mut missing = std::collections::BTreeSet::new();
    for (rel, data) in tree {
        if !rel.ends_with(".md") {
            continue;
        }
        let text = std::str::from_utf8(data).map_err(|e| format!("{rel}: {e}"))?;
        for mention in path_mentions(text) {
            let mention = mention.trim_end_matches('.');
            if mention.ends_with('/') {
                continue;
            }
            let shipped = if mention.contains('*') {
                tree.keys().any(|k| glob_match(k, mention))
            } else {
                tree.contains_key(mention)
            };
            if !shipped {
                missing.insert(format!("{rel}: {mention}"));
            }
        }
    }
    Ok(missing.into_iter().collect())
}

/// Every path under `dir` (links listed, never followed), sorted as Python
/// sorts `Path`s: by their parts.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort_by(|a, b| a.components().cmp(b.components()));
    out
}

fn posix(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The allowlisted source files of `name` (v2 `_read_source`); prints the
/// top-level entries left behind.
fn read_source(name: &str, source: &Path, out: &mut impl Write) -> Result<Tree, String> {
    let skill = source.join("SKILL.md");
    if !skill.is_file() {
        return Err(format!("{name}: missing SKILL.md at {}", skill.display()));
    }
    let mut tree = Tree::new();
    for entry in ALLOWED_ENTRIES {
        let path = source.join(entry);
        if !path.exists() {
            continue;
        }
        if is_link(&path) {
            return Err(format!("{name}: refusing symlink {}", path.display()));
        }
        if entry == "SKILL.md" {
            tree.insert(entry.to_string(), read(&path)?);
            continue;
        }
        if !path.is_dir() {
            return Err(format!("{name}: {} must be a directory", path.display()));
        }
        for file in walk(&path) {
            let Ok(rel) = file.strip_prefix(source) else {
                continue;
            };
            let hidden = rel.components().any(|c| {
                let part = c.as_os_str().to_string_lossy();
                part.starts_with('.') || part == "__pycache__"
            });
            if hidden {
                continue;
            }
            if is_link(&file) {
                return Err(format!("{name}: refusing symlink {}", file.display()));
            }
            if file.is_file() {
                tree.insert(posix(rel), read(&file)?);
            }
        }
    }
    let mut skipped: Vec<String> = fs::read_dir(source)
        .map_err(|e| format!("{}: {e}", source.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !ALLOWED_ENTRIES.contains(&n.as_str()))
        .collect();
    skipped.sort();
    if !skipped.is_empty() {
        let _ = writeln!(out, "{name}: not shipped: {}", skipped.join(", "));
    }
    Ok(tree)
}

/// `path.expanduser().resolve()`, without Windows' `\\?\` prefix.
fn resolve(path: &Path) -> PathBuf {
    let expanded = match path.to_str().and_then(|s| s.strip_prefix("~")) {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            match std::env::home_dir() {
                Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
                None => path.to_path_buf(),
            }
        }
        _ => path.to_path_buf(),
    };
    let resolved = fs::canonicalize(&expanded).unwrap_or(expanded);
    match resolved.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC\\") => PathBuf::from(plain),
        _ => resolved,
    }
}

/// Build companion `name` from `source` in memory and check it (v2
/// `_build`).
pub fn build(name: &str, source: &Path, out: &mut impl Write) -> Result<Tree, String> {
    let source = resolve(source);
    let mut tree = read_source(name, &source, out)?;
    let skill = tree.get("SKILL.md").cloned().unwrap_or_default();
    let skill = String::from_utf8(skill).map_err(|e| format!("{name}: SKILL.md: {e}"))?;
    let content = sanitize_for_kluris(name, &skill)?;
    let hits = forbidden_hits(&content);
    if !hits.is_empty() {
        return Err(format!(
            "{name}: sanitized SKILL.md still mentions forbidden paths: {}",
            hits.join(", ")
        ));
    }
    tree.insert("SKILL.md".to_string(), content.into_bytes());
    let missing = missing_mentions(&tree)?;
    if !missing.is_empty() {
        return Err(format!(
            "{name}: mentioned files are not shipped: {}",
            missing.join("; ")
        ));
    }
    Ok(tree)
}

/// Every file under `root` (`{}` when it is not a folder).
pub fn read_tree(root: &Path) -> Tree {
    if !root.is_dir() {
        return Tree::new();
    }
    walk(root)
        .into_iter()
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let rel = posix(p.strip_prefix(root).ok()?);
            Some((rel, fs::read(&p).ok()?))
        })
        .collect()
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

/// `len(data.decode("utf-8", errors="replace").splitlines())`.
pub fn line_count(data: &[u8]) -> usize {
    difflib::splitlines(&text(data), false).len()
}

/// Print a per-file diff summary of `old` -> `new` (with `full_diff` the
/// unified diffs too); `true` when anything differs (v2 `_report`).
pub fn report(name: &str, old: &Tree, new: &Tree, full_diff: bool, out: &mut impl Write) -> bool {
    let mut changed = false;
    let paths: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for rel in paths {
        let (before, after) = (old.get(rel), new.get(rel));
        if before == after {
            continue;
        }
        changed = true;
        let (before, after) = match (before, after) {
            (None, Some(after)) => {
                let _ = writeln!(
                    out,
                    "  added    {rel} ({} bytes, {} lines)",
                    after.len(),
                    line_count(after)
                );
                continue;
            }
            (Some(before), None) => {
                let _ = writeln!(
                    out,
                    "  removed  {rel} ({} bytes, {} lines)",
                    before.len(),
                    line_count(before)
                );
                continue;
            }
            (Some(before), Some(after)) => (before, after),
            (None, None) => continue,
        };
        let a = difflib::splitlines(&text(before), true);
        let b = difflib::splitlines(&text(after), true);
        let diff = difflib::unified_diff(
            &a,
            &b,
            &format!("old/{name}/{rel}"),
            &format!("new/{name}/{rel}"),
        );
        let plus = diff
            .iter()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let minus = diff
            .iter()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        let _ = writeln!(
            out,
            "  changed  {rel} (+{plus} -{minus} lines; {} -> {} bytes)",
            before.len(),
            after.len()
        );
        if full_diff {
            for line in &diff {
                let _ = out.write_all(line.as_bytes());
            }
        }
    }
    if !changed {
        let _ = writeln!(out, "  in sync");
    }
    changed
}

fn remove_tree(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

/// Replace `dest_root/name` with `tree` via stage-then-rename (v2 `_write`).
fn write_companion(dest_root: &Path, name: &str, tree: &Tree) -> Result<(), String> {
    let dest = dest_root.join(name);
    let staging = dest_root.join(format!(".{name}.tmp"));
    let backup = dest_root.join(format!(".{name}.old"));
    remove_tree(&staging);
    remove_tree(&backup);
    let result = (|| {
        for (rel, data) in tree {
            let path = staging.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        if read_tree(&staging) != *tree {
            return Err(format!("{name}: staged copy does not match the built tree"));
        }
        if dest.exists() {
            fs::rename(&dest, &backup).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
        fs::rename(&staging, &dest).map_err(|e| format!("{}: {e}", dest.display()))
    })();
    if result.is_err() {
        remove_tree(&staging);
        if backup.exists() && !dest.exists() {
            let _ = fs::rename(&backup, &dest);
        }
    }
    remove_tree(&backup);
    result?;
    if read_tree(&dest) != *tree {
        return Err(format!(
            "{name}: written copy does not match the built tree"
        ));
    }
    Ok(())
}

const USAGE: &str = "usage: cargo xtask sync-specmint [--core PATH] [--tdd PATH] [--core-html PATH] [--tdd-html PATH] [--check] [--diff]";

struct Args {
    sources: [PathBuf; 4],
    check: bool,
    diff: bool,
    dest: PathBuf,
}

fn parse_args(args: &[String], root: &Path) -> Result<Args, String> {
    let parent = root.parent().unwrap_or(root);
    let mut sources = NAMES.map(|name| parent.join("specmint").join(name));
    let (mut check, mut diff) = (false, false);
    let mut dest = root.join("vendored");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| iter.next().cloned())
                .ok_or_else(|| format!("{USAGE}\nerror: argument {flag}: expected one argument"))
        };
        match flag {
            "--check" if inline.is_none() => check = true,
            "--diff" if inline.is_none() => diff = true,
            // Where to write instead of `vendored/` (the tests' temp folders).
            "--dest" => dest = PathBuf::from(value()?),
            _ => match FLAGS.iter().position(|f| *f == flag) {
                Some(i) => sources[i] = PathBuf::from(value()?),
                None => return Err(format!("{USAGE}\nerror: unrecognized arguments: {arg}")),
            },
        }
    }
    Ok(Args {
        sources,
        check,
        diff,
        dest,
    })
}

/// The task, with `root` the workspace; `Ok` is the exit status (`--check`:
/// 1 when stale), `Err` a message for stderr.
pub fn run(args: &[String], root: &Path, out: &mut impl Write) -> Result<i32, String> {
    let args = parse_args(args, root)?;
    let mut built = Vec::new();
    for (name, source) in NAMES.iter().zip(&args.sources) {
        built.push((*name, build(name, source, out)?));
    }
    let mut stale = Vec::new();
    for (name, tree) in &built {
        let _ = writeln!(out, "{name}:");
        if report(
            name,
            &read_tree(&args.dest.join(name)),
            tree,
            args.diff,
            out,
        ) {
            stale.push((*name, tree));
        }
    }
    if args.check {
        return Ok(i32::from(!stale.is_empty()));
    }
    for (name, tree) in stale {
        write_companion(&args.dest, name, tree)?;
        let _ = writeln!(out, "synced {name} -> {}", args.dest.join(name).display());
    }
    Ok(0)
}

pub fn main(args: &[String]) -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."));
    let mut out = io::stdout().lock();
    match run(args, root, &mut out) {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(message) => {
            let _ = out.flush();
            eprintln!("{message}");
            ExitCode::from(if message.starts_with("usage:") { 2 } else { 1 })
        }
    }
}

/// `sync-specmint-oracle`: answer `tools/sync_specmint_oracle.py`'s
/// requests (one JSON line each) with this port: `{"report": {"name",
/// "old", "new", "full_diff"}}` (trees as `{path: base64}`) gets
/// `{"changed", "out"}`, `{"mentions": text}` the list of mentions.
pub fn oracle(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: sync-specmint-oracle".into());
    }
    let tree = |value: &Value| -> Result<Tree, String> {
        let Some(map) = value.as_object() else {
            return Err("a tree is a JSON object".into());
        };
        map.iter()
            .map(|(k, v)| {
                let data = BASE64
                    .decode(v.as_str().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                Ok((k.clone(), data))
            })
            .collect()
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut line = String::new();
    loop {
        line.clear();
        if io::BufRead::read_line(&mut stdin.lock(), &mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(());
        }
        let request: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let answer = if let Some(r) = request.get("report") {
            let mut printed = Vec::new();
            let changed = report(
                r["name"].as_str().unwrap_or_default(),
                &tree(&r["old"])?,
                &tree(&r["new"])?,
                r["full_diff"].as_bool().unwrap_or(false),
                &mut printed,
            );
            json!({"changed": changed, "out": String::from_utf8_lossy(&printed)})
        } else {
            json!(path_mentions(
                request["mentions"].as_str().unwrap_or_default()
            ))
        };
        writeln!(stdout, "{answer}").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests;
