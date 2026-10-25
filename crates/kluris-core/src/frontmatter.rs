//! Reading and writing frontmatter: markdown `---` blocks (python-frontmatter's
//! YAML handler) and the `#---` comment block of YAML neurons.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::yaml::{self, Map, Profile, Value, YamlError, is_py_space};

/// The line ending a file uses, for the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

/// A neuron's frontmatter and the body that follows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    pub meta: Map,
    /// Byte-exact text after the closing delimiter line (the whole text, BOM
    /// excluded, when there is no frontmatter).
    pub body: String,
    pub line_ending: LineEnding,
    pub had_bom: bool,
}

/// What went wrong reading a file's frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Io(String),
    NotUtf8,
    Yaml(YamlError),
}

/// A file whose frontmatter cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterError {
    pub path: PathBuf,
    pub problem: Problem,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Io(e) => write!(f, "cannot read the file: {e}"),
            Problem::NotUtf8 => f.write_str("the file is not valid UTF-8"),
            Problem::Yaml(e) => write!(f, "invalid frontmatter: {e}"),
        }
    }
}

impl fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.problem)
    }
}

impl std::error::Error for FrontmatterError {}

/// Lines as Python's text mode sees them: `\n`, `\r\n` and a lone `\r` end a
/// line. Yields `(line start, line end, next line start)` byte offsets.
fn text_mode_lines(text: &str, from: usize) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let mut pos = from;
    std::iter::from_fn(move || {
        if pos >= text.len() {
            return None;
        }
        let start = pos;
        let bytes = text.as_bytes();
        let (end, next) = match bytes[start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
        {
            None => (text.len(), text.len()),
            Some(i) => {
                let end = start + i;
                if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
                    (end, end + 2)
                } else {
                    (end, end + 1)
                }
            }
        };
        pos = next;
        Some((start, end, next))
    })
}

/// python-frontmatter's YAML boundary, `^-{3,}\s*$`, as a whole line.
fn is_delimiter(line: &str) -> bool {
    let rest = line.trim_start_matches('-');
    line.len() - rest.len() >= 3 && rest.chars().all(is_py_space)
}

/// The line ending of the first line break: CRLF, or LF otherwise.
fn line_ending(text: &str) -> LineEnding {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

/// The YAML text as v2's text-mode read hands it to PyYAML.
///
/// Known gap, left as is (decided 2026-10-25): PyYAML (YAML 1.1) also breaks
/// lines at `\u{85}`, `\u{2028}` and `\u{2029}`; saphyr-parser (YAML 1.2)
/// reads them as ordinary characters, and they are not converted here.
fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Load frontmatter YAML: a mapping is the metadata; an empty document or
/// any other value is no metadata, as in v2.
fn load_meta(yaml: &str) -> Result<Map, YamlError> {
    match yaml::load(&universal_newlines(yaml))? {
        Value::Map(map) => Ok(map),
        _ => Ok(Map::new()),
    }
}

/// Split markdown text into its frontmatter YAML and its body, the way
/// python-frontmatter finds a YAML block: leading whitespace is skipped, the
/// first line must be 3+ dashes (trailing whitespace allowed), and the next
/// such line closes it. The body is everything after the closing line, byte
/// for byte. A leading BOM is skipped. Without a closing line, or with JSON or
/// TOML frontmatter, there is no frontmatter and the body is the whole text.
pub fn split(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let start = text.len() - text.trim_start_matches(is_py_space).len();
    let mut lines = text_mode_lines(text, start);
    match lines.next() {
        Some((open, end, yaml_start)) if is_delimiter(&text[open..end]) => {
            for (line, end, next) in lines {
                if is_delimiter(&text[line..end]) {
                    return (Some(&text[yaml_start..line]), &text[next..]);
                }
            }
            (None, text)
        }
        _ => (None, text),
    }
}

/// Parse markdown text with optional frontmatter.
pub fn parse(text: &str) -> Result<Frontmatter, YamlError> {
    let had_bom = text.starts_with('\u{feff}');
    let (yaml, body) = split(text);
    let meta = match yaml {
        Some(yaml) => load_meta(yaml)?,
        None => Map::new(),
    };
    Ok(Frontmatter {
        meta,
        body: body.to_string(),
        line_ending: line_ending(text),
        had_bom,
    })
}

fn error(path: &Path, problem: Problem) -> FrontmatterError {
    FrontmatterError {
        path: path.to_path_buf(),
        problem,
    }
}

/// Read a markdown file's frontmatter and body. The file must be UTF-8.
pub fn read(path: &Path) -> Result<Frontmatter, FrontmatterError> {
    let bytes = fs::read(path).map_err(|e| error(path, Problem::Io(e.to_string())))?;
    let text = String::from_utf8(bytes).map_err(|_| error(path, Problem::NotUtf8))?;
    parse(&text).map_err(|e| error(path, Problem::Yaml(e)))
}

/// Python's `str.splitlines(keepends=True)` line boundaries.
fn is_py_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{b}'
            | '\u{c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Python's `str.splitlines(keepends=True)`.
fn py_splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !is_py_line_break(c) {
            continue;
        }
        let mut end = i + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
            chars.next();
            end += 1;
        }
        lines.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn is_hash_fence(line: &str) -> bool {
    line.trim_end_matches(['\r', '\n'])
        .trim_end_matches(is_py_space)
        == "#---"
}

/// Find the `#---` block the way v2's `_read_yaml_neuron` does. Returns the
/// block text (`# `/`#` prefixes removed) and the byte offset of the body.
fn yaml_neuron_block(content: &str) -> Option<(String, usize)> {
    let lines = py_splitlines(content);
    let first = lines
        .iter()
        .position(|line| !line.trim_matches(is_py_space).is_empty())?;
    if !is_hash_fence(lines[first]) {
        return None;
    }
    let mut offset: usize = lines[..=first].iter().map(|l| l.len()).sum();
    let mut block = String::new();
    for line in &lines[first + 1..] {
        offset += line.len();
        if is_hash_fence(line) {
            return Some((block, offset));
        }
        // The gate allows indented comment lines; the prefix strip does not.
        let stripped = line.strip_prefix("# ").or_else(|| line.strip_prefix('#'))?;
        block.push_str(stripped);
    }
    None
}

/// Parse a YAML neuron's text: its `#---` block and the rest of the file, byte
/// for byte. A leading BOM is skipped. Without a complete block there is no
/// metadata and the body is the whole text.
pub fn parse_yaml_neuron(text: &str) -> Result<Frontmatter, YamlError> {
    let had_bom = text.starts_with('\u{feff}');
    let content = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (meta, body) = match yaml_neuron_block(content) {
        Some((block, body_start)) => (load_meta(&block)?, &content[body_start..]),
        None => (Map::new(), content),
    };
    Ok(Frontmatter {
        meta,
        body: body.to_string(),
        line_ending: line_ending(content),
        had_bom,
    })
}

/// Read a YAML neuron's `#---` block and the rest of the file. Invalid UTF-8
/// is replaced with U+FFFD, as v2 decodes it.
pub fn read_yaml_neuron(path: &Path) -> Result<Frontmatter, FrontmatterError> {
    let bytes = fs::read(path).map_err(|e| error(path, Problem::Io(e.to_string())))?;
    parse_yaml_neuron(&String::from_utf8_lossy(&bytes)).map_err(|e| error(path, Problem::Yaml(e)))
}

fn has_yaml_suffix(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
}

/// Read any neuron like v2's `read_frontmatter`: a `.yml`/`.yaml` suffix (any
/// case) reads a YAML neuron, everything else markdown.
pub fn read_neuron(path: &Path) -> Result<Frontmatter, FrontmatterError> {
    if has_yaml_suffix(path) {
        read_yaml_neuron(path)
    } else {
        read(path)
    }
}

/// The opt-in gate (v2 `has_yaml_opt_in_block`): the first 4096 bytes open,
/// after blank lines, with `#---`, continue with lines whose first non-blank
/// character is `#`, and close with `#---`.
pub fn has_yaml_opt_in_block(path: &Path) -> bool {
    let mut head = Vec::with_capacity(4096);
    let read = fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head));
    if read.is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&head);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut lines = py_splitlines(text)
        .into_iter()
        .map(|l| l.trim_end_matches(is_py_line_break))
        .skip_while(|l| l.trim_matches(is_py_space).is_empty());
    if !lines.next().is_some_and(is_hash_fence) {
        return false;
    }
    for line in lines {
        if is_hash_fence(line) {
            return true;
        }
        if !line.trim_start_matches(is_py_space).starts_with('#') {
            return false;
        }
    }
    false
}

/// Whether `path` is a YAML neuron as v2's neuron walk decides: a name ending
/// in `.yml`/`.yaml` (case-sensitive), not `kluris.yml`, and the opt-in gate.
pub fn is_yaml_neuron(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    (name.ends_with(".yml") || name.ends_with(".yaml"))
        && name != "kluris.yml"
        && has_yaml_opt_in_block(path)
}

/// The body as v2 writes it after reading it back: universal newlines, the
/// leading whitespace-only lines and all trailing whitespace removed. Kluris 3
/// keeps the first text line's indentation (python-frontmatter strips it).
fn markdown_body(body: &str) -> String {
    let body = universal_newlines(body);
    let Some(first_text) = body.find(|c: char| !is_py_space(c)) else {
        return String::new();
    };
    let line_start = body[..first_text].rfind('\n').map_or(0, |i| i + 1);
    body[line_start..].trim_end_matches(is_py_space).to_string()
}

/// A markdown neuron as v2 writes it: python-frontmatter's
/// `dumps(Post(body, **meta))` plus a final newline, i.e. `---`, the sorted
/// libyaml YAML, `---`, a blank line, the body (see [`markdown_body`]) and
/// `\n`; without a body the file ends at the closing `---`. Written LF;
/// `LineEnding::Crlf` gives every line `\r\n`, as v2's text-mode write does on
/// Windows.
pub fn write_markdown(meta: &Map, body: &str, line_ending: LineEnding) -> String {
    let yaml = yaml::emit(meta, Profile::Md);
    let mut out = format!("---\n{}\n---\n", yaml.trim_matches(is_py_space));
    let body = markdown_body(body);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
        out.push('\n');
    }
    match line_ending {
        LineEnding::Lf => out,
        LineEnding::Crlf => out.replace('\n', "\r\n"),
    }
}

/// A YAML neuron as v2's `_write_yaml_neuron` writes it: the `yaml.safe_dump`
/// output with every line prefixed `# ` (`#` when empty) between `#---` lines,
/// LF, then the body byte for byte.
pub fn write_yaml_neuron(meta: &Map, body: &str) -> String {
    let dumped = yaml::emit(meta, Profile::Yml);
    let lines: Vec<String> = py_splitlines(dumped.trim_end_matches('\n'))
        .into_iter()
        .map(|line| match line.trim_end_matches(is_py_line_break) {
            "" => "#".to_string(),
            line => format!("# {line}"),
        })
        .collect();
    format!("#---\n{}\n#---\n{body}", lines.join("\n"))
}
