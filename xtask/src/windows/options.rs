//! The words of `cargo xtask windows`: `--machine NAME` (or `--machine=NAME`),
//! `--parity`, `--parity-files parity/TEST.PY[,...]` (repeatable, needs
//! `--parity`), `--help`, and after `--` the extra arguments of `cargo test`,
//! taken as they are.

use std::ffi::OsStr;
use std::path::{Component, Path};

pub const USAGE: &str =
    "usage: cargo xtask windows [--machine zeewin|zeewin2] [--parity [--parity-files parity/test.py[,...]]] [-- <cargo test args>]

Runs `cargo test --workspace` (and with --parity the parity harness against a
release kluris.exe) on a household Windows machine over ssh, on this worktree
as it is now (HEAD plus uncommitted tracked changes). --parity-files narrows
the parity step to the parity/*.py files it names; each must be in this tree.
With no --machine the run goes to zeewin2 while it is free and allowed, else
to zeewin. A machine whose one-run lock is held, and a refused zeewin2, end
the command with 75.";

/// The machines `--machine` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Name {
    Zeewin,
    Zeewin2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// None: the default machine, chosen when the run goes.
    pub machine: Option<Name>,
    pub parity: bool,
    /// The `parity/*.py` files `--parity-files` names; empty: the whole suite.
    pub parity_files: Vec<String>,
    /// Appended to `cargo test --workspace`.
    pub cargo_args: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
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

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut options = Options {
        machine: None,
        parity: false,
        parity_files: Vec::new(),
        cargo_args: Vec::new(),
    };
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word.as_str(), None),
        };
        match name {
            "--" => {
                options.cargo_args = args[at..].to_vec();
                break;
            }
            "--help" | "-h" => return Ok(Parsed::Help),
            "--parity" if inline.is_some() => return Err("--parity takes no value".into()),
            "--parity" => options.parity = true,
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
                        file => options.parity_files.push(file.to_owned()),
                    }
                }
            }
            "--machine" => {
                if options.machine.is_some() {
                    return Err("--machine is given once".into());
                }
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let next = args.get(at).filter(|next| !next.starts_with('-'));
                        at += 1;
                        next.ok_or("--machine takes a machine: zeewin, zeewin2")?
                    }
                };
                options.machine = Some(match value {
                    "zeewin" => Name::Zeewin,
                    "zeewin2" => Name::Zeewin2,
                    other => {
                        return Err(format!("--machine is one of zeewin, zeewin2, not {other}"));
                    }
                });
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                return Err(format!(
                    "unexpected argument {other:?}: cargo test's own arguments go after --"
                ));
            }
        }
    }
    if !options.parity_files.is_empty() && !options.parity {
        return Err("--parity-files goes with --parity".into());
    }
    Ok(Parsed::Run(options))
}
