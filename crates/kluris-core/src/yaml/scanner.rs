//! Port of PyYAML 6.0.3's `scanner.py` (and the tokens of `tokens.py`): the
//! text becomes tokens exactly where PyYAML's scanner puts their boundaries.
//! The functions follow the Python ones in order and by name.

use std::collections::VecDeque;

use super::parse::{YamlError, marked, py_repr};
use super::reader::{Mark, Reader};

/// `'\0 \t\r\n\x85  '`: the end, a blank or a line break.
fn is_blankz(c: char) -> bool {
    matches!(
        c,
        '\0' | ' ' | '\t' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `'\0 \r\n\x85  '`: like [`is_blankz`] without the tab.
fn is_spacez(c: char) -> bool {
    matches!(
        c,
        '\0' | ' ' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `'\0\r\n\x85  '`: the end or a line break.
fn is_breakz(c: char) -> bool {
    matches!(c, '\0' | '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `'\r\n\x85  '`: a line break.
fn is_break(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveValue {
    None,
    /// `%YAML major.minor`, the numbers as written.
    Yaml(String, String),
    /// `%TAG handle prefix`.
    Tag(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    StreamStart,
    StreamEnd,
    Directive {
        name: String,
        value: DirectiveValue,
    },
    DocumentStart,
    DocumentEnd,
    BlockSequenceStart,
    BlockMappingStart,
    BlockEnd,
    FlowSequenceStart,
    FlowMappingStart,
    FlowSequenceEnd,
    FlowMappingEnd,
    Key,
    Value,
    BlockEntry,
    FlowEntry,
    Alias(String),
    Anchor(String),
    /// `(handle, suffix)`.
    Tag(Option<String>, String),
    Scalar {
        value: String,
        plain: bool,
        style: Option<char>,
        /// A `\u` escape made a lone surrogate, which a Rust string cannot
        /// hold: `value` has U+FFFD in its place.
        surrogate: bool,
    },
}

impl TokenKind {
    /// `Token.id`, as PyYAML prints it in messages.
    pub fn id(&self) -> &'static str {
        match self {
            TokenKind::StreamStart => "<stream start>",
            TokenKind::StreamEnd => "<stream end>",
            TokenKind::Directive { .. } => "<directive>",
            TokenKind::DocumentStart => "<document start>",
            TokenKind::DocumentEnd => "<document end>",
            TokenKind::BlockSequenceStart => "<block sequence start>",
            TokenKind::BlockMappingStart => "<block mapping start>",
            TokenKind::BlockEnd => "<block end>",
            TokenKind::FlowSequenceStart => "[",
            TokenKind::FlowMappingStart => "{",
            TokenKind::FlowSequenceEnd => "]",
            TokenKind::FlowMappingEnd => "}",
            TokenKind::Key => "?",
            TokenKind::Value => ":",
            TokenKind::BlockEntry => "-",
            TokenKind::FlowEntry => ",",
            TokenKind::Alias(_) => "<alias>",
            TokenKind::Anchor(_) => "<anchor>",
            TokenKind::Tag(..) => "<tag>",
            TokenKind::Scalar { .. } => "<scalar>",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: Mark,
    pub end: Mark,
}

/// `possible_simple_keys`: a dict from flow level to key, in insertion order
/// like a Python dict.
#[derive(Default)]
struct SimpleKeys(Vec<(isize, SimpleKey)>);

impl SimpleKeys {
    fn get(&self, level: &isize) -> Option<&SimpleKey> {
        self.0.iter().find(|(l, _)| l == level).map(|(_, key)| key)
    }

    fn remove(&mut self, level: &isize) -> Option<SimpleKey> {
        let at = self.0.iter().position(|(l, _)| l == level)?;
        Some(self.0.remove(at).1)
    }

    fn insert(&mut self, level: isize, key: SimpleKey) {
        match self.0.iter_mut().find(|(l, _)| *l == level) {
            Some(entry) => entry.1 = key,
            None => self.0.push((level, key)),
        }
    }
}

struct SimpleKey {
    token_number: usize,
    required: bool,
    index: usize,
    line: usize,
    column: usize,
    mark: Mark,
}

type Scan<T> = Result<T, YamlError>;

fn error(
    context: Option<&str>,
    context_mark: Option<Mark>,
    problem: String,
    mark: Mark,
) -> YamlError {
    marked("ScannerError", context, context_mark, &problem, mark)
}

pub struct Scanner {
    reader: Reader,
    done: bool,
    /// Below 0 after an unmatched `]` or `}`, as in Python; it is tested
    /// for being nonzero.
    flow_level: isize,
    tokens: VecDeque<Token>,
    tokens_taken: usize,
    indent: isize,
    indents: Vec<isize>,
    allow_simple_key: bool,
    possible_simple_keys: SimpleKeys,
    /// Set while a quoted scalar is scanned when an escape made a lone
    /// surrogate.
    surrogate: bool,
}

impl Scanner {
    pub fn new(reader: Reader) -> Scanner {
        let mut scanner = Scanner {
            reader,
            done: false,
            flow_level: 0,
            tokens: VecDeque::new(),
            tokens_taken: 0,
            indent: -1,
            indents: Vec::new(),
            allow_simple_key: true,
            possible_simple_keys: SimpleKeys::default(),
            surrogate: false,
        };
        scanner.fetch_stream_start();
        scanner
    }

    fn peek(&self, index: usize) -> char {
        self.reader.peek(index)
    }

    fn ch(&self) -> char {
        self.reader.peek(0)
    }

    fn prefix(&self, length: usize) -> String {
        self.reader.prefix(length)
    }

    fn forward(&mut self, length: usize) {
        self.reader.forward(length);
    }

    fn mark(&self) -> Mark {
        self.reader.mark()
    }

    fn column(&self) -> usize {
        self.reader.column
    }

    // --- public methods ---------------------------------------------------

    /// `peek_token`: the next token, scanning as far as simple keys need.
    pub fn peek_token(&mut self) -> Scan<Option<&Token>> {
        while self.need_more_tokens()? {
            self.fetch_more_tokens()?;
        }
        Ok(self.tokens.front())
    }

    pub fn get_token(&mut self) -> Scan<Option<Token>> {
        while self.need_more_tokens()? {
            self.fetch_more_tokens()?;
        }
        let token = self.tokens.pop_front();
        if token.is_some() {
            self.tokens_taken += 1;
        }
        Ok(token)
    }

    /// `check_token(*choices)`: whether the next token is one of `choices`.
    pub fn check_token(&mut self, choices: &[fn(&TokenKind) -> bool]) -> Scan<bool> {
        Ok(match self.peek_token()? {
            Some(token) => choices.is_empty() || choices.iter().any(|c| c(&token.kind)),
            None => false,
        })
    }

    // --- private methods --------------------------------------------------

    fn need_more_tokens(&mut self) -> Scan<bool> {
        if self.done {
            return Ok(false);
        }
        if self.tokens.is_empty() {
            return Ok(true);
        }
        self.stale_possible_simple_keys()?;
        Ok(self.next_possible_simple_key() == Some(self.tokens_taken))
    }

    fn fetch_more_tokens(&mut self) -> Scan<()> {
        self.scan_to_next_token();
        self.stale_possible_simple_keys()?;
        self.unwind_indent(self.column() as isize);
        let ch = self.ch();
        match ch {
            '\0' => return self.fetch_stream_end(),
            '%' if self.check_directive() => return self.fetch_directive(),
            '-' if self.check_document_start() => {
                return self.fetch_document_indicator(TokenKind::DocumentStart);
            }
            '.' if self.check_document_end() => {
                return self.fetch_document_indicator(TokenKind::DocumentEnd);
            }
            '[' => return self.fetch_flow_collection_start(TokenKind::FlowSequenceStart),
            '{' => return self.fetch_flow_collection_start(TokenKind::FlowMappingStart),
            ']' => return self.fetch_flow_collection_end(TokenKind::FlowSequenceEnd),
            '}' => return self.fetch_flow_collection_end(TokenKind::FlowMappingEnd),
            ',' => return self.fetch_flow_entry(),
            '-' if self.check_block_entry() => return self.fetch_block_entry(),
            '?' if self.check_key() => return self.fetch_key(),
            ':' if self.check_value() => return self.fetch_value(),
            '*' => return self.fetch_anchor(true),
            '&' => return self.fetch_anchor(false),
            '!' => return self.fetch_tag(),
            '|' if self.flow_level == 0 => return self.fetch_block_scalar(false),
            '>' if self.flow_level == 0 => return self.fetch_block_scalar(true),
            '\'' => return self.fetch_flow_scalar(false),
            '"' => return self.fetch_flow_scalar(true),
            _ => {}
        }
        if self.check_plain() {
            return self.fetch_plain();
        }
        Err(error(
            Some("while scanning for the next token"),
            None,
            format!(
                "found character {} that cannot start any token",
                py_repr(ch)
            ),
            self.mark(),
        ))
    }

    // --- simple keys ------------------------------------------------------

    fn next_possible_simple_key(&self) -> Option<usize> {
        self.possible_simple_keys
            .0
            .iter()
            .map(|(_, key)| key.token_number)
            .min()
    }

    fn stale_possible_simple_keys(&mut self) -> Scan<()> {
        let (line, index) = (self.reader.line, self.reader.index());
        let stale = |key: &SimpleKey| key.line != line || index - key.index > 1024;
        if let Some(key) = self
            .possible_simple_keys
            .0
            .iter()
            .map(|(_, key)| key)
            .find(|key| key.required && stale(key))
        {
            return Err(error(
                Some("while scanning a simple key"),
                Some(key.mark),
                "could not find expected ':'".into(),
                self.mark(),
            ));
        }
        self.possible_simple_keys.0.retain(|(_, key)| !stale(key));
        Ok(())
    }

    fn save_possible_simple_key(&mut self) -> Scan<()> {
        let required = self.flow_level == 0 && self.indent == self.column() as isize;
        if self.allow_simple_key {
            self.remove_possible_simple_key()?;
            let token_number = self.tokens_taken + self.tokens.len();
            let key = SimpleKey {
                token_number,
                required,
                index: self.reader.index(),
                line: self.reader.line,
                column: self.column(),
                mark: self.mark(),
            };
            self.possible_simple_keys.insert(self.flow_level, key);
        }
        Ok(())
    }

    fn remove_possible_simple_key(&mut self) -> Scan<()> {
        if let Some(key) = self.possible_simple_keys.get(&self.flow_level) {
            if key.required {
                return Err(error(
                    Some("while scanning a simple key"),
                    Some(key.mark),
                    "could not find expected ':'".into(),
                    self.mark(),
                ));
            }
            self.possible_simple_keys.remove(&self.flow_level);
        }
        Ok(())
    }

    // --- indentation ------------------------------------------------------

    fn unwind_indent(&mut self, column: isize) {
        if self.flow_level != 0 {
            return;
        }
        while self.indent > column {
            let mark = self.mark();
            self.indent = self.indents.pop().unwrap_or(-1);
            self.push(TokenKind::BlockEnd, mark, mark);
        }
    }

    fn add_indent(&mut self, column: usize) -> bool {
        let column = column as isize;
        if self.indent < column {
            self.indents.push(self.indent);
            self.indent = column;
            return true;
        }
        false
    }

    fn push(&mut self, kind: TokenKind, start: Mark, end: Mark) {
        self.tokens.push_back(Token { kind, start, end });
    }

    // --- fetchers ---------------------------------------------------------

    fn fetch_stream_start(&mut self) {
        let mark = self.mark();
        self.push(TokenKind::StreamStart, mark, mark);
    }

    fn fetch_stream_end(&mut self) -> Scan<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        self.possible_simple_keys.0.clear();
        let mark = self.mark();
        self.push(TokenKind::StreamEnd, mark, mark);
        self.done = true;
        Ok(())
    }

    fn fetch_directive(&mut self) -> Scan<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_directive()?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_document_indicator(&mut self, kind: TokenKind) -> Scan<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        let start = self.mark();
        self.forward(3);
        let end = self.mark();
        self.push(kind, start, end);
        Ok(())
    }

    fn fetch_flow_collection_start(&mut self, kind: TokenKind) -> Scan<()> {
        self.save_possible_simple_key()?;
        self.flow_level += 1;
        self.allow_simple_key = true;
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(kind, start, end);
        Ok(())
    }

    fn fetch_flow_collection_end(&mut self, kind: TokenKind) -> Scan<()> {
        self.remove_possible_simple_key()?;
        self.flow_level -= 1;
        self.allow_simple_key = false;
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(kind, start, end);
        Ok(())
    }

    fn fetch_flow_entry(&mut self) -> Scan<()> {
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(TokenKind::FlowEntry, start, end);
        Ok(())
    }

    fn fetch_block_entry(&mut self) -> Scan<()> {
        if self.flow_level == 0 {
            if !self.allow_simple_key {
                return Err(error(
                    None,
                    None,
                    "sequence entries are not allowed here".into(),
                    self.mark(),
                ));
            }
            if self.add_indent(self.column()) {
                let mark = self.mark();
                self.push(TokenKind::BlockSequenceStart, mark, mark);
            }
        }
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(TokenKind::BlockEntry, start, end);
        Ok(())
    }

    fn fetch_key(&mut self) -> Scan<()> {
        if self.flow_level == 0 {
            if !self.allow_simple_key {
                return Err(error(
                    None,
                    None,
                    "mapping keys are not allowed here".into(),
                    self.mark(),
                ));
            }
            if self.add_indent(self.column()) {
                let mark = self.mark();
                self.push(TokenKind::BlockMappingStart, mark, mark);
            }
        }
        self.allow_simple_key = self.flow_level == 0;
        self.remove_possible_simple_key()?;
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(TokenKind::Key, start, end);
        Ok(())
    }

    fn fetch_value(&mut self) -> Scan<()> {
        if let Some(key) = self.possible_simple_keys.remove(&self.flow_level) {
            let at = key.token_number - self.tokens_taken;
            self.tokens.insert(
                at,
                Token {
                    kind: TokenKind::Key,
                    start: key.mark,
                    end: key.mark,
                },
            );
            if self.flow_level == 0 && self.add_indent(key.column) {
                self.tokens.insert(
                    at,
                    Token {
                        kind: TokenKind::BlockMappingStart,
                        start: key.mark,
                        end: key.mark,
                    },
                );
            }
            self.allow_simple_key = false;
        } else {
            if self.flow_level == 0 && !self.allow_simple_key {
                return Err(error(
                    None,
                    None,
                    "mapping values are not allowed here".into(),
                    self.mark(),
                ));
            }
            if self.flow_level == 0 && self.add_indent(self.column()) {
                let mark = self.mark();
                self.push(TokenKind::BlockMappingStart, mark, mark);
            }
            self.allow_simple_key = self.flow_level == 0;
            self.remove_possible_simple_key()?;
        }
        let start = self.mark();
        self.forward(1);
        let end = self.mark();
        self.push(TokenKind::Value, start, end);
        Ok(())
    }

    /// `fetch_alias` (`alias`) and `fetch_anchor`.
    fn fetch_anchor(&mut self, alias: bool) -> Scan<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_anchor(alias)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_tag(&mut self) -> Scan<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_tag()?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_block_scalar(&mut self, folded: bool) -> Scan<()> {
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let token = self.scan_block_scalar(folded)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_flow_scalar(&mut self, double: bool) -> Scan<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_flow_scalar(double)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_plain(&mut self) -> Scan<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_plain()?;
        self.tokens.push_back(token);
        Ok(())
    }

    // --- checkers ---------------------------------------------------------

    fn check_directive(&self) -> bool {
        self.column() == 0
    }

    fn check_document_start(&self) -> bool {
        self.column() == 0 && self.prefix(3) == "---" && is_blankz(self.peek(3))
    }

    fn check_document_end(&self) -> bool {
        self.column() == 0 && self.prefix(3) == "..." && is_blankz(self.peek(3))
    }

    fn check_block_entry(&self) -> bool {
        is_blankz(self.peek(1))
    }

    fn check_key(&self) -> bool {
        self.flow_level != 0 || is_blankz(self.peek(1))
    }

    fn check_value(&self) -> bool {
        self.flow_level != 0 || is_blankz(self.peek(1))
    }

    fn check_plain(&self) -> bool {
        let ch = self.ch();
        let indicator = is_blankz(ch) || "-?:,[]{}#&*!|>'\"%@`".contains(ch);
        !indicator
            || (!is_blankz(self.peek(1))
                && (ch == '-' || (self.flow_level == 0 && (ch == '?' || ch == ':'))))
    }

    // --- scanners ---------------------------------------------------------

    fn scan_to_next_token(&mut self) {
        if self.reader.index() == 0 && self.ch() == '\u{FEFF}' {
            self.forward(1);
        }
        loop {
            while self.ch() == ' ' {
                self.forward(1);
            }
            if self.ch() == '#' {
                while !is_breakz(self.ch()) {
                    self.forward(1);
                }
            }
            if self.scan_line_break().is_empty() {
                break;
            }
            if self.flow_level == 0 {
                self.allow_simple_key = true;
            }
        }
    }

    fn scan_directive(&mut self) -> Scan<Token> {
        let start = self.mark();
        self.forward(1);
        let name = self.scan_directive_name(start)?;
        let value;
        let end;
        if name == "YAML" {
            value = self.scan_yaml_directive_value(start)?;
            end = self.mark();
        } else if name == "TAG" {
            value = self.scan_tag_directive_value(start)?;
            end = self.mark();
        } else {
            value = DirectiveValue::None;
            end = self.mark();
            while !is_breakz(self.ch()) {
                self.forward(1);
            }
        }
        self.scan_directive_ignored_line(start)?;
        Ok(Token {
            kind: TokenKind::Directive { name, value },
            start,
            end,
        })
    }

    fn scan_directive_name(&mut self, start: Mark) -> Scan<String> {
        let mut length = 0;
        while is_word(self.peek(length)) {
            length += 1;
        }
        let ch = self.peek(length);
        if length == 0 {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        let value = self.prefix(length);
        self.forward(length);
        let ch = self.ch();
        if !is_spacez(ch) {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        Ok(value)
    }

    fn scan_yaml_directive_value(&mut self, start: Mark) -> Scan<DirectiveValue> {
        while self.ch() == ' ' {
            self.forward(1);
        }
        let major = self.scan_yaml_directive_number(start)?;
        if self.ch() != '.' {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!("expected a digit or '.', but found {}", py_repr(self.ch())),
                self.mark(),
            ));
        }
        self.forward(1);
        let minor = self.scan_yaml_directive_number(start)?;
        if !is_spacez(self.ch()) {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!("expected a digit or ' ', but found {}", py_repr(self.ch())),
                self.mark(),
            ));
        }
        Ok(DirectiveValue::Yaml(major, minor))
    }

    /// The digits of a version number, without leading zeros (`int()`).
    fn scan_yaml_directive_number(&mut self, start: Mark) -> Scan<String> {
        let ch = self.ch();
        if !ch.is_ascii_digit() {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!("expected a digit, but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        let mut length = 0;
        while self.peek(length).is_ascii_digit() {
            length += 1;
        }
        let digits = self.prefix(length);
        self.forward(length);
        let trimmed = digits.trim_start_matches('0');
        Ok(if trimmed.is_empty() {
            "0".into()
        } else {
            trimmed.into()
        })
    }

    fn scan_tag_directive_value(&mut self, start: Mark) -> Scan<DirectiveValue> {
        while self.ch() == ' ' {
            self.forward(1);
        }
        let handle = self.scan_tag_directive_handle(start)?;
        while self.ch() == ' ' {
            self.forward(1);
        }
        let prefix = self.scan_tag_directive_prefix(start)?;
        Ok(DirectiveValue::Tag(handle, prefix))
    }

    fn scan_tag_directive_handle(&mut self, start: Mark) -> Scan<String> {
        let value = self.scan_tag_handle("directive", start)?;
        let ch = self.ch();
        if ch != ' ' {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!("expected ' ', but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        Ok(value)
    }

    fn scan_tag_directive_prefix(&mut self, start: Mark) -> Scan<String> {
        let value = self.scan_tag_uri("directive", start)?;
        let ch = self.ch();
        if !is_spacez(ch) {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!("expected ' ', but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        Ok(value)
    }

    fn scan_directive_ignored_line(&mut self, start: Mark) -> Scan<()> {
        while self.ch() == ' ' {
            self.forward(1);
        }
        if self.ch() == '#' {
            while !is_breakz(self.ch()) {
                self.forward(1);
            }
        }
        let ch = self.ch();
        if !is_breakz(ch) {
            return Err(error(
                Some("while scanning a directive"),
                Some(start),
                format!(
                    "expected a comment or a line break, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        self.scan_line_break();
        Ok(())
    }

    fn scan_anchor(&mut self, alias: bool) -> Scan<Token> {
        let start = self.mark();
        let name = if alias { "alias" } else { "anchor" };
        self.forward(1);
        let mut length = 0;
        while is_word(self.peek(length)) {
            length += 1;
        }
        if length == 0 {
            return Err(error(
                Some(&format!("while scanning an {name}")),
                Some(start),
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    py_repr(self.peek(length))
                ),
                self.mark(),
            ));
        }
        let value = self.prefix(length);
        self.forward(length);
        let ch = self.ch();
        if !(is_blankz(ch) || "?:,]}%@`".contains(ch)) {
            return Err(error(
                Some(&format!("while scanning an {name}")),
                Some(start),
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        let end = self.mark();
        let kind = if alias {
            TokenKind::Alias(value)
        } else {
            TokenKind::Anchor(value)
        };
        Ok(Token { kind, start, end })
    }

    fn scan_tag(&mut self) -> Scan<Token> {
        let start = self.mark();
        let mut ch = self.peek(1);
        let handle;
        let suffix;
        if ch == '<' {
            handle = None;
            self.forward(2);
            suffix = self.scan_tag_uri("tag", start)?;
            if self.ch() != '>' {
                return Err(error(
                    Some("while parsing a tag"),
                    Some(start),
                    format!("expected '>', but found {}", py_repr(self.ch())),
                    self.mark(),
                ));
            }
            self.forward(1);
        } else if is_blankz(ch) {
            handle = None;
            suffix = "!".to_string();
            self.forward(1);
        } else {
            let mut length = 1;
            let mut use_handle = false;
            while !is_spacez(ch) {
                if ch == '!' {
                    use_handle = true;
                    break;
                }
                length += 1;
                ch = self.peek(length);
            }
            if use_handle {
                handle = Some(self.scan_tag_handle("tag", start)?);
            } else {
                handle = Some("!".to_string());
                self.forward(1);
            }
            suffix = self.scan_tag_uri("tag", start)?;
        }
        let ch = self.ch();
        if !is_spacez(ch) {
            return Err(error(
                Some("while scanning a tag"),
                Some(start),
                format!("expected ' ', but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        let end = self.mark();
        Ok(Token {
            kind: TokenKind::Tag(handle, suffix),
            start,
            end,
        })
    }

    fn scan_block_scalar(&mut self, folded: bool) -> Scan<Token> {
        let mut chunks = String::new();
        let start = self.mark();
        self.forward(1);
        let (chomping, increment) = self.scan_block_scalar_indicators(start)?;
        self.scan_block_scalar_ignored_line(start)?;
        let min_indent = (self.indent + 1).max(1) as usize;
        let indent;
        let mut breaks;
        let mut end;
        match increment {
            None => {
                let (b, max_indent, e) = self.scan_block_scalar_indentation();
                breaks = b;
                end = e;
                indent = min_indent.max(max_indent);
            }
            Some(increment) => {
                indent = min_indent + increment - 1;
                let (b, e) = self.scan_block_scalar_breaks(indent);
                breaks = b;
                end = e;
            }
        }
        let mut line_break = String::new();
        while self.column() == indent && self.ch() != '\0' {
            chunks.push_str(&breaks.concat());
            let leading_non_space = !matches!(self.ch(), ' ' | '\t');
            let mut length = 0;
            while !is_breakz(self.peek(length)) {
                length += 1;
            }
            chunks.push_str(&self.prefix(length));
            self.forward(length);
            line_break = self.scan_line_break();
            let (b, e) = self.scan_block_scalar_breaks(indent);
            breaks = b;
            end = e;
            if self.column() == indent && self.ch() != '\0' {
                if folded
                    && line_break == "\n"
                    && leading_non_space
                    && !matches!(self.ch(), ' ' | '\t')
                {
                    if breaks.is_empty() {
                        chunks.push(' ');
                    }
                } else {
                    chunks.push_str(&line_break);
                }
            } else {
                break;
            }
        }
        if chomping != Some(false) {
            chunks.push_str(&line_break);
        }
        if chomping == Some(true) {
            chunks.push_str(&breaks.concat());
        }
        Ok(Token {
            kind: TokenKind::Scalar {
                value: chunks,
                plain: false,
                style: Some(if folded { '>' } else { '|' }),
                surrogate: false,
            },
            start,
            end,
        })
    }

    fn scan_block_scalar_indicators(&mut self, start: Mark) -> Scan<(Option<bool>, Option<usize>)> {
        let mut chomping = None;
        let mut increment = None;
        let zero = |scanner: &Scanner| {
            error(
                Some("while scanning a block scalar"),
                Some(start),
                "expected indentation indicator in the range 1-9, but found 0".into(),
                scanner.mark(),
            )
        };
        let mut ch = self.ch();
        if ch == '+' || ch == '-' {
            chomping = Some(ch == '+');
            self.forward(1);
            ch = self.ch();
            if let Some(digit) = ch.to_digit(10) {
                if digit == 0 {
                    return Err(zero(self));
                }
                increment = Some(digit as usize);
                self.forward(1);
            }
        } else if let Some(digit) = ch.to_digit(10) {
            if digit == 0 {
                return Err(zero(self));
            }
            increment = Some(digit as usize);
            self.forward(1);
            ch = self.ch();
            if ch == '+' || ch == '-' {
                chomping = Some(ch == '+');
                self.forward(1);
            }
        }
        let ch = self.ch();
        if !is_spacez(ch) {
            return Err(error(
                Some("while scanning a block scalar"),
                Some(start),
                format!(
                    "expected chomping or indentation indicators, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        Ok((chomping, increment))
    }

    fn scan_block_scalar_ignored_line(&mut self, start: Mark) -> Scan<()> {
        while self.ch() == ' ' {
            self.forward(1);
        }
        if self.ch() == '#' {
            while !is_breakz(self.ch()) {
                self.forward(1);
            }
        }
        let ch = self.ch();
        if !is_breakz(ch) {
            return Err(error(
                Some("while scanning a block scalar"),
                Some(start),
                format!(
                    "expected a comment or a line break, but found {}",
                    py_repr(ch)
                ),
                self.mark(),
            ));
        }
        self.scan_line_break();
        Ok(())
    }

    fn scan_block_scalar_indentation(&mut self) -> (Vec<String>, usize, Mark) {
        let mut chunks = Vec::new();
        let mut max_indent = 0;
        let mut end = self.mark();
        while self.ch() == ' ' || is_break(self.ch()) {
            if self.ch() != ' ' {
                chunks.push(self.scan_line_break());
                end = self.mark();
            } else {
                self.forward(1);
                if self.column() > max_indent {
                    max_indent = self.column();
                }
            }
        }
        (chunks, max_indent, end)
    }

    fn scan_block_scalar_breaks(&mut self, indent: usize) -> (Vec<String>, Mark) {
        let mut chunks = Vec::new();
        let mut end = self.mark();
        while self.column() < indent && self.ch() == ' ' {
            self.forward(1);
        }
        while is_break(self.ch()) {
            chunks.push(self.scan_line_break());
            end = self.mark();
            while self.column() < indent && self.ch() == ' ' {
                self.forward(1);
            }
        }
        (chunks, end)
    }

    fn scan_flow_scalar(&mut self, double: bool) -> Scan<Token> {
        let mut chunks = String::new();
        let start = self.mark();
        let quote = self.ch();
        self.surrogate = false;
        self.forward(1);
        self.scan_flow_scalar_non_spaces(double, start, &mut chunks)?;
        while self.ch() != quote {
            self.scan_flow_scalar_spaces(double, start, &mut chunks)?;
            self.scan_flow_scalar_non_spaces(double, start, &mut chunks)?;
        }
        self.forward(1);
        let end = self.mark();
        Ok(Token {
            kind: TokenKind::Scalar {
                value: chunks,
                plain: false,
                style: Some(if double { '"' } else { '\'' }),
                surrogate: std::mem::take(&mut self.surrogate),
            },
            start,
            end,
        })
    }

    fn escape_replacement(ch: char) -> Option<char> {
        Some(match ch {
            '0' => '\0',
            'a' => '\u{7}',
            'b' => '\u{8}',
            't' | '\t' => '\t',
            'n' => '\n',
            'v' => '\u{B}',
            'f' => '\u{C}',
            'r' => '\r',
            'e' => '\u{1B}',
            ' ' => ' ',
            '"' => '"',
            '\\' => '\\',
            '/' => '/',
            'N' => '\u{85}',
            '_' => '\u{A0}',
            'L' => '\u{2028}',
            'P' => '\u{2029}',
            _ => return None,
        })
    }

    fn scan_flow_scalar_non_spaces(
        &mut self,
        double: bool,
        start: Mark,
        chunks: &mut String,
    ) -> Scan<()> {
        loop {
            let mut length = 0;
            while !matches!(self.peek(length), '\'' | '"' | '\\') && !is_blankz(self.peek(length)) {
                length += 1;
            }
            if length > 0 {
                chunks.push_str(&self.prefix(length));
                self.forward(length);
            }
            let ch = self.ch();
            if !double && ch == '\'' && self.peek(1) == '\'' {
                chunks.push('\'');
                self.forward(2);
            } else if (double && ch == '\'') || (!double && (ch == '"' || ch == '\\')) {
                chunks.push(ch);
                self.forward(1);
            } else if double && ch == '\\' {
                self.forward(1);
                let ch = self.ch();
                if let Some(replacement) = Self::escape_replacement(ch) {
                    chunks.push(replacement);
                    self.forward(1);
                } else if let Some(length) = match ch {
                    'x' => Some(2),
                    'u' => Some(4),
                    'U' => Some(8),
                    _ => None,
                } {
                    self.forward(1);
                    for k in 0..length {
                        if !is_hex(self.peek(k)) {
                            return Err(error(
                                Some("while scanning a double-quoted scalar"),
                                Some(start),
                                format!(
                                    "expected escape sequence of {length} hexadecimal numbers, but found {}",
                                    py_repr(self.peek(k))
                                ),
                                self.mark(),
                            ));
                        }
                    }
                    let code = u32::from_str_radix(&self.prefix(length), 16).unwrap_or(u32::MAX);
                    match char::from_u32(code) {
                        Some(c) => chunks.push(c),
                        // `chr()` of a surrogate: Python keeps it.
                        None if (0xD800..=0xDFFF).contains(&code) => {
                            self.surrogate = true;
                            chunks.push('\u{FFFD}');
                        }
                        // `chr()` past U+10FFFF raises ValueError.
                        None => {
                            return Err(error(
                                Some("while scanning a double-quoted scalar"),
                                Some(start),
                                format!("chr() arg not in range(0x110000): {code:#x}"),
                                self.mark(),
                            ));
                        }
                    }
                    self.forward(length);
                } else if is_break(ch) {
                    self.scan_line_break();
                    self.scan_flow_scalar_breaks(start, chunks)?;
                } else {
                    return Err(error(
                        Some("while scanning a double-quoted scalar"),
                        Some(start),
                        format!("found unknown escape character {}", py_repr(ch)),
                        self.mark(),
                    ));
                }
            } else {
                return Ok(());
            }
        }
    }

    fn scan_flow_scalar_spaces(
        &mut self,
        _double: bool,
        start: Mark,
        chunks: &mut String,
    ) -> Scan<()> {
        let mut length = 0;
        while matches!(self.peek(length), ' ' | '\t') {
            length += 1;
        }
        let whitespaces = self.prefix(length);
        self.forward(length);
        let ch = self.ch();
        if ch == '\0' {
            return Err(error(
                Some("while scanning a quoted scalar"),
                Some(start),
                "found unexpected end of stream".into(),
                self.mark(),
            ));
        } else if is_break(ch) {
            let line_break = self.scan_line_break();
            let mut breaks = String::new();
            self.scan_flow_scalar_breaks(start, &mut breaks)?;
            if line_break != "\n" {
                chunks.push_str(&line_break);
            } else if breaks.is_empty() {
                chunks.push(' ');
            }
            chunks.push_str(&breaks);
        } else {
            chunks.push_str(&whitespaces);
        }
        Ok(())
    }

    fn scan_flow_scalar_breaks(&mut self, start: Mark, chunks: &mut String) -> Scan<()> {
        loop {
            let prefix = self.prefix(3);
            if (prefix == "---" || prefix == "...") && is_blankz(self.peek(3)) {
                return Err(error(
                    Some("while scanning a quoted scalar"),
                    Some(start),
                    "found unexpected document separator".into(),
                    self.mark(),
                ));
            }
            while matches!(self.ch(), ' ' | '\t') {
                self.forward(1);
            }
            if is_break(self.ch()) {
                let line_break = self.scan_line_break();
                chunks.push_str(&line_break);
            } else {
                return Ok(());
            }
        }
    }

    fn scan_plain(&mut self) -> Scan<Token> {
        let mut chunks = String::new();
        let start = self.mark();
        let mut end = start;
        let indent = self.indent + 1;
        let mut spaces = Some(String::new());
        loop {
            let mut length = 0;
            if self.ch() == '#' {
                break;
            }
            loop {
                let ch = self.peek(length);
                if is_blankz(ch)
                    || (ch == ':'
                        && (is_blankz(self.peek(length + 1))
                            || (self.flow_level != 0 && ",[]{}".contains(self.peek(length + 1)))))
                    || (self.flow_level != 0 && ",?[]{}".contains(ch))
                {
                    break;
                }
                length += 1;
            }
            if length == 0 {
                break;
            }
            self.allow_simple_key = false;
            if let Some(spaces) = &spaces {
                chunks.push_str(spaces);
            }
            chunks.push_str(&self.prefix(length));
            self.forward(length);
            end = self.mark();
            spaces = self.scan_plain_spaces();
            match &spaces {
                Some(s) if !s.is_empty() => {}
                _ => break,
            }
            if self.ch() == '#' || (self.flow_level == 0 && (self.column() as isize) < indent) {
                break;
            }
        }
        Ok(Token {
            kind: TokenKind::Scalar {
                value: chunks,
                plain: true,
                style: None,
                surrogate: false,
            },
            start,
            end,
        })
    }

    /// `scan_plain_spaces`: `None` where Python returns nothing (a document
    /// marker follows the line break).
    fn scan_plain_spaces(&mut self) -> Option<String> {
        let mut chunks = String::new();
        let mut length = 0;
        while self.peek(length) == ' ' {
            length += 1;
        }
        let whitespaces = self.prefix(length);
        self.forward(length);
        let ch = self.ch();
        if is_break(ch) {
            let line_break = self.scan_line_break();
            self.allow_simple_key = true;
            let prefix = self.prefix(3);
            if (prefix == "---" || prefix == "...") && is_blankz(self.peek(3)) {
                return None;
            }
            let mut breaks = String::new();
            while self.ch() == ' ' || is_break(self.ch()) {
                if self.ch() == ' ' {
                    self.forward(1);
                } else {
                    breaks.push_str(&self.scan_line_break());
                    let prefix = self.prefix(3);
                    if (prefix == "---" || prefix == "...") && is_blankz(self.peek(3)) {
                        return None;
                    }
                }
            }
            if line_break != "\n" {
                chunks.push_str(&line_break);
            } else if breaks.is_empty() {
                chunks.push(' ');
            }
            chunks.push_str(&breaks);
        } else if !whitespaces.is_empty() {
            chunks.push_str(&whitespaces);
        }
        Some(chunks)
    }

    fn scan_tag_handle(&mut self, name: &str, start: Mark) -> Scan<String> {
        let ch = self.ch();
        if ch != '!' {
            return Err(error(
                Some(&format!("while scanning a {name}")),
                Some(start),
                format!("expected '!', but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        let mut length = 1;
        let mut ch = self.peek(length);
        if ch != ' ' {
            while is_word(ch) {
                length += 1;
                ch = self.peek(length);
            }
            if ch != '!' {
                self.forward(length);
                return Err(error(
                    Some(&format!("while scanning a {name}")),
                    Some(start),
                    format!("expected '!', but found {}", py_repr(ch)),
                    self.mark(),
                ));
            }
            length += 1;
        }
        let value = self.prefix(length);
        self.forward(length);
        Ok(value)
    }

    fn scan_tag_uri(&mut self, name: &str, start: Mark) -> Scan<String> {
        let mut chunks = String::new();
        let mut length = 0;
        let mut ch = self.peek(length);
        while ch.is_ascii_alphanumeric() || "-;/?:@&=+$,_.!~*'()[]%".contains(ch) {
            if ch == '%' {
                chunks.push_str(&self.prefix(length));
                self.forward(length);
                length = 0;
                chunks.push_str(&self.scan_uri_escapes(name, start)?);
            } else {
                length += 1;
            }
            ch = self.peek(length);
        }
        if length > 0 {
            chunks.push_str(&self.prefix(length));
            self.forward(length);
        }
        if chunks.is_empty() {
            return Err(error(
                Some(&format!("while parsing a {name}")),
                Some(start),
                format!("expected URI, but found {}", py_repr(ch)),
                self.mark(),
            ));
        }
        Ok(chunks)
    }

    fn scan_uri_escapes(&mut self, name: &str, start: Mark) -> Scan<String> {
        let mut codes = Vec::new();
        let mark = self.mark();
        while self.ch() == '%' {
            self.forward(1);
            for k in 0..2 {
                if !is_hex(self.peek(k)) {
                    return Err(error(
                        Some(&format!("while scanning a {name}")),
                        Some(start),
                        format!(
                            "expected URI escape sequence of 2 hexadecimal numbers, but found {}",
                            py_repr(self.peek(k))
                        ),
                        self.mark(),
                    ));
                }
            }
            codes.push(u8::from_str_radix(&self.prefix(2), 16).unwrap_or(0));
            self.forward(2);
        }
        String::from_utf8(codes).map_err(|e| {
            error(
                Some(&format!("while scanning a {name}")),
                Some(start),
                e.to_string(),
                mark,
            )
        })
    }

    fn scan_line_break(&mut self) -> String {
        let ch = self.ch();
        if matches!(ch, '\r' | '\n' | '\u{85}') {
            if self.prefix(2) == "\r\n" {
                self.forward(2);
            } else {
                self.forward(1);
            }
            return "\n".into();
        }
        if matches!(ch, '\u{2028}' | '\u{2029}') {
            self.forward(1);
            return ch.to_string();
        }
        String::new()
    }
}
