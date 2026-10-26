//! Ported from PyYAML 6.0.3 `resolver.py` and `constructor.py`; see
//! LICENSE-PyYAML.
//!
//! PyYAML 6.0.3's implicit resolvers (`resolver.py`) and scalar constructors
//! (`constructor.py`, `SafeConstructor`).
//!
//! The patterns are PyYAML's, written without verbose mode: Rust's `(?x)`
//! also drops whitespace inside character classes (`[ \t]`), Python's does
//! not.

use std::sync::LazyLock;

use num_bigint::BigInt;
use regex::Regex;

use super::value::{Date, DateTime, Value};

fn pattern(re: &str) -> Regex {
    // The patterns are constants covered by tests; a typo fails every test.
    Regex::new(re).unwrap_or_else(|e| panic!("invalid resolver pattern {re}: {e}"))
}

static BOOL: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$")
});

static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?",
        r"|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?",
        r"|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*",
        r"|[-+]?\.(?:inf|Inf|INF)",
        r"|\.(?:nan|NaN|NAN))$",
    ))
});

static INT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"^(?:[-+]?0b[0-1_]+",
        r"|[-+]?0[0-7_]+",
        r"|[-+]?(?:0|[1-9][0-9_]*)",
        r"|[-+]?0x[0-9a-fA-F_]+",
        r"|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$",
    ))
});

static MERGE: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(?:<<)$"));

static NULL: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(?:~|null|Null|NULL|)$"));

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"^(?:[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]",
        r"|[0-9][0-9][0-9][0-9]-[0-9][0-9]?-[0-9][0-9]?",
        r"(?:[Tt]|[ \t]+)[0-9][0-9]?",
        r":[0-9][0-9]:[0-9][0-9](?:\.[0-9]*)?",
        r"(?:[ \t]*(?:Z|[-+][0-9][0-9]?(?::[0-9][0-9])?))?)$",
    ))
});

static VALUE: LazyLock<Regex> = LazyLock::new(|| pattern(r"^(?:=)$"));

/// `SafeConstructor.timestamp_regexp`, used to take a matched timestamp apart.
static TIMESTAMP_PARTS: LazyLock<Regex> = LazyLock::new(|| {
    pattern(concat!(
        r"^(?P<year>[0-9][0-9][0-9][0-9])",
        r"-(?P<month>[0-9][0-9]?)",
        r"-(?P<day>[0-9][0-9]?)",
        r"(?:(?:[Tt]|[ \t]+)",
        r"(?P<hour>[0-9][0-9]?)",
        r":(?P<minute>[0-9][0-9])",
        r":(?P<second>[0-9][0-9])",
        r"(?:\.(?P<fraction>[0-9]*))?",
        r"(?:[ \t]*(?P<tz>Z|(?P<tz_sign>[-+])(?P<tz_hour>[0-9][0-9]?)",
        r"(?::(?P<tz_minute>[0-9][0-9]))?))?)?$",
    ))
});

/// The core scalar tags inside the value model (`!!str`, `!!int`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarTag {
    Str,
    Int,
    Float,
    Bool,
    Null,
    Timestamp,
}

/// Python's `str.isspace()`: what `strip()`, `int()` and `float()` skip.
pub(crate) fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// The text a Python `^...$` pattern sees: `$` also matches just before a
/// final `\n` (reachable through `!` on a block scalar).
fn dollar(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

/// The tag PyYAML's implicit resolvers give a plain scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImplicitTag {
    Bool,
    Float,
    Int,
    Merge,
    Null,
    Timestamp,
    Value,
    Str,
}

/// `Resolver.resolve` for a plain scalar: the resolvers registered for the
/// first character (`''` for the empty string), in registration order.
pub(super) fn implicit_tag(text: &str) -> ImplicitTag {
    use ImplicitTag::*;
    let candidates: &[ImplicitTag] = match text.chars().next() {
        None => &[Null],
        Some('y' | 'Y' | 't' | 'T' | 'f' | 'F' | 'o' | 'O') => &[Bool],
        Some('n' | 'N') => &[Bool, Null],
        Some('0'..='9') => &[Float, Int, Timestamp],
        Some('-' | '+') => &[Float, Int],
        Some('.') => &[Float],
        Some('<') => &[Merge],
        Some('~') => &[Null],
        Some('=') => &[Value],
        Some(_) => &[],
    };
    let probe = dollar(text);
    candidates
        .iter()
        .copied()
        .find(|tag| {
            let pattern: &Regex = match tag {
                Bool => &BOOL,
                Float => &FLOAT,
                Int => &INT,
                Merge => &MERGE,
                Null => &NULL,
                Timestamp => &TIMESTAMP,
                Value => &VALUE,
                Str => return false,
            };
            pattern.is_match(probe)
        })
        .unwrap_or(Str)
}

/// Construct a scalar explicitly tagged with a core tag, whatever its style.
pub fn tagged_scalar(tag: ScalarTag, text: &str) -> Result<Value, String> {
    match tag {
        ScalarTag::Str => Ok(Value::Str(text.to_string())),
        ScalarTag::Int => construct_int(text),
        ScalarTag::Float => construct_float(text),
        ScalarTag::Bool => construct_bool(text),
        ScalarTag::Null => Ok(Value::Null),
        ScalarTag::Timestamp => construct_timestamp(text),
    }
}

/// `SafeConstructor.construct_yaml_bool`.
fn construct_bool(text: &str) -> Result<Value, String> {
    match text.to_lowercase().as_str() {
        "yes" | "true" | "on" => Ok(Value::Bool(true)),
        "no" | "false" | "off" => Ok(Value::Bool(false)),
        _ => Err(format!("invalid boolean '{text}'")),
    }
}

/// Split one leading sign off, like the constructors do.
fn sign(value: &str) -> (bool, &str) {
    match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    }
}

/// Unicode 14.0.0: generated by `parity/tools/unicode_digits.py`.
const DIGIT_ZEROS: [u32; 66] = [
    0x00030, 0x00660, 0x006F0, 0x007C0, 0x00966, 0x009E6, 0x00A66, 0x00AE6, 0x00B66, 0x00BE6,
    0x00C66, 0x00CE6, 0x00D66, 0x00DE6, 0x00E50, 0x00ED0, 0x00F20, 0x01040, 0x01090, 0x017E0,
    0x01810, 0x01946, 0x019D0, 0x01A80, 0x01A90, 0x01B50, 0x01BB0, 0x01C40, 0x01C50, 0x0A620,
    0x0A8D0, 0x0A900, 0x0A9D0, 0x0A9F0, 0x0AA50, 0x0ABF0, 0x0FF10, 0x104A0, 0x10D30, 0x11066,
    0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650, 0x116C0, 0x11730, 0x118E0,
    0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x16A60, 0x16AC0, 0x16B50, 0x1D7CE, 0x1D7D8, 0x1D7E2,
    0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E950, 0x1FBF0,
];

/// What Python's `int()` and `float()` parse: every Unicode decimal digit as
/// its ASCII digit and Unicode whitespace as a space
/// (`_PyUnicode_TransformDecimalAndSpaceToASCII`). `None` when another
/// non-ASCII character is left, which neither accepts.
fn py_ascii_number(text: &str) -> Option<String> {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                return Some(c);
            }
            if is_py_space(c) {
                return Some(' ');
            }
            let code = u32::from(c);
            let zero = DIGIT_ZEROS.iter().rev().find(|&&zero| zero <= code)?;
            let digit = u8::try_from(code - zero).ok().filter(|d| *d < 10)?;
            Some(char::from(b'0' + digit))
        })
        .collect()
}

/// Python's `int(text, radix)` for the radixes PyYAML uses: surrounding
/// whitespace, one sign and (radix 2, 8, 16) the matching `0b`/`0o`/`0x`
/// prefix are accepted, and any Unicode decimal digit.
fn py_int(text: &str, radix: u32, original: &str) -> Result<BigInt, String> {
    let invalid = || format!("invalid integer '{original}'");
    let ascii = py_ascii_number(text).ok_or_else(invalid)?;
    let (negative, digits) = sign(ascii.trim_matches(is_py_space));
    let prefix = match radix {
        2 => Some("0b"),
        8 => Some("0o"),
        16 => Some("0x"),
        _ => None,
    };
    let digits = match prefix {
        Some(prefix)
            if digits
                .get(..2)
                .is_some_and(|p| p.eq_ignore_ascii_case(prefix)) =>
        {
            &digits[2..]
        }
        _ => digits,
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(invalid());
    }
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), radix).ok_or_else(invalid)?;
    Ok(if negative { -magnitude } else { magnitude })
}

/// `SafeConstructor.construct_yaml_int`.
fn construct_int(text: &str) -> Result<Value, String> {
    let cleaned = text.replace('_', "");
    if cleaned.is_empty() {
        return Err(format!("invalid integer '{text}'"));
    }
    let (negative, value) = sign(&cleaned);
    let magnitude = if value == "0" {
        BigInt::ZERO
    } else if let Some(bin) = value.strip_prefix("0b") {
        py_int(bin, 2, text)?
    } else if let Some(hex) = value.strip_prefix("0x") {
        py_int(hex, 16, text)?
    } else if value.starts_with('0') {
        py_int(value, 8, text)?
    } else if value.contains(':') {
        let mut total = BigInt::ZERO;
        for part in value.split(':') {
            total = total * 60 + py_int(part, 10, text)?;
        }
        total
    } else {
        py_int(value, 10, text)?
    };
    Ok(Value::Int(if negative { -magnitude } else { magnitude }))
}

/// Python's `float(text)`: surrounding whitespace, a sign, `inf`, `infinity`
/// and `nan` are accepted, and any Unicode decimal digit.
fn py_float(text: &str, original: &str) -> Result<f64, String> {
    let invalid = || format!("invalid float '{original}'");
    // For ASCII text Rust's `f64` grammar is Python's `float()` grammar.
    py_ascii_number(text)
        .ok_or_else(invalid)?
        .trim_matches(is_py_space)
        .parse::<f64>()
        .map_err(|_| invalid())
}

/// `SafeConstructor.construct_yaml_float`.
fn construct_float(text: &str) -> Result<Value, String> {
    let cleaned = text.replace('_', "").to_lowercase();
    if cleaned.is_empty() {
        return Err(format!("invalid float '{text}'"));
    }
    let (negative, value) = sign(&cleaned);
    let magnitude = if value == ".inf" {
        f64::INFINITY
    } else if value == ".nan" {
        return Ok(Value::Float(f64::NAN));
    } else if value.contains(':') {
        // Least significant part first, as PyYAML adds them.
        let mut total = 0.0;
        let mut base = 1.0;
        for part in value.split(':').rev() {
            total += py_float(part, text)? * base;
            base *= 60.0;
        }
        total
    } else {
        py_float(value, text)?
    };
    Ok(Value::Float(if negative { -magnitude } else { magnitude }))
}

/// `SafeConstructor.construct_yaml_timestamp`; Python's range checks become
/// errors.
fn construct_timestamp(text: &str) -> Result<Value, String> {
    let bad = || format!("invalid timestamp '{text}'");
    let caps = TIMESTAMP_PARTS.captures(dollar(text)).ok_or_else(bad)?;
    let num = |name: &str| -> Option<u32> { caps.name(name)?.as_str().parse().ok() };
    let (year, month, day) = (
        num("year").ok_or_else(bad)?,
        num("month").ok_or_else(bad)?,
        num("day").ok_or_else(bad)?,
    );
    let date = Date::new(year, month, day).ok_or_else(|| format!("date out of range '{text}'"))?;
    let Some(hour) = num("hour") else {
        return Ok(Value::Date(date));
    };
    let (minute, second) = (
        num("minute").ok_or_else(bad)?,
        num("second").ok_or_else(bad)?,
    );
    let microsecond = match caps.name("fraction").map(|m| m.as_str()) {
        Some(fraction) if !fraction.is_empty() => {
            let mut digits: String = fraction.chars().take(6).collect();
            while digits.len() < 6 {
                digits.push('0');
            }
            digits.parse().map_err(|_| bad())?
        }
        _ => 0,
    };
    let utc_offset = if let Some(tz_sign) = caps.name("tz_sign") {
        let hours = i32::try_from(num("tz_hour").ok_or_else(bad)?).map_err(|_| bad())?;
        let minutes = i32::try_from(num("tz_minute").unwrap_or(0)).map_err(|_| bad())?;
        let seconds = hours * 3600 + minutes * 60;
        Some(if tz_sign.as_str() == "-" {
            -seconds
        } else {
            seconds
        })
    } else if caps.name("tz").is_some() {
        Some(0)
    } else {
        None
    };
    DateTime::new(date, hour, minute, second, microsecond, utc_offset)
        .map(Value::DateTime)
        .ok_or_else(|| format!("datetime out of range '{text}'"))
}
