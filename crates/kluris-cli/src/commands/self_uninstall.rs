//! `kluris self-uninstall [--purge] [--yes] [--json]` (research-02 §5; the
//! removals are `kluris_core::dist::uninstall`'s).

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use kluris_core::companions::prompt::Aborted;
use kluris_core::config::read_global_config;
use kluris_core::dist::{
    Receipt, ReceiptError, UninstallReport, UninstallRequest, UserPath, uninstall,
};
use kluris_core::doctor::{home_relative, slashed};

use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::prompt;
use crate::resolve::{DistPaths, config_path, copilot_home};

pub fn run(purge: bool, yes: bool, as_json: bool) -> Result<(), CliError> {
    let paths = DistPaths::from_env()?;
    let env = paths.env();
    let receipt_path = env.receipt_path().ok_or_else(|| {
        CliError::new("Cannot find the install receipt folder; set LOCALAPPDATA.")
    })?;
    let receipt = Receipt::read(&receipt_path).map_err(|e| refusal(&e))?;

    if purge && !yes {
        if !prompt::can_prompt(as_json) {
            return Err(CliError::new(
                "--purge removes ~/.kluris (config.yml, companions) and every Kluris-written \
                 skill; pass --yes to confirm when not running interactively.",
            ));
        }
        let confirmed = prompt::ask(|input, output| {
            confirm(
                "Remove ~/.kluris (config.yml, companions) and every Kluris-written agent \
                 skill? Your brains are kept.",
                input,
                output,
            )
        })?;
        if !confirmed {
            return Err(CliError::aborted());
        }
    }

    // The registered brains, so a purge never removes one.
    let brains: Vec<(String, PathBuf)> = if purge {
        let config = read_global_config(&config_path()?).map_err(|e| {
            CliError::new(format!("{e}; --purge needs it to keep your brains safe."))
        })?;
        config
            .brains
            .iter()
            .map(|(name, entry)| (name.clone(), PathBuf::from(&entry.path)))
            .collect()
    } else {
        Vec::new()
    };
    let copilot = copilot_home();
    let request = UninstallRequest {
        env,
        receipt: &receipt,
        receipt_path: &receipt_path,
        purge,
        copilot_home: copilot.as_deref(),
        brains: &brains,
    };
    let exe = std::env::current_exe().ok();
    let protect = if purge {
        paths.home.join(".kluris")
    } else {
        receipt.install_prefix.clone()
    };
    let mut remove_binary = |path: &Path| remove_binary(path, exe.as_deref(), &protect);
    let mut registry = user_path();
    let report = uninstall(
        &request,
        registry.as_mut().map(|r| r as &mut dyn UserPath),
        &mut remove_binary,
    )
    .map_err(|e| CliError::new(e.to_string()))?;

    if as_json {
        human::print(&json::to_line(&report_json(&report)));
    } else {
        human::print(&report_text(&report, &paths.home));
    }
    if report.ok() {
        Ok(())
    } else {
        Err(CliError::exit_only())
    }
}

/// The registry's user PATH on Windows; none elsewhere.
#[cfg(windows)]
fn user_path() -> Option<crate::platform::user_path::RegistryUserPath> {
    Some(crate::platform::user_path::RegistryUserPath)
}

#[cfg(not(windows))]
fn user_path() -> Option<NoUserPath> {
    None
}

/// Unix has no user PATH to edit.
#[cfg(not(windows))]
struct NoUserPath;

#[cfg(not(windows))]
impl UserPath for NoUserPath {
    fn read(&self) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn write(&mut self, _value: &str) -> io::Result<()> {
        Ok(())
    }
}

/// Remove an installed binary. The running one on Windows cannot be
/// unlinked while it runs: `self_replace` moves it out of `protect` and has
/// it deleted once this process exits, so `protect` can be removed now.
fn remove_binary(path: &Path, exe: Option<&Path>, protect: &Path) -> io::Result<()> {
    let running = exe.is_some_and(|exe| {
        matches!(
            (std::fs::canonicalize(exe), std::fs::canonicalize(path)),
            (Ok(a), Ok(b)) if a == b
        )
    });
    if running && cfg!(windows) {
        self_replace::self_delete_outside_path(protect)
    } else {
        std::fs::remove_file(path)
    }
}

fn refusal(err: &ReceiptError) -> CliError {
    match err {
        ReceiptError::Missing(path) => CliError::new(format!(
            "No install receipt at {}: kluris was not installed by the Kluris installer \
             here, so there is nothing self-uninstall knows to remove. Remove it the way it \
             was installed.",
            path.display()
        )),
        other => CliError::new(other.to_string()),
    }
}

/// Click's `confirm` with a `[y/N]` default: `y`/`yes` or `n`/`no` (any
/// case), empty for no (a purge is not undone); anything else asks again;
/// end of input aborts.
fn confirm(
    question: &str,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<bool, Aborted> {
    loop {
        let _ = write!(output, "{question} [y/N]: ");
        let _ = output.flush();
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => {
                let _ = writeln!(output);
                return Err(Aborted);
            }
            Ok(_) => {}
        }
        match line.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            _ => {
                let _ = writeln!(output, "Error: invalid input");
            }
        }
    }
}

fn paths_json(paths: &[PathBuf]) -> Json {
    Json::Array(paths.iter().map(|p| Json::Str(slashed(p))).collect())
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| Json::Str(s.clone())).collect())
}

/// `{"ok", "removed", "edited", "kept", "path_entries", "skills": [{"path",
/// "removed", "kept", "failed"}], "purged", "failed", "blocked": [{"path",
/// "problem", "fix"}], "left"}`.
fn report_json(report: &UninstallReport) -> Json {
    let skills = report
        .skills
        .iter()
        .map(|d| {
            Json::Object(vec![
                ("path".into(), Json::Str(slashed(&d.path))),
                ("removed".into(), strings(&d.removed)),
                ("kept".into(), strings(&d.kept)),
                ("failed".into(), strings(&d.failed)),
            ])
        })
        .collect();
    Json::Object(vec![
        ("ok".into(), Json::Bool(report.ok())),
        ("removed".into(), paths_json(&report.removed)),
        ("edited".into(), paths_json(&report.edited)),
        ("kept".into(), paths_json(&report.kept)),
        ("path_entries".into(), strings(&report.path_entries)),
        ("skills".into(), Json::Array(skills)),
        ("purged".into(), Json::Bool(report.purged)),
        ("failed".into(), strings(&report.failed)),
        (
            "blocked".into(),
            Json::Array(
                report
                    .blocked
                    .iter()
                    .map(|b| {
                        Json::Object(vec![
                            ("path".into(), Json::Str(slashed(&b.path))),
                            ("problem".into(), Json::Str(b.problem.clone())),
                            ("fix".into(), Json::Str(b.fix.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("left".into(), strings(&report.left)),
    ])
}

fn report_text(report: &UninstallReport, home: &Path) -> String {
    let mut out = String::new();
    if !report.blocked.is_empty() {
        out.push_str("Nothing was changed. Fix these, then run kluris self-uninstall again:\n");
        for b in &report.blocked {
            out.push_str(&format!(
                "  {}: {}\n    fix: {}\n",
                home_relative(&b.path, home),
                b.problem,
                b.fix
            ));
        }
        return out;
    }
    for path in &report.removed {
        out.push_str(&format!("Removed {}\n", home_relative(path, home)));
    }
    for path in &report.edited {
        out.push_str(&format!(
            "Took the kluris PATH line out of {}\n",
            home_relative(path, home)
        ));
    }
    for path in &report.kept {
        out.push_str(&format!(
            "Kept {} and the lines that source it: it is not the file the installer wrote\n",
            home_relative(path, home)
        ));
    }
    for entry in &report.path_entries {
        out.push_str(&format!("Removed {entry} from your user PATH\n"));
    }
    for dest in &report.skills {
        if !dest.removed.is_empty() {
            out.push_str(&format!(
                "Removed Kluris skills from {}: {}\n",
                home_relative(&dest.path, home),
                dest.removed.join(", ")
            ));
        }
        if !dest.kept.is_empty() {
            out.push_str(&format!(
                "Kept in {} (not written by Kluris): {}\n",
                home_relative(&dest.path, home),
                dest.kept.join(", ")
            ));
        }
        for failure in &dest.failed {
            out.push_str(&format!("FAILED {failure}\n"));
        }
    }
    for failure in &report.failed {
        out.push_str(&format!("FAILED {failure}\n"));
    }
    if report.purged {
        out.push_str("Kept your brains.\n");
    } else {
        out.push_str(
            "Kept ~/.kluris/config.yml, the companions, the agent skills and your brains.\n",
        );
    }
    if report.ok() {
        out.push_str(
            "kluris is uninstalled. Open a new terminal so PATH no longer has ~/.kluris/bin.\n",
        );
    } else if !report.left.is_empty() {
        out.push_str("kluris is uninstalled, but these are left; to finish by hand:\n");
        for todo in &report.left {
            out.push_str(&format!("  {todo}\n"));
        }
    } else {
        out.push_str("kluris is partly uninstalled: see FAILED above, then run it again.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(input: &str) -> (Result<bool, Aborted>, String) {
        let mut out = Vec::new();
        let answer = confirm("Go?", &mut io::Cursor::new(input.as_bytes()), &mut out);
        (answer, String::from_utf8(out).unwrap())
    }

    #[test]
    fn confirm_reads_yes_and_no() {
        for yes in ["y\n", "Y\n", "yes\n", " YES \n"] {
            assert_eq!(ask(yes).0, Ok(true), "{yes:?}");
        }
        for no in ["\n", "n\n", "No\n"] {
            assert_eq!(ask(no).0, Ok(false), "{no:?}");
        }
        assert_eq!(ask("y\n").1, "Go? [y/N]: ");
    }

    #[test]
    fn confirm_asks_again_and_aborts_at_end_of_input() {
        let (answer, out) = ask("maybe\ny\n");
        assert_eq!(answer, Ok(true));
        assert_eq!(out, "Go? [y/N]: Error: invalid input\nGo? [y/N]: ");
        assert_eq!(ask("").0, Err(Aborted));
        assert_eq!(ask("maybe\n").0, Err(Aborted));
    }
}
