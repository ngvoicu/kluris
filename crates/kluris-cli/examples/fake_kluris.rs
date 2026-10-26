//! A stand-in for a release's new `kluris.exe` in the self-update tests on
//! Windows (on Unix they use a shell script, `tests/common/mod.rs`). It is
//! never shipped: dist packages the `kluris` binary only.
//!
//! It reads what to do from a trailer the tests append to its own file
//! (`\n#KLURIS-FAKE#` then one JSON object), so one build serves every
//! release in a test:
//! - `--version` alone: print `version_line`, exit `version_code`;
//! - anything else: write how it was started (the program as given, then
//!   the arguments), one per line, to `args_file`, print `out`, exit `code`.

use std::io::Write;

const MARKER: &[u8] = b"\n#KLURIS-FAKE#";

fn main() {
    let exe = std::env::current_exe().unwrap_or_default();
    let bytes = std::fs::read(&exe).unwrap_or_default();
    let config: serde_json::Value = bytes
        .windows(MARKER.len())
        .rposition(|w| w == MARKER)
        .and_then(|at| serde_json::from_slice(&bytes[at + MARKER.len()..]).ok())
        .unwrap_or(serde_json::Value::Null);
    let text = |key: &str| config[key].as_str().unwrap_or_default().to_string();
    let code = |key: &str| config[key].as_i64().unwrap_or(0) as i32;
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--version" {
        println!("{}", text("version_line"));
        std::process::exit(code("version_code"));
    }
    let record = args.iter().map(|l| format!("{l}\n")).collect::<String>();
    let _ = std::fs::write(text("args_file"), record);
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", text("out"));
    std::process::exit(code("code"));
}
