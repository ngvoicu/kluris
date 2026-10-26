//! `cargo xtask linux`: the workspace's tests, and with `--parity` the parity
//! harness, on the household laptop's Ubuntu `zeelin` (WSL2), over ssh, the
//! way `cargo xtask windows` runs them on zeewin and zeewin2.
//!
//! ```text
//! cargo xtask linux [--parity [--parity-files f.py[,...]]] [-- <extra cargo test args>]
//! ```
//!
//! A run, in order:
//!
//! 1. The probe ([`script::probe`]), one bounded ssh that sends nothing: zeelin
//!    not answering ends the command with 75 ("zeelin not running"; it is never
//!    started from here), and so does any of the three locks held, named:
//!    poker-lab's `~/.compile-lock/owner` (zeelin is poker-lab's compute box
//!    first), the record of a run holding zeewin2 (the same laptop's Windows,
//!    `/mnt/c/Users/AGV/consensflow-build/.windows-run.id`, written by
//!    ConsensFlow's and Kluris's `cargo xtask windows`), and Kluris's own
//!    one-run lock `~/.kluris-ci/.linux-run.lock`. Locks are only looked at:
//!    never taken (but Kluris's own), broken or waited on.
//! 2. The tarball ([`crate::ssh_ci::pack`]): `git archive` of HEAD, or of a
//!    `git stash create` commit when tracked files have uncommitted changes,
//!    with the run script beside the files, sent by scp to
//!    `~/.kluris-ci/incoming/<run id>.tgz` (removed there at once when the
//!    upload fails).
//! 3. The bootstrap ([`script::bootstrap`]), in a session of its own: Kluris's
//!    lock taken with `flock -n` for the whole run, the three locks asked again
//!    under it (75 at once while one is held), the run's record written, the
//!    tarball unpacked into `~/.kluris-ci/runs/<run id>` and the run script
//!    sourced. Its `trap` ends every other process of the run's session and
//!    removes the tarball and the record, however the run ends.
//! 4. The run script: `ulimit -u` caps the user's tasks while the run goes,
//!    run folders and tarballs older than a day are swept, then each step under
//!    `timeout`: the tree's Rust toolchain into Kluris's own `RUSTUP_HOME`
//!    (with the user's `~/.cargo/bin/rustup` by its full path), `cargo test
//!    --workspace --no-fail-fast`, and with `--parity` uv (fetched once, its
//!    hash checked), a release `kluris` and the parity harness with
//!    `KLURIS_V3_BIN` pointing at it. Everything lives under `~/.kluris-ci/`;
//!    PATH changes for the run's own process only, and no profile is touched.
//! 5. The output streams back as it comes; the status is the run's last line
//!    (`== exit N`), and a summary line says each step's. A run stopped here
//!    (Ctrl-C: 130), past its bound (124) or whose connection was lost is ended
//!    there by the stop script.

mod options;
mod run;
pub(crate) mod script;
#[cfg(test)]
mod tests;

use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ssh_ci::{pack, runner};
use options::Parsed;

/// `cargo xtask linux <args>`.
pub fn main(args: &[String]) -> ExitCode {
    let options = match options::parse(args) {
        Ok(Parsed::Run(options)) => options,
        Ok(Parsed::Help) => {
            println!("{}", options::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("linux: {why}\n\n{}", options::USAGE);
            return ExitCode::from(2);
        }
    };
    let root = workspace_root();
    let channel = match pack::channel(root) {
        Ok(channel) => channel,
        Err(why) => {
            eprintln!("linux: {why}");
            return ExitCode::from(1);
        }
    };
    let stopped = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        if let Err(why) = signal_hook::flag::register(signal, Arc::clone(&stopped)) {
            eprintln!("linux: could not listen for a stop: {why}");
            return ExitCode::from(1);
        }
    }
    let stdout = io::stdout();
    let stderr = io::stderr();
    let files = match options.wrapper.as_deref().map(options::wrapper_file) {
        None => vec![],
        Some(Ok(file)) => vec![file],
        Some(Err(why)) => {
            eprintln!("linux: {why}");
            return ExitCode::from(2);
        }
    };
    let mut pack = |tarball: &Path, run_script: &str| {
        pack::pack_with_files(root, tarball, script::RUN_FILE, run_script, &files)
    };
    let mut ctx = run::Ctx {
        runner: &mut runner::System,
        out: &mut stdout.lock(),
        err: &mut stderr.lock(),
        stop: &|| stopped.load(Ordering::SeqCst),
        channel,
        pack: &mut pack,
        root,
    };
    let code = run::linux(&options, &mut ctx);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// The worktree this xtask was built from.
fn workspace_root() -> &'static Path {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest)
}
