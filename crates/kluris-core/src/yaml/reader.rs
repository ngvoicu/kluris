//! Ported from PyYAML 6.0.3 `reader.py`; see LICENSE-PyYAML.
//!
//! Port of PyYAML 6.0.3's `reader.py` for `str` input: the characters of the
//! text plus a final `'\0'`, the printable-character check, and the position
//! (`index`, `line`, `column`) as PyYAML counts it.

use super::parse::YamlError;

/// A position in the text, 0-based like PyYAML's `Mark`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mark {
    pub index: usize,
    pub line: usize,
    pub column: usize,
}

pub struct Reader {
    buffer: Vec<char>,
    pointer: usize,
    pub line: usize,
    pub column: usize,
}

/// `Reader.NON_PRINTABLE`: everything outside
/// `[\x09\x0A\x0D\x20-\x7E\x85\xA0-퟿-�\U00010000-\U0010ffff]`.
fn is_printable(c: char) -> bool {
    matches!(c,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{7E}' | '\u{85}'
        | '\u{A0}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

impl Reader {
    /// `Reader(stream)` for a `str`: `check_printable`, then the buffer with
    /// a final `'\0'`.
    pub fn new(text: &str) -> Result<Reader, YamlError> {
        let mut buffer: Vec<char> = text.chars().collect();
        if let Some(position) = buffer.iter().position(|&c| !is_printable(c)) {
            let character = u32::from(buffer[position]);
            // PyYAML's ReaderError gives the position only; the line and
            // column are counted the way the reader counts them.
            let mut reader = Reader {
                buffer: buffer.clone(),
                pointer: 0,
                line: 0,
                column: 0,
            };
            reader.buffer.push('\0');
            reader.forward(position);
            return Err(YamlError::new(
                format!(
                    "ReaderError: unacceptable character #x{character:04x}: special characters are not allowed (position {position})"
                ),
                reader.mark(),
            ));
        }
        buffer.push('\0');
        Ok(Reader {
            buffer,
            pointer: 0,
            line: 0,
            column: 0,
        })
    }

    /// The character `index` places ahead; `'\0'` past the end.
    pub fn peek(&self, index: usize) -> char {
        self.buffer
            .get(self.pointer + index)
            .copied()
            .unwrap_or('\0')
    }

    /// The next `length` characters (fewer at the end).
    pub fn prefix(&self, length: usize) -> String {
        let end = (self.pointer + length).min(self.buffer.len());
        self.buffer[self.pointer.min(end)..end].iter().collect()
    }

    pub fn forward(&mut self, length: usize) {
        for _ in 0..length {
            let Some(&ch) = self.buffer.get(self.pointer) else {
                return;
            };
            self.pointer += 1;
            let next = self.buffer.get(self.pointer).copied().unwrap_or('\0');
            if matches!(ch, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
                || (ch == '\r' && next != '\n')
            {
                self.line += 1;
                self.column = 0;
            } else if ch != '\u{FEFF}' {
                self.column += 1;
            }
        }
    }

    pub fn index(&self) -> usize {
        self.pointer
    }

    pub fn mark(&self) -> Mark {
        Mark {
            index: self.pointer,
            line: self.line,
            column: self.column,
        }
    }
}
