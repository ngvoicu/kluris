//! The words of `cargo xtask windows` ([`crate::ssh_ci::words`]): `--machine
//! zeewin|zeewin2`, `--parity`, `--parity-files`, `--installer-smoke`,
//! `--wrapper`, `--published`, `--unpinned`, `--help`, and after `--` the
//! extra arguments of `cargo test`.

use crate::ssh_ci::words;
pub use crate::ssh_ci::words::checked_parity_files;

pub const USAGE: &str =
    "usage: cargo xtask windows [--machine zeewin|zeewin2] [--parity [--parity-files parity/test.py[,...]]] [-- <cargo test args>]
       cargo xtask windows [--machine zeewin|zeewin2] --installer-smoke [--wrapper <site install.ps1>]
       cargo xtask windows [--machine zeewin|zeewin2] --installer-smoke --published <version> [--unpinned]
       cargo xtask windows [--machine zeewin|zeewin2] --clippy

Runs `cargo test --workspace` (and with --parity the parity harness against a
release kluris.exe) on a household Windows machine over ssh, on this worktree
as it is now (HEAD plus uncommitted tracked changes). --parity-files narrows
the parity step to the parity/*.py files it names; each must be in this tree.
With no --machine the run goes to zeewin2 while it is free and allowed, else
to zeewin. A machine whose one-run lock is held, and a refused zeewin2, end
the command with 75.

--clippy runs no tests: `cargo clippy --workspace --all-targets -- -D
warnings`, ci.yml's gate, on Windows.

--installer-smoke runs no tests: it builds this tree's release with dist and
runs scripts\\release\\installer-smoke.ps1 against it (with --wrapper, through
the site's install.ps1 too). That smoke changes the user PATH while it runs,
having saved it first, and writes the saved bytes back (the run does again if
the step was ended), then compares them byte for byte.

--installer-smoke --published <version> builds nothing: it runs
scripts\\release\\published-smoke.ps1, which installs that published GitHub
release through the live https://kluris.ngvoicu.dev/install.ps1
(KLURIS_VERSION=<version>) into a sandbox profile, checks --version, doctor
--json and self-update --check --json, then self-uninstall --yes; the user
PATH is saved, written back and compared the same way. With --unpinned the
wrapper runs without KLURIS_VERSION, so it installs GitHub's latest release,
which must be <version>.";

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
    /// The release installer smoke instead of the tests.
    pub installer_smoke: bool,
    /// The site's install.ps1, sent beside the tree for the smoke.
    pub wrapper: Option<String>,
    /// Clippy on every target (`-D warnings`) instead of the tests.
    pub clippy: bool,
    /// With the installer smoke: the published release to install from
    /// GitHub through the live site wrapper, instead of this tree's build.
    pub published: Option<String>,
    /// With `--published`: no `KLURIS_VERSION`, so the wrapper installs
    /// GitHub's latest release, which must be the one `--published` names.
    pub unpinned: bool,
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
        installer_smoke: words.installer_smoke,
        wrapper: words.wrapper,
        clippy: words.clippy,
        published: words.published,
        unpinned: words.unpinned,
    }))
}

/// The `--wrapper` file as it goes in the tarball: a `.ps1`, read whole.
pub fn wrapper_file(given: &str) -> Result<(String, String), String> {
    words::wrapper_file(given, "ps1")
}
