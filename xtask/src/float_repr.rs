//! `float-repr`: the Rust side of the float differential test
//! (`parity/test_float_differential.py`).

use std::io::{self, BufRead, BufWriter, Write};

use kluris_core::pyrepr::{float_repr, json_float};

/// Read one IEEE 754 bit pattern per stdin line (16 hex digits) and answer
/// `repr(x)`, a tab and `json.dumps(x)`, as Python writes them.
pub fn main(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("float-repr takes no arguments".into());
    }
    let mut out = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("cannot read stdin: {e}"))?;
        let bits = u64::from_str_radix(line.trim(), 16)
            .map_err(|e| format!("bad bit pattern '{line}': {e}"))?;
        let x = f64::from_bits(bits);
        writeln!(out, "{}\t{}", float_repr(x), json_float(x))
            .map_err(|e| format!("cannot write stdout: {e}"))?;
    }
    out.flush().map_err(|e| format!("cannot write stdout: {e}"))
}
