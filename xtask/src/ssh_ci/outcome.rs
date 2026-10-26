//! How a run ends, whichever machine it went to: the lines the machine marks
//! its output with, the status this command answers, and the summary line.

use std::io::{self, Write};

/// A machine that cannot take the run now: its lock is held, or it is refused.
pub const REFUSED: i32 = 75;
/// This side was stopped (Ctrl-C).
pub const STOPPED: i32 = 130;
/// The run passed its bound.
pub const TIMED_OUT: i32 = 124;

/// The run's last line: `== exit N`.
pub const EXIT_MARK: &str = "== exit ";
/// The commit the machine unpacked: `== tree <commit>`.
pub const TREE_MARK: &str = "== tree ";
/// Each step's start and end: `== step NAME exit CODE SECONDSs`.
pub const STEP_MARK: &str = "== step ";

/// The status a line says the run ended with: `== exit N`, nothing else.
pub fn exit_said(line: &str) -> Option<i32> {
    let number = line.trim_end().strip_prefix(EXIT_MARK)?;
    let digits = number.strip_prefix('-').unwrap_or(number);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

/// The status a run that ssh ended by itself comes to, and what is to be said
/// of it besides. `said` is the machine's own last line (`== exit N`): it is the
/// answer whatever ssh's status is, since ssh to a PowerShell answers 1 for any
/// failure. Without it ssh's status is the answer if it is a failure, and never
/// 0. A pass needs the tree said: a run that never unpacked passed nothing. A
/// status no program here ends with (Windows' -1073741819) is 1.
pub fn answer(ssh: Option<i32>, said: Option<i32>, tree_said: bool) -> (i32, Option<String>) {
    let holds = |status: i32| (0..=255).contains(&status);
    match said {
        Some(0) if !tree_said => (
            1,
            Some(format!(
                "no `{TREE_MARK}…` line said which tree the machine ran, so its 0 is not a pass: answering 1"
            )),
        ),
        Some(status) if holds(status) => (status, None),
        Some(status) => (
            1,
            Some(format!(
                "the run ended with status {status}, which no status here holds: answering 1"
            )),
        ),
        None => {
            let missing = format!("no `{EXIT_MARK}N` line ended the output");
            match ssh {
                Some(code) if code != 0 && holds(code) => (
                    code,
                    Some(format!("{missing}, so ssh's status is the answer: {code}")),
                ),
                Some(code) => (
                    1,
                    Some(format!("{missing}, and ssh ended with {code}: answering 1")),
                ),
                None => (
                    1,
                    Some(format!(
                        "{missing}, and ssh was ended by a signal: answering 1"
                    )),
                ),
            }
        }
    }
}

/// What the machine's output said, read as it goes by: the last line, the
/// tree, and each step's end (`== step NAME exit CODE SECONDSs`).
#[derive(Default)]
pub struct Said {
    partial: Vec<u8>,
    pub last_line: Option<String>,
    pub tree: Option<String>,
    pub steps: Vec<(String, String, String)>,
}

impl Said {
    pub fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                let line = String::from_utf8_lossy(&self.partial).trim_end().to_owned();
                self.partial.clear();
                self.line(line);
            } else {
                self.partial.push(byte);
            }
        }
    }

    pub fn finish(&mut self) {
        if !self.partial.is_empty() {
            let line = String::from_utf8_lossy(&self.partial).trim_end().to_owned();
            self.partial.clear();
            self.line(line);
        }
    }

    fn line(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        if let Some(tree) = line.strip_prefix(TREE_MARK) {
            self.tree = Some(tree.trim().to_owned());
        }
        if let Some(rest) = line.strip_prefix(STEP_MARK)
            && let [name, "exit", code, seconds] = rest.split_whitespace().collect::<Vec<_>>()[..]
        {
            self.steps
                .push((name.to_owned(), code.to_owned(), seconds.to_owned()));
        }
        self.last_line = Some(line);
    }
}

/// `<task>: <machine> run <id> tree <commit>: <step> <status> (<time>), ...; exit <code>`.
pub fn summary(
    task: &str,
    machine: &str,
    id: &str,
    commit: &str,
    said: &Said,
    code: i32,
) -> String {
    let tree: String = commit.chars().take(12).collect();
    let steps = if said.steps.is_empty() {
        "no step ran".to_owned()
    } else {
        said.steps
            .iter()
            .map(|(name, code, seconds)| format!("{name} {code} ({seconds})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{task}: {machine} run {id} tree {tree}: {steps}; exit {code}")
}

pub fn say(to: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(to, "{line}")
        .and_then(|()| to.flush())
        .map_err(|e: io::Error| format!("could not write: {e}"))
}
