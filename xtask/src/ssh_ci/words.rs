//! The words `cargo xtask windows` and `cargo xtask linux` share:
//! `--installer-smoke` and its `--wrapper FILE` or `--published VERSION`
//! (with `--unpinned`), `--clippy`, `--wait MINUTES` (the turn in the shared
//! queue, [`super::queue`]), `--help`, after `--` the
//! extra arguments of `cargo test` taken as they are, and for a task with a
//! machine to name `--machine NAME` (or `--machine=NAME`).

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use super::plan::WRAPPER_DIR;
use super::queue::{DEFAULT_WAIT_MINUTES, MAX_WAIT_MINUTES};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Words {
    /// Appended to `cargo test --workspace`.
    pub cargo_args: Vec<String>,
    /// The release installer smoke instead of the tests.
    pub installer_smoke: bool,
    /// The site wrapper `--wrapper` names, sent beside the tree for the smoke.
    pub wrapper: Option<String>,
    /// `cargo clippy --workspace --all-targets -- -D warnings` instead of the tests.
    pub clippy: bool,
    /// The release `--published` names, installed from GitHub through the
    /// live site wrapper instead of a build of this tree.
    pub published: Option<String>,
    /// With `--published`: install GitHub's latest release (no
    /// `KLURIS_VERSION`) and expect it to be the one `--published` names.
    pub unpinned: bool,
    /// How long the run waits for its turn at a busy machine, in minutes; 0
    /// asks once and is refused at once.
    pub wait: u32,
}

/// `args` as [`Words`], or none for `--help`. `machine` is the one value
/// `--machine` takes, which names the machine the run goes to anyway; with
/// none, `--machine` is an unknown option.
pub fn parse(args: &[String], machine: Option<&str>) -> Result<Option<Words>, String> {
    let mut words = Words {
        wait: DEFAULT_WAIT_MINUTES,
        ..Words::default()
    };
    let mut waits = false;
    let mut named = false;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word.as_str(), None),
        };
        match name {
            "--" => {
                words.cargo_args = args[at..].to_vec();
                break;
            }
            "--help" | "-h" => return Ok(None),
            "--installer-smoke" if inline.is_some() => {
                return Err("--installer-smoke takes no value".into());
            }
            "--installer-smoke" => words.installer_smoke = true,
            "--unpinned" if inline.is_some() => return Err("--unpinned takes no value".into()),
            "--unpinned" => words.unpinned = true,
            "--clippy" if inline.is_some() => return Err("--clippy takes no value".into()),
            "--clippy" => words.clippy = true,
            "--wrapper" => {
                if words.wrapper.is_some() {
                    return Err("--wrapper is given once".into());
                }
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or(
                            "--wrapper takes the site's installer: --wrapper <install file>",
                        )?
                    }
                };
                words.wrapper = Some(value.to_owned());
            }
            "--published" => {
                if words.published.is_some() {
                    return Err("--published is given once".into());
                }
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or("--published takes a release: --published 3.0.0-rc.1")?
                    }
                };
                words.published = Some(release_number(value)?);
            }
            "--wait" => {
                if waits {
                    return Err("--wait is given once".into());
                }
                waits = true;
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or_else(|| {
                            format!("--wait takes the minutes to wait for a turn, 0 to {MAX_WAIT_MINUTES}")
                        })?
                    }
                };
                words.wait = value
                    .parse::<u32>()
                    .ok()
                    .filter(|minutes| *minutes <= MAX_WAIT_MINUTES)
                    .ok_or_else(|| {
                        format!(
                            "--wait is the minutes to wait for a turn, 0 to {MAX_WAIT_MINUTES}: {value}"
                        )
                    })?;
            }
            "--machine" if let Some(machine) = machine => {
                if named {
                    return Err("--machine is given once".into());
                }
                named = true;
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or_else(|| format!("--machine takes the machine: {machine}"))?
                    }
                };
                if value != machine {
                    return Err(format!(
                        "--machine is {machine}, the one machine, not {value}"
                    ));
                }
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                return Err(format!(
                    "unexpected argument {other:?}: cargo test's own arguments go after --"
                ));
            }
        }
    }
    if words.wrapper.is_some() && !words.installer_smoke {
        return Err("--wrapper goes with --installer-smoke".into());
    }
    if words.unpinned && words.published.is_none() {
        return Err("--unpinned goes with --published <version>".into());
    }
    if words.published.is_some() && !words.installer_smoke {
        return Err("--published goes with --installer-smoke".into());
    }
    if words.published.is_some() && words.wrapper.is_some() {
        return Err(
            "--published installs through the live site's install script: not with --wrapper"
                .into(),
        );
    }
    if words.clippy && words.installer_smoke {
        return Err("--clippy runs clippy alone: not with --installer-smoke".into());
    }
    if words.clippy && !words.cargo_args.is_empty() {
        return Err("--clippy runs no cargo test: nothing goes after --".into());
    }
    if words.installer_smoke && !words.cargo_args.is_empty() {
        return Err("--installer-smoke runs no cargo test: nothing goes after --".into());
    }
    Ok(Some(words))
}

/// `given` as a release number, `3.0.0` or `3.0.0-rc.1` (a leading `v`
/// dropped): three dot-separated numbers, then optionally `-` and a
/// prerelease of letters, digits, dots and hyphens. Anything else is refused
/// before it reaches a script.
fn release_number(given: &str) -> Result<String, String> {
    let version = given.strip_prefix('v').unwrap_or(given);
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    let core_ok = numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let pre_ok = pre.is_none_or(|pre| {
        !pre.is_empty()
            && pre
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    });
    if core_ok && pre_ok {
        Ok(version.to_owned())
    } else {
        Err(format!(
            "--published takes a release number such as 3.0.0 or 3.0.0-rc.1, not {given:?}"
        ))
    }
}

/// The site wrapper `--wrapper` names, as it goes in the tarball: its path
/// there (`.kluris-site/install.<extension>`) and its text, read whole. It
/// must be a `.<extension>` file (`ps1` for Windows, `sh` for Linux).
pub fn wrapper_file(given: &str, extension: &str) -> Result<(String, String), String> {
    let path = Path::new(given);
    if path.extension() != Some(OsStr::new(extension)) {
        return Err(format!(
            "--wrapper takes a .{extension} file, not {given:?}"
        ));
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("--wrapper: could not read {given}: {e}"))?;
    Ok((format!("{WRAPPER_DIR}/install.{extension}"), text))
}
