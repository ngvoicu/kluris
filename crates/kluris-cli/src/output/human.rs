//! Human-readable output: v2's wording, not Rich's layout.
//!
//! - Styles are applied by code, around text: user text (names, paths,
//!   descriptions, commit subjects, snippets) is printed as it is, and a
//!   `[bold]` or `[/x]` in it is never markup (v2 handed it to Rich, which
//!   rendered it or raised `MarkupError`).
//! - Colour and bold appear only when stdout is a terminal that takes them:
//!   never with `NO_COLOR` set (non-empty), when stdout is not a terminal, or
//!   with `TERM=dumb`; on Windows only in a terminal known to read ANSI
//!   (Windows Terminal, ConEmu, a `TERM`-setting shell). `CLICOLOR_FORCE`
//!   (non-empty, not `0`) forces them, `NO_COLOR` still wins.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};

/// A text style (v2's Rich styles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "the commands still to port use the colours")]
pub enum Style {
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Bold => "1",
            Style::Dim => "2",
            Style::Red => "31",
            Style::Green => "32",
            Style::Yellow => "33",
        }
    }
}

/// Whether styles are written as ANSI escapes or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Painter {
    ansi: bool,
}

impl Painter {
    /// The painter for stdout, from the environment.
    pub fn stdout() -> Painter {
        Painter {
            ansi: ansi_enabled(io::stdout().is_terminal(), |key| std::env::var_os(key)),
        }
    }

    /// A painter that never writes escapes.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "used by tests and later commands")
    )]
    pub fn plain() -> Painter {
        Painter { ansi: false }
    }

    /// `text` in `style`.
    pub fn paint(self, style: Style, text: &str) -> String {
        if self.ansi && !text.is_empty() {
            format!("\x1b[{}m{text}\x1b[0m", style.code())
        } else {
            text.to_string()
        }
    }

    pub fn bold(self, text: &str) -> String {
        self.paint(Style::Bold, text)
    }
}

/// The colour decision (see the module docs); `var` reads the environment.
pub fn ansi_enabled(is_terminal: bool, var: impl Fn(&str) -> Option<OsString>) -> bool {
    let set = |key: &str| var(key).is_some_and(|v| !v.is_empty());
    if set("NO_COLOR") {
        return false;
    }
    if var("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0") {
        return true;
    }
    if !is_terminal || var("TERM").is_some_and(|t| t == "dumb") {
        return false;
    }
    !cfg!(windows)
        || set("WT_SESSION")
        || set("TERM")
        || var("ConEmuANSI").is_some_and(|v| v == "ON")
}

/// Write `text` to stdout as it is (a closed pipe is not an error).
pub fn print(text: &str) {
    let _ = io::stdout().write_all(text.as_bytes());
}

/// A plain-text table (v2's Rich tables, without the boxes): `title`, the
/// header row in bold, a rule and the rows, columns padded to the widest cell
/// (in characters) and two spaces apart.
pub fn table(painter: Painter, title: &str, headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>, style: Option<Style>| {
        let last = cells.len().saturating_sub(1);
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (cell, width))| {
                let pad = if i == last {
                    String::new()
                } else {
                    " ".repeat(width - cell.chars().count())
                };
                match style {
                    Some(style) => format!("{}{pad}", painter.paint(style, cell)),
                    None => format!("{cell}{pad}"),
                }
            })
            .collect();
        format!("{}\n", padded.join("  "))
    };
    let mut out = format!("{title}\n");
    out.push_str(&line(
        headers.iter().map(|h| h.to_string()).collect(),
        Some(Style::Bold),
    ));
    out.push_str(&line(widths.iter().map(|w| "-".repeat(*w)).collect(), None));
    for row in rows {
        out.push_str(&line(row.clone(), None));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn pads_columns_to_the_widest_cell() {
        let rows = vec![vec!["a".to_string(), "Grüße".to_string()]];
        assert_eq!(
            table(Painter::plain(), "T", &["Name", "X"], &rows),
            "T\nName  X\n----  -----\na     Grüße\n"
        );
    }

    #[test]
    fn the_last_column_is_not_padded() {
        let rows = vec![vec!["a".to_string(), "".to_string()]];
        assert_eq!(
            table(Painter::plain(), "T", &["N", "Long"], &rows),
            "T\nN  Long\n-  ----\na  \n"
        );
    }

    #[test]
    fn header_is_bold_and_padding_stays_outside_the_escapes() {
        let painter = Painter { ansi: true };
        let rows = vec![vec!["abc".to_string(), "x".to_string()]];
        assert_eq!(
            table(painter, "T", &["N", "H"], &rows),
            "T\n\x1b[1mN\x1b[0m    \x1b[1mH\x1b[0m\n---  -\nabc  x\n"
        );
    }

    #[test]
    fn user_text_is_never_markup() {
        let text = "fix [bold]this[/bold] and [/oops]";
        assert_eq!(Painter::plain().bold(text), text);
        assert_eq!(
            Painter { ansi: true }.bold(text),
            format!("\x1b[1m{text}\x1b[0m")
        );
    }

    #[test]
    fn colour_needs_a_terminal() {
        assert!(!ansi_enabled(false, env(&[("TERM", "xterm")])));
        let terminal = ansi_enabled(true, env(&[("TERM", "xterm")]));
        assert!(terminal, "a terminal with TERM takes colour on every OS");
    }

    #[test]
    fn no_color_and_dumb_terminals_turn_it_off() {
        assert!(!ansi_enabled(
            true,
            env(&[("NO_COLOR", "1"), ("TERM", "xterm")])
        ));
        assert!(!ansi_enabled(true, env(&[("TERM", "dumb")])));
        assert!(
            ansi_enabled(true, env(&[("NO_COLOR", ""), ("TERM", "xterm")])),
            "an empty NO_COLOR is unset (no-color.org)"
        );
    }

    #[test]
    fn clicolor_force_forces_it_but_no_color_wins() {
        assert!(ansi_enabled(false, env(&[("CLICOLOR_FORCE", "1")])));
        assert!(!ansi_enabled(false, env(&[("CLICOLOR_FORCE", "0")])));
        assert!(!ansi_enabled(
            false,
            env(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")])
        ));
    }

    #[test]
    fn windows_needs_a_terminal_known_to_read_ansi() {
        let bare = ansi_enabled(true, env(&[]));
        assert_eq!(bare, !cfg!(windows));
        assert!(ansi_enabled(true, env(&[("WT_SESSION", "x")])));
        assert!(ansi_enabled(true, env(&[("ConEmuANSI", "ON")])));
    }
}
