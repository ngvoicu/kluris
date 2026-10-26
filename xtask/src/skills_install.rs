//! `skills-install` and `posix-path`: kluris-core's skill install and its
//! POSIX path rule, for `parity/test_skill_install.py`, which migrates a HOME
//! v2 wrote with the install alone (create, register, remove, companion
//! add/remove and doctor call the same install).

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use kluris_core::agents::install::{
    DestinationRole, InstallReport, InstallRequest, install_skills, read_brain_configs,
};
use kluris_core::agents::render::posix_path;
use kluris_core::config::read_global_config;
use serde_json::{Value, json};

/// `skills-install --home <dir>`: install every registered brain's skill
/// into `<dir>` as HOME, with the config at `KLURIS_CONFIG` (else
/// `<dir>/.kluris/config.yml`) and `COPILOT_HOME` from the environment.
/// Prints the report as JSON.
pub fn main(args: &[String]) -> Result<(), String> {
    let [flag, home] = args else {
        return Err("usage: skills-install --home <dir>".into());
    };
    if flag != "--home" {
        return Err("usage: skills-install --home <dir>".into());
    }
    let home = PathBuf::from(home);
    let config_path = std::env::var_os("KLURIS_CONFIG")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".kluris").join("config.yml"));
    let config = read_global_config(&config_path).map_err(|e| e.to_string())?;
    let copilot_home = std::env::var_os("COPILOT_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let brains = read_brain_configs(&config);
    let report = install_skills(&InstallRequest {
        config: &config,
        brains: &brains,
        home: &home,
        copilot_home: copilot_home.as_deref(),
    });
    println!("{}", report_json(&report, &home));
    Ok(())
}

fn rel(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn report_json(report: &InstallReport, home: &Path) -> Value {
    let destinations: Vec<Value> = report
        .destinations
        .iter()
        .map(|d| {
            let role = match &d.role {
                DestinationRole::Universal => "universal".to_string(),
                DestinationRole::Agent(a) => format!("agent:{a}"),
                DestinationRole::Deselected(a) => format!("deselected:{a}"),
                DestinationRole::Migration => "migration".to_string(),
            };
            json!({
                "path": rel(&d.path, home),
                "role": role,
                "alias_of": d.alias_of.as_deref().map(|p| rel(p, home)),
                "written": d.written,
                "removed": d.removed,
                "kept": d.kept,
                "failed": d.failed,
            })
        })
        .collect();
    json!({
        "ok": report.ok(),
        "skills": report.skills,
        "agents": report.agents,
        "destinations": destinations,
        "brain_errors": report.brain_errors,
    })
}

/// `posix-path`: one JSON string per stdin line, answered with the JSON
/// string of its POSIX form (v2 `agents._posix_path`).
pub fn posix(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: posix-path".into());
    }
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let input: String = serde_json::from_str(&line).map_err(|e| format!("{line}: {e}"))?;
        let answer = serde_json::to_string(&posix_path(&input)).map_err(|e| e.to_string())?;
        writeln!(out, "{answer}").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
