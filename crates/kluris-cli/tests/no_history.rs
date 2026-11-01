//! The code, its tests and its docs tell only the current story: no task,
//! finding or spec IDs, no dates in prose, no talk of earlier releases.
//!
//! What is read, and how:
//! - `.rs` files under `crates/*/src` and `crates/*/tests`: only their
//!   comments, doc comments and the names of `#[test]` functions, through
//!   [`rust_prose`]. String literals are data and are skipped, and so is
//!   every other file under those folders (licences, fixtures, snapshots,
//!   corpora).
//! - Every file, as text, under `crates/*/templates`, `xtask/src`,
//!   `scripts` and `.github`, and `README.md`, `CLAUDE.md`, `AGENTS.md` and
//!   `THIRD_PARTY_NOTICES.md`.
//! - The name of every folder under `crates/*/tests`, `xtask/fixtures` and
//!   `scripts`, fixture folders included. File names are data: a fixture
//!   named for the release that rendered it keeps its name.
//!
//! Names (test functions and folders) are matched with `_` read as a space,
//! so a test named `reads_the_v2_config` is a hit.
//!
//! What is not a hit:
//! - the paths in [`EXEMPT`] and the exact pieces of data in [`EXEMPT_TEXT`];
//! - a date on a line inside a fenced code block of a Markdown file: there it
//!   is example front matter or output (the other patterns still apply);
//! - a version right after `@`, as in a workflow's `uses: owner/action@vN`,
//!   which is a version pin and never prose.
//!
//! What the tokenizer misses: prose inside string literals (help text,
//! messages, `#[doc = "..."]`), identifiers other than `#[test]` function
//! names (helpers, constants, modules), tests that a macro names, a test
//! attribute spelled other than `#[test]`, and dates written other than as
//! `YYYY-MM-DD`.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Paths read as data, never as prose; a folder ends with `/`.
const EXEMPT: &[(&str, &str)] = &[
    (
        "xtask/src/sync_specmint/LICENSE-PSF",
        "a licence text, kept as published",
    ),
    (
        "crates/kluris-core/src/pycase_tables.rs",
        "the frozen Unicode case tables",
    ),
    (
        "scripts/release/fixtures/",
        "fixture contents: dist plans recorded as dist writes them",
    ),
];

/// Pieces of data in files otherwise read as prose: the path, then the
/// exact text holding the hit.
const EXEMPT_TEXT: &[(&str, &str)] = &[
    (
        "scripts/release/test_build_local_ps1.py",
        "'rustc 1.99.0 (00000000 2026-01-01)'",
    ),
    (
        "scripts/release/test_release_notes.py",
        "\"## Install kluris 3.0.0",
    ),
    (
        "xtask/src/linux/tests.rs",
        "installer-smoke: ok (Linux x86_64, kluris 3.0.0-dev,",
    ),
];

/// The files read as text, besides the folders in [`TEXT_ROOTS`].
const TEXT_FILES: &[&str] = &[
    "README.md",
    "CLAUDE.md",
    "AGENTS.md",
    "THIRD_PARTY_NOTICES.md",
];

/// The folders whose files are all read as text.
const TEXT_ROOTS: &[&str] = &["xtask/src", "scripts", ".github"];

/// The folders whose folder names are read, besides `crates/*/tests`.
const FOLDER_ROOTS: &[&str] = &["xtask/fixtures", "scripts"];

/// A pattern, and whether it is a date (data in a Markdown code block).
struct Pattern {
    regex: Regex,
    date: bool,
}

fn patterns() -> Vec<Pattern> {
    let words = [
        r"\bT-\d+\b",
        r"\bF\d{3}-\d\b",
        r"\bPAR-\d+\b",
        r"\bTEST-K3-",
        r"\bKS-\d+\b",
        r"(?i)\bresearch-0\d",
        r"(?i)\binterview-0\d",
        r"(?i)\bdecision log\b",
        r"(?i)(?:^|[^@\w])v2\b",
        r"(?i)\bkluris [23]\b",
        r"(?i)\bno longer\b",
        r"(?i)\bused to\b",
        r"\b\w+\.py:\d+",
    ];
    let mut all: Vec<Pattern> = words
        .iter()
        .map(|p| Pattern {
            regex: Regex::new(p).expect("a valid pattern"),
            date: false,
        })
        .collect();
    all.push(Pattern {
        regex: Regex::new(r"\b20\d{2}-\d{2}-\d{2}\b").expect("a valid pattern"),
        date: true,
    });
    all
}

/// The workspace root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root resolves")
}

/// `path` relative to the root, with `/` separators.
fn rel(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn exempt(rel: &str) -> bool {
    EXEMPT.iter().any(|(path, _)| {
        if path.ends_with('/') {
            rel.starts_with(path)
        } else {
            rel == *path
        }
    })
}

/// Every file under `dir`, sorted.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut |path, is_dir| {
        if !is_dir {
            out.push(path.to_path_buf());
        }
    });
    out.sort();
    out
}

/// Every folder under `dir` (not `dir` itself), sorted.
fn folders_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut |path, is_dir| {
        if is_dir {
            out.push(path.to_path_buf());
        }
    });
    out.sort();
    out
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path, bool)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            visit(&path, true);
            walk(&path, visit);
        } else if kind.is_file() {
            visit(&path, false);
        }
    }
}

/// The crate folders, `crates/*`.
fn crates(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("crates/ reads")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// A piece of prose: where it starts, and its text.
struct Prose {
    line: usize,
    text: String,
}

/// The comments, doc comments and `#[test]` function names of Rust
/// source, each comment line on its own. String, byte, C and raw string
/// literals and char literals are skipped; block comments nest.
fn rust_prose(src: &str) -> Vec<Prose> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    let mut test_pending = false;
    let mut expect_test_name = false;
    let at = |i: usize| chars.get(i).copied();
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if at(i + 1) == Some('/') => {
                let start = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                out.push(Prose {
                    line,
                    text: chars[start..i].iter().collect(),
                });
            }
            '/' if at(i + 1) == Some('*') => {
                let mut depth = 0;
                let mut text = String::new();
                let mut text_line = line;
                while i < chars.len() {
                    if chars[i] == '/' && at(i + 1) == Some('*') {
                        depth += 1;
                        text.push_str("/*");
                        i += 2;
                    } else if chars[i] == '*' && at(i + 1) == Some('/') {
                        depth -= 1;
                        text.push_str("*/");
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else if chars[i] == '\n' {
                        out.push(Prose {
                            line: text_line,
                            text: std::mem::take(&mut text),
                        });
                        line += 1;
                        text_line = line;
                        i += 1;
                    } else {
                        text.push(chars[i]);
                        i += 1;
                    }
                }
                out.push(Prose {
                    line: text_line,
                    text,
                });
            }
            '"' => i = skip_string(&chars, i + 1, &mut line),
            '\'' => {
                // A char literal ('a', '\'', '\u{1F600}'), else a lifetime
                // or a label ('a).
                if at(i + 1) == Some('\\') {
                    i += 3;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if at(i + 2) == Some('\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '#' if src_starts_with(&chars, i, "#[test]") => {
                test_pending = true;
                i += "#[test]".len();
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                // Raw strings: r"..", r#".."#, br"..", cr#".."#.
                if matches!(word.as_str(), "r" | "br" | "cr")
                    && matches!(at(i), Some('"') | Some('#'))
                {
                    let mut hashes = 0;
                    while at(i) == Some('#') {
                        hashes += 1;
                        i += 1;
                    }
                    if at(i) == Some('"') {
                        i = skip_raw_string(&chars, i + 1, hashes, &mut line);
                    }
                    continue;
                }
                if expect_test_name {
                    out.push(Prose {
                        line,
                        text: format!("fn {}", word.replace('_', " ")),
                    });
                    expect_test_name = false;
                    test_pending = false;
                } else if test_pending && word == "fn" {
                    expect_test_name = true;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn src_starts_with(chars: &[char], i: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(k, c)| chars.get(i + k) == Some(&c))
}

/// The index after the closing `"` of a string whose body starts at `i`.
fn skip_string(chars: &[char], mut i: usize, line: &mut usize) -> usize {
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if chars.get(i + 1) == Some(&'\n') {
                    *line += 1;
                }
                i += 2;
            }
            '"' => return i + 1,
            '\n' => {
                *line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    i
}

/// The index after the closing `"` and `hashes` `#`s of a raw string whose
/// body starts at `i`.
fn skip_raw_string(chars: &[char], mut i: usize, hashes: usize, line: &mut usize) -> usize {
    while i < chars.len() {
        if chars[i] == '\n' {
            *line += 1;
        }
        if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
            return i + 1 + hashes;
        }
        i += 1;
    }
    i
}

/// The lines of a text file, each with whether it sits inside a fenced
/// code block of a Markdown file.
fn text_prose(rel: &str, src: &str) -> Vec<(Prose, bool)> {
    let markdown = rel.ends_with(".md");
    let mut fenced = false;
    let mut out = Vec::new();
    for (n, text) in src.lines().enumerate() {
        let fence = markdown && text.trim_start().starts_with("```");
        out.push((
            Prose {
                line: n + 1,
                text: text.to_string(),
            },
            fenced && !fence,
        ));
        if fence {
            fenced = !fenced;
        }
    }
    out
}

/// The hits in one piece of prose, as `path:line: text`.
fn hits_in(
    patterns: &[Pattern],
    rel: &str,
    prose: &Prose,
    in_code_block: bool,
    hits: &mut Vec<String>,
) {
    for pattern in patterns {
        if pattern.date && in_code_block {
            continue;
        }
        for m in pattern.regex.find_iter(&prose.text) {
            let exempt = EXEMPT_TEXT.iter().any(|(path, data)| {
                *path == rel
                    && prose
                        .text
                        .match_indices(data)
                        .any(|(start, d)| start <= m.start() && m.end() <= start + d.len())
            });
            if !exempt {
                hits.push(if prose.line == 0 {
                    format!("{rel}/: {}", prose.text)
                } else {
                    format!("{rel}:{}: {}", prose.line, prose.text.trim())
                });
                return;
            }
        }
    }
}

/// Every hit in the tree at `root`.
fn scan(root: &Path) -> Vec<String> {
    let patterns = patterns();
    let mut hits = Vec::new();
    for krate in crates(root) {
        for sub in ["src", "tests"] {
            for path in files_under(&krate.join(sub)) {
                let rel = rel(root, &path);
                if exempt(&rel) || path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let src = fs::read_to_string(&path).expect("a Rust file reads");
                for prose in rust_prose(&src) {
                    hits_in(&patterns, &rel, &prose, false, &mut hits);
                }
            }
        }
    }
    let mut text_files: Vec<PathBuf> = crates(root)
        .iter()
        .flat_map(|krate| files_under(&krate.join("templates")))
        .collect();
    for dir in TEXT_ROOTS {
        text_files.extend(files_under(&root.join(dir)));
    }
    text_files.extend(TEXT_FILES.iter().map(|f| root.join(f)));
    for path in text_files {
        let rel = rel(root, &path);
        if exempt(&rel) {
            continue;
        }
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{rel} reads: {e}"));
        let src = String::from_utf8_lossy(&bytes);
        for (prose, in_code_block) in text_prose(&rel, &src) {
            hits_in(&patterns, &rel, &prose, in_code_block, &mut hits);
        }
    }
    let mut folder_roots: Vec<PathBuf> = crates(root).iter().map(|k| k.join("tests")).collect();
    folder_roots.extend(FOLDER_ROOTS.iter().map(|d| root.join(d)));
    for dir in folder_roots {
        for folder in folders_under(&dir) {
            let rel = rel(root, &folder);
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().replace('_', " "))
                .unwrap_or_default();
            let prose = Prose {
                line: 0,
                text: format!("folder {name}"),
            };
            hits_in(&patterns, &rel, &prose, false, &mut hits);
        }
    }
    hits
}

#[test]
fn code_tests_and_docs_tell_only_the_current_story() {
    let hits = scan(&root());
    assert!(
        hits.is_empty(),
        "{} hits; rewrite each as the current fact or delete it:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn claude_md_and_agents_md_are_identical() {
    let root = root();
    let claude = fs::read(root.join("CLAUDE.md")).expect("CLAUDE.md reads");
    let agents = fs::read(root.join("AGENTS.md")).expect("AGENTS.md reads");
    assert!(claude == agents, "CLAUDE.md and AGENTS.md differ");
}

#[test]
fn the_tokenizer_reads_comments_and_test_names_and_skips_strings() {
    let src = concat!(
        "// one\n",
        "/* two /* nested */\n still two */\n",
        "let s = \"// not a comment\";\n",
        "let r = r#\"/* not \"a\" comment */\"#;\n",
        "let c = '\"'; let q = '\\''; let l: &'a str = x;\n",
        "#[test]\n#[ignore]\nfn a_test_name() {}\n",
        "fn not_a_test() {}\n",
        "/// three\n",
    );
    let prose: Vec<(usize, String)> = rust_prose(src)
        .into_iter()
        .map(|p| (p.line, p.text))
        .collect();
    assert_eq!(
        prose,
        vec![
            (1, "// one".to_string()),
            (2, "/* two /* nested */".to_string()),
            (3, " still two */".to_string()),
            (9, "fn a test name".to_string()),
            (11, "/// three".to_string()),
        ]
    );
}
