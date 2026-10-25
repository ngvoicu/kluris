//! Python's float formatting: `repr(x)` and `json.dumps(x)`.
//!
//! Every expected string is the output of `repr(x)` and `json.dumps(x)` in
//! the oracle venv (Python 3.11). `parity/test_float_differential.py`
//! compares 20,000+ more against the oracle.

use kluris_core::pyrepr::{float_repr, json_float};

/// `(value, repr(value), json.dumps(value))`.
const CASES: &[(f64, &str, &str)] = &[
    // exact ties at the shortest length: Python rounds half to even
    (70368744177664.12, "70368744177664.12", "70368744177664.12"),
    (
        2.9802322387695312e-08,
        "2.9802322387695312e-08",
        "2.9802322387695312e-08",
    ),
    (
        0.0018625259399414062,
        "0.0018625259399414062",
        "0.0018625259399414062",
    ),
    // positional between 1e-4 and 1e16, exponent form outside
    (1.0, "1.0", "1.0"),
    (0.1, "0.1", "0.1"),
    (2.5, "2.5", "2.5"),
    (0.5, "0.5", "0.5"),
    (0.0001, "0.0001", "0.0001"),
    (0.00011, "0.00011", "0.00011"),
    (1e-5, "1e-05", "1e-05"),
    (1.5e-7, "1.5e-07", "1.5e-07"),
    (1e15, "1000000000000000.0", "1000000000000000.0"),
    (
        9999999999999998.0,
        "9999999999999998.0",
        "9999999999999998.0",
    ),
    (
        9007199254740993.0,
        "9007199254740992.0",
        "9007199254740992.0",
    ),
    (1e16, "1e+16", "1e+16"),
    (1e20, "1e+20", "1e+20"),
    (
        123456789012345678.0,
        "1.2345678901234568e+17",
        "1.2345678901234568e+17",
    ),
    (
        1.7976931348623157e308,
        "1.7976931348623157e+308",
        "1.7976931348623157e+308",
    ),
    // subnormals and the normal boundary
    (5e-324, "5e-324", "5e-324"),
    (
        2.2250738585072014e-308,
        "2.2250738585072014e-308",
        "2.2250738585072014e-308",
    ),
    (
        2.225073858507201e-308,
        "2.225073858507201e-308",
        "2.225073858507201e-308",
    ),
    // signs and specials
    (-0.0, "-0.0", "-0.0"),
    (-1.5, "-1.5", "-1.5"),
    (f64::INFINITY, "inf", "Infinity"),
    (f64::NEG_INFINITY, "-inf", "-Infinity"),
    (f64::NAN, "nan", "NaN"),
];

#[test]
fn float_repr_matches_python() {
    let failures: Vec<String> = CASES
        .iter()
        .filter(|(x, repr, _)| float_repr(*x) != *repr)
        .map(|(x, repr, _)| format!("{x:?}: expected {repr}, got {}", float_repr(*x)))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn json_float_matches_json_dumps() {
    let failures: Vec<String> = CASES
        .iter()
        .filter(|(x, _, json)| json_float(*x) != *json)
        .map(|(x, _, json)| format!("{x:?}: expected {json}, got {}", json_float(*x)))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn negative_nan_is_still_nan() {
    assert_eq!(float_repr(-f64::NAN), "nan");
    assert_eq!(json_float(-f64::NAN), "NaN");
}
