//! The words of `cargo xtask windows`: `--machine NAME` (or `--machine=NAME`),
//! `--parity`, `--help`, and after `--` the extra arguments of `cargo test`,
//! taken as they are.

pub const USAGE: &str =
    "usage: cargo xtask windows [--machine zeewin|zeewin2] [--parity] [-- <cargo test args>]

Runs `cargo test --workspace` (and with --parity the parity harness against a
release kluris.exe) on a household Windows machine over ssh, on this worktree
as it is now (HEAD plus uncommitted tracked changes). With no --machine the run
goes to zeewin2 while it is free and allowed, else to zeewin. A machine whose
one-run lock is held, and a refused zeewin2, end the command with 75.";

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
    /// Appended to `cargo test --workspace`.
    pub cargo_args: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
}

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut options = Options {
        machine: None,
        parity: false,
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
    Ok(Parsed::Run(options))
}
