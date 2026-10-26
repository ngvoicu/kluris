//! The brain resolver every `--brain` command uses: v2's `_resolve_brains`,
//! `_check_brain_paths` and `_pick_brain_interactively` (research-03 §1.0,
//! `cli.py:268-395`), with v2's exact texts.
//!
//! The cases, in v2's order:
//! 1. `--brain all`: refused unless the command fans out (`allow_all`), an
//!    error with no brains, else every brain in config order;
//! 2. `--brain NAME` that is not registered: an error;
//! 3. no brains: an error;
//! 4. exactly one brain: that brain;
//! 5. two or more and not interactive (`--json`, `KLURIS_NO_PROMPT=1`, stdin
//!    not a terminal; the caller decides): an error listing them;
//! 6. otherwise the [`Picker`].
//!
//! Every brain that comes out is path-checked ([`check_brain_paths`]). An
//! empty `--brain ""` is no `--brain` (Click's `if brain_name:`).

use std::fmt;
use std::io::{BufRead, Write};
use std::path::Path;

use crate::config::{BrainEntry, GlobalConfig};
use crate::pyrepr::{py_path_str, py_repr_str};
use crate::yaml::py_int_from_str;

/// A brain the resolver picked, with its config entry unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBrain {
    pub name: String,
    pub entry: BrainEntry,
}

/// What a command asks the resolver for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    /// The `--brain` value.
    pub brain: Option<&'a str>,
    /// Whether `--brain all` and the picker's `all` are offered (dream,
    /// status, mri and the companion commands).
    pub allow_all: bool,
    /// `--json`, `KLURIS_NO_PROMPT=1` or stdin not a terminal.
    pub non_interactive: bool,
}

/// The brains to work on, or the picker to ask with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Brains(Vec<ResolvedBrain>),
    Pick(Picker),
}

/// Why no brain was resolved; `Display` is v2's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    AllNotSupported,
    NoBrains,
    UnknownBrain(String),
    MultipleBrains {
        names: Vec<String>,
        allow_all: bool,
    },
    /// `(name, path as stored)` of each brain whose path is missing or not a
    /// directory.
    MissingPaths(Vec<(String, String)>),
    /// End of input or Ctrl-C at the picker: Click's `Abort`.
    Aborted,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::AllNotSupported => f.write_str(
                "--brain all is only supported on dream, status, mri, and companion commands.",
            ),
            ResolveError::NoBrains => {
                f.write_str("No brains registered. Run 'kluris create <name>' to create one.")
            }
            ResolveError::UnknownBrain(name) => write!(
                f,
                "No brain named '{name}' is registered. Run 'kluris list' to see available brains."
            ),
            ResolveError::MultipleBrains { names, allow_all } => write!(
                f,
                "Multiple brains registered. Pass --brain NAME{}. Available: {}",
                if *allow_all { " or 'all'" } else { "" },
                names.join(", ")
            ),
            ResolveError::MissingPaths(rows) => {
                f.write_str("The following registered brain(s) have missing or invalid paths:\n")?;
                for (name, path) in rows {
                    writeln!(f, "  - {name}: {path}")?;
                }
                f.write_str("Run `kluris remove <name>` or fix the path in the global config.")
            }
            ResolveError::Aborted => f.write_str("Aborted!"),
        }
    }
}

impl std::error::Error for ResolveError {}

fn all_brains(config: &GlobalConfig) -> Vec<ResolvedBrain> {
    config
        .brains
        .iter()
        .map(|(name, entry)| ResolvedBrain {
            name: name.clone(),
            entry: entry.clone(),
        })
        .collect()
}

fn checked(brains: Vec<ResolvedBrain>) -> Result<Resolution, ResolveError> {
    check_brain_paths(&brains)?;
    Ok(Resolution::Brains(brains))
}

/// Resolve `request` against the registered brains (research-03 §1.0).
pub fn resolve(config: &GlobalConfig, request: &Request) -> Result<Resolution, ResolveError> {
    let brain = request.brain.filter(|name| !name.is_empty());
    if brain == Some("all") {
        if !request.allow_all {
            return Err(ResolveError::AllNotSupported);
        }
        if config.brains.is_empty() {
            return Err(ResolveError::NoBrains);
        }
        return checked(all_brains(config));
    }
    if let Some(name) = brain {
        let Some(entry) = config.brains.get(name) else {
            return Err(ResolveError::UnknownBrain(name.to_string()));
        };
        return checked(vec![ResolvedBrain {
            name: name.to_string(),
            entry: entry.clone(),
        }]);
    }
    match config.brains.len() {
        0 => Err(ResolveError::NoBrains),
        1 => checked(all_brains(config)),
        _ if request.non_interactive => Err(ResolveError::MultipleBrains {
            names: config.brains.keys().cloned().collect(),
            allow_all: request.allow_all,
        }),
        _ => Ok(Resolution::Pick(Picker {
            brains: all_brains(config),
            allow_all: request.allow_all,
        })),
    }
}

/// v2's `_check_brain_paths`: every brain's path (read as Python's `Path()`
/// reads it) must be an existing directory; the error lists the others with
/// their paths as stored.
pub fn check_brain_paths(brains: &[ResolvedBrain]) -> Result<(), ResolveError> {
    let missing: Vec<(String, String)> = brains
        .iter()
        .filter(|b| !Path::new(&py_path_str(&b.entry.path)).is_dir())
        .map(|b| (b.name.clone(), b.entry.path.clone()))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ResolveError::MissingPaths(missing))
    }
}

/// One line of picker input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// An empty line: Click asks again without a message.
    Empty,
    /// Click's `IntRange` message, printed as `Error: <message>`.
    Invalid(String),
    /// A 1-based option number.
    Picked(usize),
}

/// The prompt Click prints before each read.
pub const PROMPT: &str = "Choice: ";

/// One line as Click's `prompt(type=IntRange(1, max))` reads it: an empty
/// line asks again, else Python's `int()` on the text, then the range
/// check, with Click's messages.
pub fn parse_int_range(line: &str, max: usize) -> Choice {
    if line.is_empty() {
        return Choice::Empty;
    }
    let Some(value) = py_int_from_str(line) else {
        return Choice::Invalid(format!(
            "{} is not a valid integer range.",
            py_repr_str(line)
        ));
    };
    match usize::try_from(&value) {
        Ok(n) if (1..=max).contains(&n) => Choice::Picked(n),
        _ => Choice::Invalid(format!("{value} is not in the range 1<=x<={max}.")),
    }
}

/// Write `prompt` to `out` and read one line from `input`, without its final
/// `\n` (and a `\r` before it); invalid UTF-8 reads as U+FFFD. `None` at
/// end of input (Click's `Abort`).
pub fn prompt_line(input: &mut impl BufRead, out: &mut impl Write, prompt: &str) -> Option<String> {
    let _ = out.write_all(prompt.as_bytes());
    let _ = out.flush();
    let mut raw = Vec::new();
    match input.read_until(b'\n', &mut raw) {
        Ok(0) | Err(_) => return None,
        Ok(_) => {}
    }
    if raw.ends_with(b"\n") {
        raw.pop();
        if raw.ends_with(b"\r") {
            raw.pop();
        }
    }
    Some(String::from_utf8_lossy(&raw).into_owned())
}

/// The interactive picker (`_pick_brain_interactively`): the brains in
/// config order, then `all` when the command fans out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    brains: Vec<ResolvedBrain>,
    allow_all: bool,
}

impl Picker {
    /// The number of options, `all` included.
    pub fn option_count(&self) -> usize {
        self.brains.len() + usize::from(self.allow_all)
    }

    /// The menu v2 prints before the first prompt. A brain whose path does
    /// not exist (Python's `Path(path).exists()`) is marked `(missing)`.
    pub fn menu(&self) -> String {
        let mut out = String::from("Multiple brains registered. Pick one:\n");
        for (i, brain) in self.brains.iter().enumerate() {
            let missing = if Path::new(&py_path_str(&brain.entry.path)).exists() {
                ""
            } else {
                "  (missing)"
            };
            out.push_str(&format!(
                "  [{}] {}  ({}){missing}\n",
                i + 1,
                brain.name,
                brain.entry.path
            ));
        }
        if self.allow_all {
            out.push_str(&format!("  [{}] all\n", self.brains.len() + 1));
        }
        out
    }

    /// Read one line as Click's `prompt(type=IntRange(1, N))` does
    /// ([`parse_int_range`]).
    pub fn parse(&self, line: &str) -> Choice {
        parse_int_range(line, self.option_count())
    }

    /// The brains option `index` (1-based) stands for, path-checked.
    pub fn choose(self, index: usize) -> Result<Vec<ResolvedBrain>, ResolveError> {
        let brains = if index > self.brains.len() {
            self.brains
        } else {
            self.brains
                .into_iter()
                .nth(index.saturating_sub(1))
                .into_iter()
                .collect()
        };
        check_brain_paths(&brains)?;
        Ok(brains)
    }

    /// Print the menu to `out` and prompt on `input` until a valid choice;
    /// end of input is [`ResolveError::Aborted`]. Lines lose their final
    /// `\n` (and a `\r` before it); invalid UTF-8 reads as U+FFFD.
    pub fn run(
        self,
        input: &mut impl BufRead,
        out: &mut impl Write,
    ) -> Result<Vec<ResolvedBrain>, ResolveError> {
        let _ = out.write_all(self.menu().as_bytes());
        loop {
            let Some(line) = prompt_line(input, out, PROMPT) else {
                return Err(ResolveError::Aborted);
            };
            match self.parse(&line) {
                Choice::Empty => {}
                Choice::Invalid(message) => {
                    let _ = writeln!(out, "Error: {message}");
                }
                Choice::Picked(index) => return self.choose(index),
            }
        }
    }
}
