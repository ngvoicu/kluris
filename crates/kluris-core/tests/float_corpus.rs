//! Python float formatting against the frozen float corpus
//! (`fixtures/corpus/floats.tsv`): IEEE 754 bit patterns with Python's
//! `repr(x)` and `json.dumps(x)` of each. The patterns are random ones,
//! subnormals, exact half-way ties at the shortest length (where Rust's
//! shortest formatting rounds up and Python rounds half to even), few-bit
//! mantissas at every exponent, powers of 2 and 10 with their neighbours, the
//! positional and exponent boundaries (1e-4/1e-5, 1e16), and the specials.

mod common;

use common::corpus::{self, Failures};
use kluris_core::pyrepr::{float_repr, json_float};
use num_bigint::BigUint;

const MIN_ROWS: usize = 20_000;
const MIN_TIES: usize = 100;

struct Row {
    bits: u64,
    repr: String,
    json: String,
}

/// The rows, and the number of ties the header counts.
fn corpus() -> (Vec<Row>, usize) {
    let (header, rows) = corpus::tsv("floats.tsv");
    let ties = header
        .iter()
        .find_map(|l| l.strip_prefix("ties: "))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("floats.tsv: the header has no tie count"));
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(n, fields)| match &fields[..] {
            [bits, repr, json] => Row {
                bits: u64::from_str_radix(bits, 16)
                    .unwrap_or_else(|e| panic!("row {}: bad bits {bits:?}: {e}", n + 1)),
                repr: repr.clone(),
                json: json.clone(),
            },
            _ => panic!("row {}: not bits, repr and json", n + 1),
        })
        .collect();
    (rows, ties)
}

/// The exact decimal digits of a finite float, zeros stripped at both ends
/// (Python's `format(Decimal(x), "f")` without its point).
fn exact_digits(x: f64) -> String {
    let bits = x.abs().to_bits();
    let exponent = (bits >> 52) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, power) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), exponent - 1075)
    };
    let mantissa = BigUint::from(mantissa);
    let digits = if power >= 0 {
        mantissa << power as usize
    } else {
        // m / 2^k is m * 5^k / 10^k: the same digits.
        mantissa * BigUint::from(5u32).pow(power.unsigned_abs())
    };
    digits.to_string().trim_matches('0').to_string()
}

/// The exact value is half-way between two candidates of the shortest
/// length: Python's `repr` takes the even one.
fn is_tie(x: f64, repr: &str) -> bool {
    if !x.is_finite() || x == 0.0 {
        return false;
    }
    let mantissa = repr.trim_start_matches('-').split('e').next().unwrap_or("");
    let shortest = mantissa.replace('.', "");
    let shortest = shortest.trim_start_matches('0');
    let exact = exact_digits(x);
    exact.len() == shortest.len() + 1 && exact.ends_with('5')
}

#[test]
fn the_corpus_holds_enough_patterns_and_ties() {
    let (rows, counted) = corpus();
    let ties = rows
        .iter()
        .filter(|r| is_tie(f64::from_bits(r.bits), &r.repr))
        .count();
    assert!(rows.len() >= MIN_ROWS, "only {} patterns", rows.len());
    assert_eq!(ties, counted, "the header counts {counted} ties");
    assert!(ties >= MIN_TIES, "the corpus holds only {ties} ties");
}

#[test]
fn every_float_is_written_as_python_writes_it() {
    let (rows, _) = corpus();
    let mut failures = Failures::default();
    for (n, row) in rows.iter().enumerate() {
        let x = f64::from_bits(row.bits);
        let (repr, json) = (float_repr(x), json_float(x));
        if repr != row.repr || json != row.json {
            failures.add(
                n + 1,
                format!(
                    "{:016x}: Python {} / {}, Rust {repr} / {json}",
                    row.bits, row.repr, row.json
                ),
            );
        }
    }
    failures.assert_none(rows.len());
}
