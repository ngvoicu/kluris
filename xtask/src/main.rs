//! Development tasks for the Kluris 3 workspace (`cargo xtask <task>`).

use std::process::ExitCode;

const USAGE: &str = "usage: cargo xtask <task>\n\nNo tasks yet.";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(task) => {
            eprintln!("unknown task '{task}'\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
