//! The companion pickers of `kluris companion add|remove` (v2
//! `_prompt_for_companions`, `_prompt_companions_to_remove`), with Click's
//! prompts and messages. They read `input` and write `out`; the caller
//! decides whether a terminal may be asked at all.

use std::io::{BufRead, Write};

use super::KNOWN;
use crate::pyrepr::py_repr_str;
use crate::resolve::{Choice, parse_int_range, prompt_line};

/// End of input at a prompt: Click's `Abort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aborted;

/// The add picker's menu.
pub const ADD_MENU: &str = "  Install specmint companions for this brain?
    [1] specmint-core      (spec-driven-development companion)
    [2] specmint-tdd       (spec-driven-development with test-driven-development focus)
    [3] core + tdd
    [4] skip
    [5] specmint-core-html (spec-driven-development with canonical SPEC.html)
    [6] specmint-tdd-html  (TDD-focused specs with canonical SPEC.html)
    [7] all
";

/// The add picker's prompt (`click.prompt("  Choice", default="4")`).
pub const ADD_PROMPT: &str = "  Choice [4]: ";

/// The companions add-picker choice `line` stands for (an empty line is the
/// default, `4`); else Click's `Choice` message.
pub fn add_choice(line: &str) -> Result<Vec<String>, String> {
    let line = if line.is_empty() { "4" } else { line };
    let names: &[&str] = match line {
        "1" => &["specmint-core"],
        "2" => &["specmint-tdd"],
        "3" => &["specmint-core", "specmint-tdd"],
        "4" => &[],
        "5" => &["specmint-core-html"],
        "6" => &["specmint-tdd-html"],
        "7" => &KNOWN,
        other => {
            return Err(format!(
                "{} is not one of '1', '2', '3', '4', '5', '6', '7'.",
                py_repr_str(other)
            ));
        }
    };
    Ok(names.iter().map(|n| n.to_string()).collect())
}

/// Ask which companions to add, until a valid choice.
pub fn pick_to_add(input: &mut impl BufRead, out: &mut impl Write) -> Result<Vec<String>, Aborted> {
    let _ = out.write_all(ADD_MENU.as_bytes());
    loop {
        let line = prompt_line(input, out, ADD_PROMPT).ok_or(Aborted)?;
        match add_choice(&line) {
            Ok(names) => return Ok(names),
            Err(message) => {
                let _ = writeln!(out, "Error: {message}");
            }
        }
    }
}

/// A `click.confirm(..., default=True)` answer: `Some(yes)`, or `None` for
/// input Click refuses (`Error: invalid input`).
pub fn confirm_answer(line: &str) -> Option<bool> {
    match line.to_lowercase().trim() {
        "y" | "yes" | "" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

/// The remove picker's menu for `enabled` (two or more).
pub fn remove_menu(enabled: &[String]) -> String {
    let mut out = String::from("  Which companion(s) to remove?\n");
    for (i, name) in enabled.iter().enumerate() {
        out.push_str(&format!("    [{}] {name}\n", i + 1));
    }
    out.push_str(&format!("    [{}] all\n", enabled.len() + 1));
    out.push_str(&format!("    [{}] cancel\n", enabled.len() + 2));
    out
}

/// The remove picker's prompt.
pub const REMOVE_PROMPT: &str = "  Choice: ";

/// Ask which of `enabled` to remove: a yes/no question for one, a numbered
/// menu with `all` and `cancel` for several, nothing for none.
pub fn pick_to_remove(
    enabled: &[String],
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<Vec<String>, Aborted> {
    match enabled {
        [] => Ok(Vec::new()),
        [only] => {
            let prompt = format!("  Remove {only}? [Y/n]: ");
            loop {
                let line = prompt_line(input, out, &prompt).ok_or(Aborted)?;
                match confirm_answer(&line) {
                    Some(true) => return Ok(vec![only.clone()]),
                    Some(false) => return Ok(Vec::new()),
                    None => {
                        let _ = writeln!(out, "Error: invalid input");
                    }
                }
            }
        }
        _ => {
            let _ = out.write_all(remove_menu(enabled).as_bytes());
            let count = enabled.len() + 2;
            loop {
                let line = prompt_line(input, out, REMOVE_PROMPT).ok_or(Aborted)?;
                match parse_int_range(&line, count) {
                    Choice::Empty => {}
                    Choice::Invalid(message) => {
                        let _ = writeln!(out, "Error: {message}");
                    }
                    Choice::Picked(n) if n == count => return Ok(Vec::new()),
                    Choice::Picked(n) if n == count - 1 => return Ok(enabled.to_vec()),
                    Choice::Picked(n) => return Ok(vec![enabled[n - 1].clone()]),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_add(typed: &str) -> (Result<Vec<String>, Aborted>, String) {
        let mut out = Vec::new();
        let picked = pick_to_add(&mut typed.as_bytes(), &mut out);
        (picked, String::from_utf8(out).unwrap())
    }

    fn run_remove(enabled: &[&str], typed: &str) -> (Result<Vec<String>, Aborted>, String) {
        let enabled: Vec<String> = enabled.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        let picked = pick_to_remove(&enabled, &mut typed.as_bytes(), &mut out);
        (picked, String::from_utf8(out).unwrap())
    }

    #[test]
    fn add_choices_map_like_v2() {
        assert_eq!(add_choice("").unwrap(), Vec::<String>::new());
        assert_eq!(add_choice("3").unwrap(), ["specmint-core", "specmint-tdd"]);
        assert_eq!(add_choice("7").unwrap(), KNOWN);
        assert_eq!(
            add_choice(" 3").unwrap_err(),
            "' 3' is not one of '1', '2', '3', '4', '5', '6', '7'."
        );
        assert_eq!(
            add_choice("it's").unwrap_err(),
            "\"it's\" is not one of '1', '2', '3', '4', '5', '6', '7'."
        );
    }

    #[test]
    fn add_picker_reprompts_then_takes_the_default() {
        let (picked, out) = run_add("9\n\n");
        assert_eq!(picked, Ok(vec![]));
        assert_eq!(
            out,
            format!(
                "{ADD_MENU}  Choice [4]: Error: '9' is not one of '1', '2', '3', '4', '5', '6', '7'.\n  Choice [4]: "
            )
        );
        assert_eq!(run_add("").0, Err(Aborted));
    }

    #[test]
    fn confirm_reads_like_click() {
        for yes in ["", "y", "Y", " yes ", "YES"] {
            assert_eq!(confirm_answer(yes), Some(true), "{yes:?}");
        }
        for no in ["n", "No", " NO"] {
            assert_eq!(confirm_answer(no), Some(false), "{no:?}");
        }
        assert_eq!(confirm_answer("yep"), None);
    }

    #[test]
    fn one_enabled_is_a_question() {
        let (picked, out) = run_remove(&["specmint-tdd"], "junk\nn\n");
        assert_eq!(picked, Ok(vec![]));
        assert_eq!(
            out,
            "  Remove specmint-tdd? [Y/n]: Error: invalid input\n  Remove specmint-tdd? [Y/n]: "
        );
        assert_eq!(
            run_remove(&["specmint-tdd"], "\n").0,
            Ok(vec!["specmint-tdd".to_string()])
        );
        assert_eq!(run_remove(&["specmint-tdd"], "").0, Err(Aborted));
    }

    #[test]
    fn several_enabled_is_a_menu() {
        let enabled = ["specmint-core", "specmint-tdd-html"];
        let (picked, out) = run_remove(&enabled, "0\nx\n\n2\n");
        assert_eq!(picked, Ok(vec!["specmint-tdd-html".to_string()]));
        assert_eq!(
            out,
            "  Which companion(s) to remove?\n    [1] specmint-core\n    [2] specmint-tdd-html\n    [3] all\n    [4] cancel\n  Choice: Error: 0 is not in the range 1<=x<=4.\n  Choice: Error: 'x' is not a valid integer range.\n  Choice:   Choice: "
        );
        assert_eq!(run_remove(&enabled, "3\n").0.unwrap(), enabled);
        assert_eq!(run_remove(&enabled, "4\n").0, Ok(vec![]));
        assert_eq!(run_remove(&[], "").0, Ok(vec![]));
    }
}
