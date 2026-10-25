//! Python's float formatting, which v2's output carries: `repr(x)` (PyYAML's
//! float representer builds on it) and `json.dumps(x)`.

/// Python's `float.__repr__`: the shortest digits that read back as the same
/// float (the closest such digits, an exact tie rounded half to even),
/// positional when the decimal exponent is in -4..16, else `d.ddde+XX`;
/// `inf`, `-inf` and `nan` (whatever its sign).
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.into();
    }
    // `{:e}` gives the shortest round-trip digits (`1.2345e-7`), but rounds an
    // exact tie up where Python takes the even neighbour (70368744177664.125
    // is `...64.12`). The exact mode rounds half to even: use it at the same
    // length whenever it still reads back as the same float.
    let shortest = format!("{:e}", value.abs());
    let length = shortest
        .split('e')
        .next()
        .map_or(1, |m| m.replace('.', "").len());
    let even = format!("{:.*e}", length.saturating_sub(1), value.abs());
    let scientific = if even.parse::<f64>() == Ok(value.abs()) {
        even
    } else {
        shortest
    };
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    // The value is 0.<digits> x 10^decpt.
    let decpt = exponent + 1;
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let len = digits.len() as i32;
    let body = if decpt <= -4 || decpt > 16 {
        let (first, rest) = digits.split_at(1);
        let dot = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp = decpt - 1;
        let exp_sign = if exp < 0 { '-' } else { '+' };
        format!("{first}{dot}e{exp_sign}{:02}", exp.unsigned_abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat(decpt.unsigned_abs() as usize))
    } else if decpt >= len {
        format!("{digits}{}.0", "0".repeat((decpt - len) as usize))
    } else {
        let (int, frac) = digits.split_at(decpt as usize);
        format!("{int}.{frac}")
    };
    format!("{sign}{body}")
}

/// `json.dumps(x)` for a float: `float.__repr__`, with `Infinity`,
/// `-Infinity` and `NaN` for the specials.
pub fn json_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        float_repr(value)
    }
}
