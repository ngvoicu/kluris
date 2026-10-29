//! The words of `cargo xtask windows` ([`crate::ssh_ci::words`]): `--machine
//! zeewin2`, `--wait MINUTES`, `--installer-smoke`,
//! `--wrapper`, `--published`, `--unpinned`, `--help`, and after `--` the
//! extra arguments of `cargo test`.

use super::machine::ZEEWIN2;
use crate::ssh_ci::words;

pub const USAGE: &str =
    "usage: cargo xtask windows [--machine zeewin2] [--wait MINUTES] [-- <cargo test args>]
       cargo xtask windows [--machine zeewin2] [--wait MINUTES] --installer-smoke [--wrapper <site install.ps1>]
       cargo xtask windows [--machine zeewin2] [--wait MINUTES] --installer-smoke --published <version> [--unpinned]
       cargo xtask windows [--machine zeewin2] [--wait MINUTES] --clippy

Runs `cargo test --workspace --no-fail-fast` on zeewin2, the household
laptop's Windows, over ssh, on this worktree as it is now (HEAD plus
uncommitted tracked changes). zeewin2 is the one machine: --machine zeewin2
says so and changes nothing.

A busy zeewin2 is waited for, first come first served, in the queue every
runner on this Mac shares with ConsensFlow's (~/.cf-windows-queues/zeewin2,
or $CF_WINDOWS_QUEUES): up to --wait MINUTES (60 unless given, at most 1440),
saying each minute where the run stands, then 75. At the head of the queue
zeewin2's own checks decide (poker-lab's lock, a Kluris Linux run on zeelin,
the disk floor), and a one-run lock that still refuses the run sends it back
to wait. --wait 0 asks once and ends with 75 if zeewin2 is busy or a run is
queued for it.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
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
    let Some(words) = words::parse(args, Some(ZEEWIN2.name))? else {
        return Ok(Parsed::Help);
    };
    Ok(Parsed::Run(Options {
        cargo_args: words.cargo_args,
        installer_smoke: words.installer_smoke,
        wrapper: words.wrapper,
        clippy: words.clippy,
        published: words.published,
        unpinned: words.unpinned,
        wait: words.wait,
    }))
}

/// The `--wrapper` file as it goes in the tarball: a `.ps1`, read whole.
pub fn wrapper_file(given: &str) -> Result<(String, String), String> {
    words::wrapper_file(given, "ps1")
}
