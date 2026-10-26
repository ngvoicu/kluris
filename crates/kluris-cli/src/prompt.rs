//! Asking on the terminal: whether a prompt may run at all (v2's
//! `_wizard_can_prompt`), and running one with Ctrl-C and end of input
//! aborting as Click does. The questions themselves are `kluris-core`'s.

use std::io::{self, IsTerminal};

use kluris_core::companions::prompt::Aborted;

use crate::error::CliError;
use crate::resolve::AbortOnInterrupt;

/// Not `--json`, `KLURIS_NO_PROMPT` not `1`, and stdin a terminal.
pub fn can_prompt(as_json: bool) -> bool {
    let no_prompt = std::env::var_os("KLURIS_NO_PROMPT").is_some_and(|v| v == "1");
    !as_json && !no_prompt && io::stdin().is_terminal()
}

/// Run `question` on stdin and stdout; Ctrl-C ends the process as Click's
/// `Abort` does, end of input is [`Aborted`].
pub fn on_terminal<T>(
    question: impl FnOnce(&mut io::StdinLock, &mut io::StdoutLock) -> Result<T, Aborted>,
) -> Result<T, Aborted> {
    let guard = AbortOnInterrupt::install();
    let answer = question(&mut io::stdin().lock(), &mut io::stdout().lock());
    drop(guard);
    answer
}

/// [`on_terminal`], end of input printing `Aborted!` (exit 1).
pub fn ask<T>(
    question: impl FnOnce(&mut io::StdinLock, &mut io::StdoutLock) -> Result<T, Aborted>,
) -> Result<T, CliError> {
    on_terminal(question).map_err(|Aborted| CliError::aborted())
}
