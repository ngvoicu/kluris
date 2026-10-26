//! Development tasks for the Kluris 3 workspace (`cargo xtask <task>`).

mod brain_dump;
mod config_rewrite;
mod float_repr;
mod search_eval;
mod windows;
mod yaml_emit;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>

Tasks:
  brain-dump --path <brain>
      Print the brain model of a brain as JSON: its neurons, glossary
      entries, deprecation issues and the neurons that could not be read.
  config-rewrite global <config.yml> | brain <brain dir>
      Read a config with kluris-core and write it back (for the parity
      harness: no kluris command writes a config yet).
  float-repr
      Read one f64 bit pattern (16 hex digits) per stdin line and answer each
      with Python's repr(x), a tab and json.dumps(x).
  search-eval [--local] [--queries FILE] [--bin KLURIS] [--seed N] [--repeat N] [--report FILE]
      The search-quality eval through the real `kluris search`: the public
      fixture (parity/fixtures/search-eval/, gated at hit@5 0.90 and MRR
      0.74) or, with --local, the tuning queries against the registered
      brains. Writes a text-free JSON report under target/search-eval/.
  windows [--machine zeewin|zeewin2] [--parity] [-- <cargo test args>]
      Run cargo test (and with --parity the parity harness) on a household
      Windows machine over ssh. `cargo xtask windows --help` says more.
  yaml-emit --profile md|yml|config
      Read one tagged-JSON mapping per stdin line and answer each with the
      YAML Kluris writes for it, as one JSON string per stdout line.
  yaml-load
      Read one JSON string of YAML per stdin line and answer each with
      {\"value\": <tagged JSON>} or {\"error\": <message>}.

Tagged JSON: parity/tools/tagged_json.py.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("brain-dump") => brain_dump::main(&args[1..]),
        Some("config-rewrite") => config_rewrite::main(&args[1..]),
        Some("float-repr") => float_repr::main(&args[1..]),
        Some("search-eval") => search_eval::main(&args[1..]),
        Some("windows") => return windows::main(&args[1..]),
        Some("yaml-emit") => yaml_emit::emit(&args[1..]),
        Some("yaml-load") => yaml_emit::load(&args[1..]),
        Some(task) => Err(format!("unknown task '{task}'")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
