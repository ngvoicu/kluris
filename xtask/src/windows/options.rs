//! The words of `cargo xtask windows` ([`crate::ssh_ci::words`]): `--machine
//! zeewin|zeewin2`, `--parity`, `--parity-files`, `--help`, and after `--` the
//! extra arguments of `cargo test`.

use crate::ssh_ci::words;
pub use crate::ssh_ci::words::checked_parity_files;

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

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let Some(words) = words::parse(args, &["zeewin", "zeewin2"])? else {
        return Ok(Parsed::Help);
    };
    let machine = match words.machine.as_deref() {
        None => None,
        Some("zeewin") => Some(Name::Zeewin),
        Some(_) => Some(Name::Zeewin2),
    };
    Ok(Parsed::Run(Options {
        machine,
        parity: words.parity,
        parity_files: words.parity_files,
        cargo_args: words.cargo_args,
    }))
}
