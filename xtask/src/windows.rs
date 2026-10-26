//! `cargo xtask windows`: the workspace's tests, and with `--parity` the
//! parity harness, on the household Windows machines `zeewin` and `zeewin2`,
//! over ssh, the way ConsensFlow's `cargo xtask windows` runs its own.
//!
//! ```text
//! cargo xtask windows --machine zeewin|zeewin2 [--parity [--parity-files f.py[,...]]] [-- <extra cargo test args>]
//! ```
//!
//! A run, in order:
//!
//! 1. The machine: the one `--machine` names, or zeewin2 while it is free and
//!    allowed, else zeewin ([`machine`]). zeewin2 is asked first (bounded probes)
//!    and refused with 75 while poker-lab holds the laptop or C: is short of
//!    disk, having sent nothing.
//! 2. The tarball ([`pack`]): `git archive` of HEAD, or of a `git stash create`
//!    commit when tracked files have uncommitted changes, without `target/`,
//!    `parity/.oracle/` and `parity/.venv/`, with the run script beside the
//!    files. It is sent by scp to `%USERPROFILE%\<run id>.tgz`; when the upload
//!    fails, that one file is removed there at once (a bounded ssh that takes
//!    no lock, the name being the run's own), and the command says whether it
//!    was removed, was not there, or the machine could not be reached.
//! 3. The bootstrap ([`script`]), on ssh's command line: it takes the
//!    per-machine one-run lock it shares with ConsensFlow
//!    (`%USERPROFILE%\consensflow-build\.windows-run.lock`, opened for the run
//!    alone, with ConsensFlow's records beside it) before anything else, and
//!    refuses a run at once with 75 naming the holder while another run of
//!    either project holds it: it never waits, takes or breaks a held lock. Then
//!    the run's folder `%USERPROFILE%\.kluris-ci\runs\<run id>`, the unpack, and
//!    the run script. Its `finally` removes the tarball and its lock records and
//!    closes the lock, however the run ends.
//! 4. The run script: the run in a job (at most 64 processes, all ended with the
//!    run), run folders older than a day swept, then each step with its own
//!    bound: the tree's Rust toolchain into Kluris's own `RUSTUP_HOME`,
//!    `cargo test --workspace --no-fail-fast`, and with `--parity` uv (fetched once, its hash
//!    checked), a release `kluris.exe` and the parity harness with
//!    `KLURIS_V3_BIN` pointing at it (`--parity-files` narrows the harness to
//!    those `parity/*.py` files, checked against the worktree before anything
//!    is sent). Tools and caches live under
//!    `%USERPROFILE%\.kluris-ci\` (`tools\`, `cache\target` as
//!    `CARGO_TARGET_DIR`, `cache\oracle`, ...); PATH changes for the run's own
//!    process only.
//! 5. The output streams back as it comes. The status is the machine's last
//!    line (`== exit N`, [`run::answer`]), a summary line says each step's, and a
//!    run stopped here (Ctrl-C: 130), past its bound (124) or whose connection
//!    was lost is ended there by the stop script.

mod machine;
mod options;
mod pack;
mod run;
mod script;
#[cfg(test)]
mod tests;

use std::io;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ssh_ci::runner as remote;
use options::Parsed;

/// `cargo xtask windows <args>`.
pub fn main(args: &[String]) -> ExitCode {
    let options = match options::parse(args) {
        Ok(Parsed::Run(options)) => options,
        Ok(Parsed::Help) => {
            println!("{}", options::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("windows: {why}\n\n{}", options::USAGE);
            return ExitCode::from(2);
        }
    };
    let root = workspace_root();
    let channel = match pack::channel(root) {
        Ok(channel) => channel,
        Err(why) => {
            eprintln!("windows: {why}");
            return ExitCode::from(1);
        }
    };
    let stopped = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        if let Err(why) = signal_hook::flag::register(signal, Arc::clone(&stopped)) {
            eprintln!("windows: could not listen for a stop: {why}");
            return ExitCode::from(1);
        }
    }
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut pack = |tarball: &Path, run_script: &str| pack::pack(root, tarball, run_script);
    let mut ctx = run::Ctx {
        runner: &mut remote::System,
        out: &mut stdout.lock(),
        err: &mut stderr.lock(),
        stop: &|| stopped.load(Ordering::SeqCst),
        channel,
        pack: &mut pack,
        root,
    };
    let code = run::windows(&options, &mut ctx);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// The worktree this xtask was built from.
fn workspace_root() -> &'static Path {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest)
}
