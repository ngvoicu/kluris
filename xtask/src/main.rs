//! Development tasks for the Kluris 3 workspace (`cargo xtask <task>`).

mod linux;
mod search_eval;
mod ssh_ci;
mod sync_specmint;
mod windows;

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>

Tasks:
  linux [--wait MINUTES] [-- <cargo test args>]
      Run cargo test on zeelin, the household laptop's Ubuntu, over ssh,
      after its turn in the machines' shared queue. `cargo xtask linux
      --help` says more.
  search-eval [--local] [--queries FILE] [--bin KLURIS] [--seed N] [--repeat N] [--report FILE]
      The search-quality eval through the real `kluris search`: the public
      fixture (xtask/fixtures/search-eval/, gated at hit@5 0.90 and MRR
      0.74) or, with --local, the tuning queries against the registered
      brains. Writes a text-free JSON report under target/search-eval/.
  sync-specmint [--core PATH] [--tdd PATH] [--core-html PATH] [--tdd-html PATH] [--check] [--diff]
      Re-vendor the four Specmint companion folders into vendored/ from
      ../specmint/<name> (or the paths given): SKILL.md, references/ and
      assets/ only, SKILL.md sanitized for Kluris (each rule must match
      once), checked for plugin text and for mentioned files that are not
      shipped. --check writes nothing and exits 1 when vendored/ is stale;
      --diff prints the full diffs.
  windows [--machine zeewin|zeewin2] [--wait MINUTES] [-- <cargo test args>]
      Run cargo test on a household Windows machine over ssh, after its
      turn in the machines' shared queue. `cargo xtask windows --help` says
      more.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("linux") => return linux::main(&args[1..]),
        Some("search-eval") => search_eval::main(&args[1..]),
        Some("sync-specmint") => return sync_specmint::main(&args[1..]),
        Some("windows") => return windows::main(&args[1..]),
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
