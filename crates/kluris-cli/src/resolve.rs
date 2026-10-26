//! The resolver's terminal side: the environment and stdin decide whether the
//! picker may run (`--json`, `KLURIS_NO_PROMPT` exactly `1`, stdin not a
//! terminal), and the picker reads stdin and writes stdout, with Ctrl-C
//! ending it as Click's `Abort` does. The decisions are
//! `kluris_core::resolve`'s.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use kluris_core::config::{global_config_path, read_global_config};
use kluris_core::resolve::{Request, Resolution, ResolveError, ResolvedBrain, resolve};

use crate::error::CliError;

/// The global config path, as every command finds it.
pub fn config_path() -> Result<PathBuf, CliError> {
    global_config_path(
        std::env::var_os("KLURIS_CONFIG").as_deref(),
        std::env::var_os("HOME").as_deref(),
        std::env::home_dir(),
    )
    .ok_or_else(|| CliError::new("Cannot find the home directory; set HOME or KLURIS_CONFIG."))
}

fn cli_error(err: ResolveError) -> CliError {
    match err {
        ResolveError::Aborted => CliError::aborted(),
        other => CliError::new(other.to_string()),
    }
}

/// The brains a `--brain` command works on, in config order.
pub fn resolve_brains(
    brain: Option<&str>,
    allow_all: bool,
    as_json: bool,
) -> Result<Vec<ResolvedBrain>, CliError> {
    let config = read_global_config(&config_path()?).map_err(|e| CliError::new(e.to_string()))?;
    let no_prompt = std::env::var_os("KLURIS_NO_PROMPT").is_some_and(|v| v == "1");
    let request = Request {
        brain,
        allow_all,
        non_interactive: as_json || no_prompt || !io::stdin().is_terminal(),
    };
    match resolve(&config, &request).map_err(cli_error)? {
        Resolution::Brains(brains) => Ok(brains),
        Resolution::Pick(picker) => {
            let guard = AbortOnInterrupt::install();
            let picked = picker.run(&mut io::stdin().lock(), &mut io::stdout().lock());
            drop(guard);
            picked.map_err(cli_error)
        }
    }
}

/// While alive, Ctrl-C (SIGINT) prints `Aborted!` on stderr and exits 1,
/// which is what Click does when it interrupts a prompt.
pub struct AbortOnInterrupt(Option<signal_hook::SigId>);

impl AbortOnInterrupt {
    pub fn install() -> Self {
        // SAFETY: the handler writes to stderr, which is unbuffered and not
        // in use (the main thread is blocked reading stdin), then leaves
        // with `_exit`, which is async-signal-safe.
        let id = unsafe {
            signal_hook::low_level::register(signal_hook::consts::SIGINT, || {
                let _ = io::stderr().write_all(b"Aborted!\n");
                signal_hook::low_level::exit(1);
            })
        };
        Self(id.ok())
    }
}

impl Drop for AbortOnInterrupt {
    fn drop(&mut self) {
        if let Some(id) = self.0.take() {
            signal_hook::low_level::unregister(id);
        }
    }
}
