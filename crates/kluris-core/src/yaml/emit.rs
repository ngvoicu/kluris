//! YAML writing that reproduces PyYAML 6.0.3 byte for byte, the way v2 writes
//! frontmatter, YAML neurons and configs (block style, width 80, indent 2).
//!
//! A port of PyYAML's `SafeRepresenter` and serializer (Python code in every
//! profile) and of its two emitters: the pure-Python `emitter.py` and
//! libyaml's `emitter.c` (0.2.5, behind `CSafeDumper`). The emitters differ in
//! a handful of places, marked where they occur.

use super::resolve::{ImplicitTag, implicit_tag};
use super::value::{Date, DateTime, Map, Value};

const BEST_WIDTH: usize = 80;
const BEST_INDENT: usize = 2;

/// Which v2 writer to reproduce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Markdown frontmatter: python-frontmatter's `yaml.dump(meta,
    /// Dumper=CSafeDumper, default_flow_style=False, allow_unicode=True)`;
    /// libyaml's emitter, keys sorted.
    Md,
    /// YAML neurons: `yaml.safe_dump(meta, sort_keys=True,
    /// default_flow_style=False)`; the Python emitter, non-ASCII escaped.
    Yml,
    /// `~/.kluris/config.yml` and `kluris.yml`: `yaml.dump(data,
    /// default_flow_style=False, sort_keys=False)`; the Python emitter,
    /// insertion order, non-ASCII escaped.
    Config,
}

impl Profile {
    /// libyaml (with `allow_unicode`) rather than the Python emitter (without).
    fn libyaml(self) -> bool {
        self == Profile::Md
    }

    fn sort_keys(self) -> bool {
        self != Profile::Config
    }
}

/// Write a mapping as one YAML document, exactly as the profile's v2 writer.
pub fn emit(map: &Map, profile: Profile) -> String {
    let mut emitter = Emitter {
        profile,
        out: String::new(),
        column: 0,
        whitespace: true,
        indention: true,
        indent: None,
        indents: Vec::new(),
    };
    emitter.mapping(map);
    // The implicit document end.
    emitter.write_indent();
    emitter.out
}

/// What the scalar is, for the serializer's implicit flags and the tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Null,
    Bool,
    Int,
    Float,
    Timestamp,
    Str,
}

impl Tag {
    fn shorthand(self) -> &'static str {
        match self {
            Tag::Null => "!!null",
            Tag::Bool => "!!bool",
            Tag::Int => "!!int",
            Tag::Float => "!!float",
            Tag::Timestamp => "!!timestamp",
            Tag::Str => "!!str",
        }
    }

    /// Whether the resolver gives this tag to `text` written plain.
    fn resolves(self, text: &str) -> bool {
        matches!(
            (self, implicit_tag(text)),
            (Tag::Null, ImplicitTag::Null)
                | (Tag::Bool, ImplicitTag::Bool)
                | (Tag::Int, ImplicitTag::Int)
                | (Tag::Float, ImplicitTag::Float)
                | (Tag::Timestamp, ImplicitTag::Timestamp)
                | (Tag::Str, ImplicitTag::Str)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Single,
    Double,
}

/// The node's place in its parent (`expect_node`'s flags).
#[derive(Debug, Clone, Copy, Default)]
struct Context {
    mapping: bool,
    simple_key: bool,
}

/// `analyze_scalar`'s verdict, reduced to what block context needs.
struct Analysis {
    empty: bool,
    multiline: bool,
    allow_block_plain: bool,
    allow_single_quoted: bool,
    /// Characters (Python) or UTF-8 bytes (libyaml), for the simple-key limit.
    length: usize,
}

/// `\0`, space, tab and the line breaks: what counts as "whitespace" around
/// an indicator.
fn is_blankz(c: char) -> bool {
    matches!(
        c,
        '\0' | ' ' | '\t' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Line breaks: libyaml's `IS_BREAK` includes `\r`, Python's emitter does not.
fn is_break(c: char, libyaml: bool) -> bool {
    matches!(c, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}') || (libyaml && c == '\r')
}

/// The characters written as they are. libyaml (with `allow_unicode`) uses
/// `IS_PRINTABLE`, which has no range past the BMP and excludes `\u{85}`;
/// the Python emitter without `allow_unicode` takes printable ASCII only.
fn is_printable(c: char, libyaml: bool) -> bool {
    c == '\n'
        || (' '..='~').contains(&c)
        || (libyaml
            && (('\u{a0}'..='\u{d7ff}').contains(&c)
                || (('\u{e000}'..='\u{fffd}').contains(&c) && c != '\u{feff}')))
}

fn analyze(text: &str, libyaml: bool) -> Analysis {
    let length = if libyaml {
        text.len()
    } else {
        text.chars().count()
    };
    if text.is_empty() {
        return Analysis {
            empty: true,
            multiline: false,
            allow_block_plain: true,
            allow_single_quoted: true,
            length,
        };
    }
    let chars: Vec<char> = text.chars().collect();
    let mut block_indicators = text.starts_with("---") || text.starts_with("...");
    let mut line_breaks = false;
    let mut special_characters = false;
    let (mut leading_space, mut leading_break) = (false, false);
    let (mut trailing_space, mut trailing_break) = (false, false);
    let (mut break_space, mut space_break) = (false, false);
    let (mut previous_space, mut previous_break) = (false, false);
    let mut preceded_by_whitespace = true;
    let mut followed_by_whitespace = chars.get(1).is_none_or(|&c| is_blankz(c));
    let last = chars.len() - 1;
    for (index, &c) in chars.iter().enumerate() {
        if index == 0 {
            if "#,[]{}&*!|>'\"%@`".contains(c)
                || ((c == '?' || c == ':' || c == '-') && followed_by_whitespace)
            {
                block_indicators = true;
            }
        } else if (c == ':' && followed_by_whitespace) || (c == '#' && preceded_by_whitespace) {
            block_indicators = true;
        }
        let breaks = is_break(c, libyaml);
        line_breaks |= breaks;
        special_characters |= !is_printable(c, libyaml);
        if c == ' ' {
            leading_space |= index == 0;
            trailing_space |= index == last;
            break_space |= previous_break;
            (previous_space, previous_break) = (true, false);
        } else if breaks {
            leading_break |= index == 0;
            trailing_break |= index == last;
            space_break |= previous_space;
            (previous_space, previous_break) = (false, true);
        } else {
            (previous_space, previous_break) = (false, false);
        }
        preceded_by_whitespace = is_blankz(c);
        followed_by_whitespace = chars.get(index + 2).is_none_or(|&c| is_blankz(c));
    }
    let allow_single_quoted = !(break_space || space_break || special_characters);
    let allow_block_plain = allow_single_quoted
        && !(leading_space || leading_break || trailing_space || trailing_break)
        && !line_breaks
        && !block_indicators;
    Analysis {
        empty: false,
        multiline: line_breaks,
        allow_block_plain,
        allow_single_quoted,
        length,
    }
}

/// `ESCAPE_REPLACEMENTS`, then `\xXX`, `\uXXXX` or `\UXXXXXXXX`.
fn escape(c: char) -> String {
    let short = match c {
        '\0' => '0',
        '\u{7}' => 'a',
        '\u{8}' => 'b',
        '\t' => 't',
        '\n' => 'n',
        '\u{b}' => 'v',
        '\u{c}' => 'f',
        '\r' => 'r',
        '\u{1b}' => 'e',
        '"' => '"',
        '\\' => '\\',
        '\u{85}' => 'N',
        '\u{a0}' => '_',
        '\u{2028}' => 'L',
        '\u{2029}' => 'P',
        _ => {
            let code = u32::from(c);
            return if code <= 0xff {
                format!("\\x{code:02X}")
            } else if code <= 0xffff {
                format!("\\u{code:04X}")
            } else {
                format!("\\U{code:08X}")
            };
        }
    };
    format!("\\{short}")
}

/// Python's `repr(float)`: the shortest digits that round-trip, in exponent
/// form when the decimal exponent is below -4 or above 16.
fn python_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.into();
    }
    // `{:e}` gives the shortest round-trip digits (`1.2345e-7`), but rounds an
    // exact tie up where Python takes the even neighbour (70368744177664.125
    // is `...64.12`). The exact mode rounds half to even: use it at the same
    // length whenever it still reads back as the same float.
    let shortest = format!("{:e}", value.abs());
    let length = shortest
        .split('e')
        .next()
        .map_or(1, |m| m.replace('.', "").len());
    let even = format!("{:.*e}", length.saturating_sub(1), value.abs());
    let scientific = if even.parse::<f64>() == Ok(value.abs()) {
        even
    } else {
        shortest
    };
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    // The value is 0.<digits> x 10^decpt.
    let decpt = exponent + 1;
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let len = digits.len() as i32;
    let body = if decpt <= -4 || decpt > 16 {
        let (first, rest) = digits.split_at(1);
        let dot = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp = decpt - 1;
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{first}{dot}e{exp_sign}{:02}", exp.unsigned_abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat(decpt.unsigned_abs() as usize))
    } else if decpt >= len {
        format!("{digits}{}.0", "0".repeat((decpt - len) as usize))
    } else {
        let (int, frac) = digits.split_at(decpt as usize);
        format!("{int}.{frac}")
    };
    format!("{sign}{body}")
}

/// `SafeRepresenter.represent_float`.
fn represent_float(value: f64) -> String {
    if value.is_nan() {
        ".nan".into()
    } else if value == f64::INFINITY {
        ".inf".into()
    } else if value == f64::NEG_INFINITY {
        "-.inf".into()
    } else {
        let repr = python_float_repr(value).to_lowercase();
        if !repr.contains('.') && repr.contains('e') {
            repr.replacen('e', ".0e", 1)
        } else {
            repr
        }
    }
}

/// `date.isoformat()`.
fn date_iso(date: &Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

/// `datetime.isoformat(' ')`.
fn datetime_iso(dt: &DateTime) -> String {
    let mut out = format!(
        "{} {:02}:{:02}:{:02}",
        date_iso(&dt.date()),
        dt.hour(),
        dt.minute(),
        dt.second()
    );
    if dt.microsecond() != 0 {
        out.push_str(&format!(".{:06}", dt.microsecond()));
    }
    if let Some(offset) = dt.utc_offset() {
        let sign = if offset < 0 { '-' } else { '+' };
        let abs = offset.unsigned_abs();
        out.push_str(&format!("{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60));
        if abs % 60 != 0 {
            out.push_str(&format!(":{:02}", abs % 60));
        }
    }
    out
}

/// A scalar's tag and text as `SafeRepresenter` makes them.
fn represent(value: &Value) -> (Tag, String) {
    match value {
        Value::Null => (Tag::Null, "null".into()),
        Value::Bool(b) => (Tag::Bool, if *b { "true" } else { "false" }.into()),
        Value::Int(i) => (Tag::Int, i.to_string()),
        Value::Float(f) => (Tag::Float, represent_float(*f)),
        Value::Str(s) => (Tag::Str, s.clone()),
        Value::Date(d) => (Tag::Timestamp, date_iso(d)),
        Value::DateTime(dt) => (Tag::Timestamp, datetime_iso(dt)),
        // Collections never reach here.
        Value::Seq(_) | Value::Map(_) => (Tag::Str, String::new()),
    }
}

struct Emitter {
    profile: Profile,
    out: String,
    column: usize,
    whitespace: bool,
    indention: bool,
    indent: Option<usize>,
    indents: Vec<Option<usize>>,
}

impl Emitter {
    fn libyaml(&self) -> bool {
        self.profile.libyaml()
    }

    // --- structure ---------------------------------------------------------

    fn increase_indent(&mut self, flow: bool, indentless: bool) {
        self.indents.push(self.indent);
        self.indent = match self.indent {
            None => Some(if flow { BEST_INDENT } else { 0 }),
            Some(indent) if !indentless => Some(indent + BEST_INDENT),
            same => same,
        };
    }

    fn pop_indent(&mut self) {
        self.indent = self.indents.pop().flatten();
    }

    fn node(&mut self, value: &Value, context: Context) {
        match value {
            Value::Seq(items) if items.is_empty() => self.empty_flow("[", "]"),
            Value::Map(map) if map.is_empty() => self.empty_flow("{", "}"),
            Value::Seq(items) => self.block_sequence(items, context),
            Value::Map(map) => self.mapping(map),
            scalar => self.scalar(scalar, context),
        }
    }

    /// `[]` and `{}`: the flow style of an empty collection.
    fn empty_flow(&mut self, open: &str, close: &str) {
        self.write_indicator(open, true, true, false);
        self.write_indicator(close, false, false, false);
    }

    fn block_sequence(&mut self, items: &[Value], context: Context) {
        // A sequence that is a mapping value sits level with its key.
        let indentless = context.mapping && !self.indention;
        self.increase_indent(false, indentless);
        for item in items {
            self.write_indent();
            self.write_indicator("-", true, false, true);
            self.node(item, Context::default());
        }
        self.pop_indent();
    }

    /// A block mapping (`{}` when empty, also at the root).
    fn mapping(&mut self, map: &Map) {
        if map.is_empty() {
            self.empty_flow("{", "}");
            return;
        }
        let mut entries: Vec<(&String, &Value)> = map.iter().collect();
        if self.profile.sort_keys() {
            entries.sort_by(|a, b| a.0.cmp(b.0));
        }
        self.increase_indent(false, false);
        let value_context = Context {
            mapping: true,
            simple_key: false,
        };
        for (key, value) in entries {
            self.write_indent();
            let analysis = analyze(key, self.libyaml());
            if self.is_simple_key(&analysis) {
                let key_context = Context {
                    mapping: true,
                    simple_key: true,
                };
                self.str_scalar(key, &analysis, key_context);
                self.write_indicator(":", false, false, false);
            } else {
                self.write_indicator("?", true, false, true);
                self.str_scalar(key, &analysis, value_context);
                self.write_indent();
                self.write_indicator(":", true, false, true);
            }
            self.node(value, value_context);
        }
        self.pop_indent();
    }

    /// `check_simple_key` for a string key. Python counts characters plus the
    /// `!!str` tag it prepares and needs fewer than 128; libyaml counts bytes
    /// (no tag for an implicit string), allows up to 128 and an empty key.
    fn is_simple_key(&self, analysis: &Analysis) -> bool {
        if self.libyaml() {
            !analysis.multiline && analysis.length <= 128
        } else {
            !analysis.empty
                && !analysis.multiline
                && analysis.length + Tag::Str.shorthand().len() < 128
        }
    }

    // --- scalars -----------------------------------------------------------

    fn str_scalar(&mut self, text: &str, analysis: &Analysis, context: Context) {
        self.write_scalar(Tag::Str, text, analysis, context);
    }

    fn scalar(&mut self, value: &Value, context: Context) {
        let (tag, text) = represent(value);
        let analysis = analyze(&text, self.libyaml());
        self.write_scalar(tag, &text, &analysis, context);
    }

    fn write_scalar(&mut self, tag: Tag, text: &str, analysis: &Analysis, context: Context) {
        // The serializer's implicit flags: plain text resolves back to the
        // tag; quoted text is always a string.
        let plain_implicit = tag.resolves(text);
        let quoted_implicit = tag == Tag::Str;
        let style = self.choose_style(analysis, context, plain_implicit, quoted_implicit);
        let tagged = if self.libyaml() {
            // libyaml writes the tag only when neither flag holds, or `!`.
            if !plain_implicit && !quoted_implicit {
                Some(tag.shorthand())
            } else if !quoted_implicit && style != Style::Plain {
                Some("!")
            } else {
                None
            }
        } else if (style == Style::Plain && plain_implicit)
            || (style != Style::Plain && quoted_implicit)
        {
            None
        } else {
            Some(tag.shorthand())
        };
        if let Some(tag) = tagged {
            self.write_indicator(tag, true, false, false);
        }
        self.increase_indent(true, false);
        let split = !context.simple_key;
        match style {
            Style::Plain => self.write_plain(text, split),
            Style::Single => self.write_single_quoted(text, split),
            Style::Double if self.libyaml() => self.write_double_quoted_libyaml(text, split),
            Style::Double => self.write_double_quoted_python(text, split),
        }
        self.pop_indent();
    }

    fn choose_style(
        &self,
        analysis: &Analysis,
        context: Context,
        plain_implicit: bool,
        quoted_implicit: bool,
    ) -> Style {
        if self.libyaml() {
            // `yaml_emitter_select_scalar_style`: with a tag (neither flag),
            // plain stays allowed.
            if context.simple_key && analysis.multiline {
                return Style::Double;
            }
            let has_tag = !plain_implicit && !quoted_implicit;
            let single = !analysis.allow_block_plain
                || (analysis.length == 0 && context.simple_key)
                || (!has_tag && !plain_implicit);
            match (single, analysis.allow_single_quoted) {
                (false, _) => Style::Plain,
                (true, true) => Style::Single,
                (true, false) => Style::Double,
            }
        } else if plain_implicit
            && !(context.simple_key && (analysis.empty || analysis.multiline))
            && analysis.allow_block_plain
        {
            Style::Plain
        } else if analysis.allow_single_quoted && !(context.simple_key && analysis.multiline) {
            Style::Single
        } else {
            Style::Double
        }
    }

    // --- writers -----------------------------------------------------------

    fn write_str(&mut self, text: &str) {
        self.out.push_str(text);
        self.column += text.chars().count();
    }

    fn write_chars(&mut self, chars: &[char]) {
        self.out.extend(chars);
        self.column += chars.len();
    }

    fn write_indicator(
        &mut self,
        indicator: &str,
        need_whitespace: bool,
        whitespace: bool,
        indention: bool,
    ) {
        if !self.whitespace && need_whitespace {
            self.write_str(" ");
        }
        self.write_str(indicator);
        self.whitespace = whitespace;
        self.indention = self.indention && indention;
    }

    fn write_indent(&mut self) {
        let indent = self.indent.unwrap_or(0);
        if !self.indention || self.column > indent || (self.column == indent && !self.whitespace) {
            self.write_line_break(None);
        }
        if self.column < indent {
            self.whitespace = true;
            self.out.push_str(&" ".repeat(indent - self.column));
            self.column = indent;
        }
    }

    /// A line break: `\n`, or the break character itself.
    fn write_line_break(&mut self, data: Option<char>) {
        self.out.push(data.unwrap_or('\n'));
        self.whitespace = true;
        self.indention = true;
        self.column = 0;
    }

    /// The line breaks `chars` holds, with an extra one ahead of a leading
    /// `\n` (a single break inside a quoted scalar folds into a space).
    fn write_breaks(&mut self, chars: &[char]) {
        if chars.first() == Some(&'\n') {
            self.write_line_break(None);
        }
        for &c in chars {
            self.write_line_break((c != '\n').then_some(c));
        }
        self.write_indent();
    }

    fn write_plain(&mut self, text: &str, split: bool) {
        if text.is_empty() {
            return;
        }
        if !self.whitespace {
            self.write_str(" ");
        }
        self.whitespace = false;
        self.indention = false;
        let chars: Vec<char> = text.chars().collect();
        let libyaml = self.libyaml();
        let (mut spaces, mut breaks) = (false, false);
        let (mut start, mut end) = (0, 0);
        while end <= chars.len() {
            let ch = chars.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end && self.column > BEST_WIDTH && split {
                        self.write_indent();
                        self.whitespace = false;
                        self.indention = false;
                    } else {
                        self.write_chars(&chars[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if ch.is_none_or(|c| !is_break(c, libyaml)) {
                    self.write_breaks(&chars[start..end]);
                    self.whitespace = false;
                    self.indention = false;
                    start = end;
                }
            } else if ch.is_none_or(|c| c == ' ' || is_break(c, libyaml)) {
                self.write_chars(&chars[start..end]);
                start = end;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c, libyaml);
            }
            end += 1;
        }
    }

    fn write_single_quoted(&mut self, text: &str, split: bool) {
        self.write_indicator("'", true, false, false);
        let chars: Vec<char> = text.chars().collect();
        let libyaml = self.libyaml();
        let (mut spaces, mut breaks) = (false, false);
        let (mut start, mut end) = (0, 0);
        while end <= chars.len() {
            let ch = chars.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end
                        && self.column > BEST_WIDTH
                        && split
                        && start != 0
                        && end != chars.len()
                    {
                        self.write_indent();
                    } else {
                        self.write_chars(&chars[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if ch.is_none_or(|c| !is_break(c, libyaml)) {
                    self.write_breaks(&chars[start..end]);
                    start = end;
                }
            } else if ch.is_none_or(|c| c == ' ' || c == '\'' || is_break(c, libyaml))
                && start < end
            {
                self.write_chars(&chars[start..end]);
                start = end;
            }
            if ch == Some('\'') {
                self.write_str("''");
                start = end + 1;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c, libyaml);
            }
            end += 1;
        }
        self.write_indicator("'", false, false, false);
    }

    /// The Python emitter's double quotes: a long line breaks before a space
    /// or after an escape, ending in `\`; a space that starts the next line is
    /// written `\ `.
    fn write_double_quoted_python(&mut self, text: &str, split: bool) {
        self.write_indicator("\"", true, false, false);
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let (mut start, mut end) = (0, 0);
        while end <= len {
            let ch = chars.get(end).copied();
            let escaped = ch.is_none_or(|c| {
                matches!(
                    c,
                    '"' | '\\' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}'
                ) || !(' '..='~').contains(&c)
            });
            if escaped {
                if start < end {
                    self.write_chars(&chars[start..end]);
                    start = end;
                }
                if let Some(c) = ch {
                    self.write_str(&escape(c));
                    start = end + 1;
                }
            }
            // `end - start` is -1 right after an escape.
            let column = self.column as isize + end as isize - start as isize;
            if 0 < end
                && end + 1 < len
                && (ch == Some(' ') || start >= end)
                && column > BEST_WIDTH as isize
                && split
            {
                if start < end {
                    self.write_chars(&chars[start..end]);
                    start = end;
                }
                self.write_str("\\");
                self.write_indent();
                self.whitespace = false;
                self.indention = false;
                if chars.get(start) == Some(&' ') {
                    self.write_str("\\");
                }
            }
            end += 1;
        }
        self.write_indicator("\"", false, false, false);
    }

    /// libyaml's double quotes: a long line breaks at a space, which the
    /// break replaces; a second space after it is written `\ `.
    fn write_double_quoted_libyaml(&mut self, text: &str, split: bool) {
        self.write_indicator("\"", true, false, false);
        let chars: Vec<char> = text.chars().collect();
        let mut spaces = false;
        for (index, &c) in chars.iter().enumerate() {
            if !is_printable(c, true) || c == '"' || c == '\\' || is_break(c, true) {
                self.write_str(&escape(c));
                spaces = false;
            } else if c == ' ' {
                if split
                    && !spaces
                    && self.column > BEST_WIDTH
                    && index != 0
                    && index != chars.len() - 1
                {
                    self.write_indent();
                    if chars.get(index + 1) == Some(&' ') {
                        self.write_str("\\");
                    }
                } else {
                    self.write_str(" ");
                }
                spaces = true;
            } else {
                self.write_chars(&[c]);
                spaces = false;
            }
        }
        self.write_indicator("\"", false, false, false);
    }
}
