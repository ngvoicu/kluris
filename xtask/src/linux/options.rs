//! The words of `cargo xtask linux` ([`crate::ssh_ci::words`]): `--wait
//! MINUTES`, `--parity`,
//! `--parity-files`, `--installer-smoke`, `--wrapper`, `--published`,
//! `--unpinned`, `--help`, and after `--` the extra arguments of `cargo test`. There is one machine, so no
//! `--machine`.

use crate::ssh_ci::words;

pub const USAGE: &str = "usage: cargo xtask linux [--wait MINUTES] [--parity [--parity-files parity/test.py[,...]]] [-- <cargo test args>]
       cargo xtask linux [--wait MINUTES] --installer-smoke [--wrapper <site install.sh>]
       cargo xtask linux [--wait MINUTES] --installer-smoke --published <version> [--unpinned]
       cargo xtask linux [--wait MINUTES] --clippy

Runs `cargo test --workspace` (and with --parity the parity harness against a
release kluris) on zeelin, the household laptop's Ubuntu (WSL2), over ssh, on
this worktree as it is now (HEAD plus uncommitted tracked changes).
--parity-files narrows the parity step to the parity/*.py files it names; each
must be in this tree.

zeelin is the laptop zeewin2 is, so a run waits for its turn in zeewin2's
queue, first come first served with every runner on this Mac (ConsensFlow's
too: ~/.cf-windows-queues/zeewin2, or $CF_WINDOWS_QUEUES/zeewin2), up to
--wait MINUTES (60 unless given, at most 1440), saying each minute where it
stands, then 75. At the head of the queue zeelin is asked: poker-lab's lock,
a run on zeewin2 and Kluris's own one-run lock make it wait on; zeelin not
running ends the command with 75 at once, never waited on. --wait 0 asks once
and ends with 75 if zeelin is busy or a run is queued.

--clippy runs no tests: `cargo clippy --workspace --all-targets -- -D
warnings`, ci.yml's gate, on zeelin.

--installer-smoke runs no tests: it builds this tree's release with dist and
runs scripts/release/installer-smoke.sh against it in a HOME of its own (with
--wrapper, through the site's install.sh too).

--installer-smoke --published <version> builds nothing: it runs
scripts/release/published-smoke.sh, which installs that published GitHub
release through the live https://kluris.ngvoicu.dev/install.sh
(KLURIS_VERSION=<version>) in a HOME of its own, checks --version, doctor
--json and self-update --check --json, then self-uninstall --yes. With
--unpinned the wrapper runs without KLURIS_VERSION, so it installs GitHub's
latest release, which must be <version>.";

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
    /// Clippy on every target (`-D warnings`) instead of the tests.
    pub clippy: bool,
    /// With the installer smoke: the published release to install from
    /// GitHub through the live site wrapper, instead of this tree's build.
    pub published: Option<String>,
    /// With `--published`: no `KLURIS_VERSION`, so the wrapper installs
    /// GitHub's latest release, which must be the one `--published` names.
    pub unpinned: bool,
    /// How long the run waits for its turn at a busy machine, in minutes
    /// (`--wait`); 0 asks once and is refused at once.
    pub wait: u32,
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
        clippy: words.clippy,
        published: words.published,
        unpinned: words.unpinned,
        wait: words.wait,
    }))
}

/// The `--wrapper` file as it goes in the tarball: a `.sh`, read whole.
pub fn wrapper_file(given: &str) -> Result<(String, String), String> {
    words::wrapper_file(given, "sh")
}
