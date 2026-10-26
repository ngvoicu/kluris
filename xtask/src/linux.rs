//! `cargo xtask linux` (being written).

mod options;
mod run;
mod script;
#[cfg(test)]
mod tests;

use std::process::ExitCode;

pub fn main(_args: &[String]) -> ExitCode {
    ExitCode::from(1)
}
