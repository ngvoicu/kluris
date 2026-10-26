//! The words `cargo xtask windows` and `cargo xtask linux` share: `--parity`,
//! `--parity-files parity/TEST.PY[,...]` (repeatable, needs `--parity`),
//! `--installer-smoke` and its `--wrapper FILE` or `--published VERSION`,
//! `--help`, after `--` the
//! extra arguments of `cargo test` taken as they are, and for a task with more
//! than one machine `--machine NAME` (or `--machine=NAME`).

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path};

use super::plan::WRAPPER_DIR;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Words {
    /// The `--machine` value, one of the task's machines; none when not given.
    pub machine: Option<String>,
    pub parity: bool,
    /// The `parity/*.py` files `--parity-files` names; empty: the whole suite.
    pub parity_files: Vec<String>,
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
}

/// `args` as [`Words`], or none for `--help`. `machines` are the values
/// `--machine` takes; with none, `--machine` is an unknown option.
pub fn parse(args: &[String], machines: &[&str]) -> Result<Option<Words>, String> {
    let mut words = Words::default();
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
            "--parity" if inline.is_some() => return Err("--parity takes no value".into()),
            "--parity" => words.parity = true,
            "--installer-smoke" if inline.is_some() => {
                return Err("--installer-smoke takes no value".into());
            }
            "--installer-smoke" => words.installer_smoke = true,
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
            "--parity-files" => {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or(
                            "--parity-files takes a path: parity/test_x.py[,parity/test_y.py]",
                        )?
                    }
                };
                for file in value.split(',') {
                    match file.trim() {
                        "" => {
                            return Err(
                                "--parity-files takes a path: parity/test_x.py[,parity/test_y.py]"
                                    .into(),
                            );
                        }
                        file => words.parity_files.push(file.to_owned()),
                    }
                }
            }
            "--machine" if !machines.is_empty() => {
                if words.machine.is_some() {
                    return Err("--machine is given once".into());
                }
                let list = machines.join(", ");
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or_else(|| format!("--machine takes a machine: {list}"))?
                    }
                };
                if !machines.contains(&value) {
                    return Err(format!("--machine is one of {list}, not {value}"));
                }
                words.machine = Some(value.to_owned());
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                return Err(format!(
                    "unexpected argument {other:?}: cargo test's own arguments go after --"
                ));
            }
        }
    }
    if !words.parity_files.is_empty() && !words.parity {
        return Err("--parity-files goes with --parity".into());
    }
    if words.wrapper.is_some() && !words.installer_smoke {
        return Err("--wrapper goes with --installer-smoke".into());
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
    if words.installer_smoke && words.parity {
        return Err(
            "--installer-smoke runs the installers, not the tests: not with --parity".into(),
        );
    }
    if words.clippy && (words.parity || words.installer_smoke) {
        return Err("--clippy runs clippy alone: not with --parity or --installer-smoke".into());
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

/// The `--parity-files` paths checked against the tree at `root`: each must
/// name a `parity/*.py` file that is there, and what the remote pytest is
/// given is the path in its `parity/...` form. An empty list is no check.
pub fn checked_parity_files(root: &Path, given: &[String]) -> Result<Vec<String>, String> {
    given.iter().map(|file| parity_file(root, file)).collect()
}

fn parity_file(root: &Path, given: &str) -> Result<String, String> {
    let path = Path::new(given);
    let mut components = path.components();
    let under_parity = matches!(
        components.next(),
        Some(Component::Normal(first)) if first == OsStr::new("parity")
    );
    let rest: Vec<Component> = components.collect();
    if !under_parity
        || rest.is_empty()
        || !rest.iter().all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "--parity-files takes files under parity/, as parity/test_x.py, not {given:?}"
        ));
    }
    if path.extension() != Some(OsStr::new("py")) {
        return Err(format!("--parity-files takes .py files, not {given:?}"));
    }
    if !root.join(path).is_file() {
        return Err(format!("--parity-files: {given} is not in this tree"));
    }
    let mut normalized = String::from("parity");
    for part in rest {
        normalized.push('/');
        normalized.push_str(&part.as_os_str().to_string_lossy());
    }
    Ok(normalized)
}
