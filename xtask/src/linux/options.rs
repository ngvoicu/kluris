//! The words of `cargo xtask linux` ([`crate::ssh_ci::words`]): `--parity`,
//! `--parity-files`, `--installer-smoke`, `--wrapper`, `--help`, and after
//! `--` the extra arguments of `cargo test`. There is one machine, so no
//! `--machine`.

use crate::ssh_ci::words;

pub const USAGE: &str = "usage: cargo xtask linux [--parity [--parity-files parity/test.py[,...]]] [-- <cargo test args>]
       cargo xtask linux --installer-smoke [--wrapper <site install.sh>]

Runs `cargo test --workspace` (and with --parity the parity harness against a
release kluris) on zeelin, the household laptop's Ubuntu (WSL2), over ssh, on
this worktree as it is now (HEAD plus uncommitted tracked changes).
--parity-files narrows the parity step to the parity/*.py files it names; each
must be in this tree. The command ends with 75, having run nothing, while
zeelin is not running, while poker-lab's lock or a run on zeewin2 (the same
laptop) holds it, and while another run holds its one-run lock.

--installer-smoke runs no tests: it builds this tree's release with dist and
runs scripts/release/installer-smoke.sh against it in a HOME of its own (with
--wrapper, through the site's install.sh too).";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub parity: bool,
    /// The `parity/*.py` files `--parity-files` names; empty: the whole suite.
    pub parity_files: Vec<String>,
    /// Appended to `cargo test --workspace`.
    pub cargo_args: Vec<String>,
    /// The release installer smoke instead of the tests.
    pub installer_smoke: bool,
    /// The site's install.sh, sent beside the tree for the smoke.
    pub wrapper: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
}

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let Some(words) = words::parse(args, &[])? else {
        return Ok(Parsed::Help);
    };
    Ok(Parsed::Run(Options {
        parity: words.parity,
        parity_files: words.parity_files,
        cargo_args: words.cargo_args,
        installer_smoke: words.installer_smoke,
        wrapper: words.wrapper,
    }))
}

/// The `--wrapper` file as it goes in the tarball: a `.sh`, read whole.
pub fn wrapper_file(given: &str) -> Result<(String, String), String> {
    words::wrapper_file(given, "sh")
}
