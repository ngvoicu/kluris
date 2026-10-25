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

impl ScalarTag {
    /// The tag as written in YAML (`!!int`).
    pub fn name(self) -> &'static str {
        match self {
            ScalarTag::Str => "!!str",
            ScalarTag::Int => "!!int",
            ScalarTag::Float => "!!float",
            ScalarTag::Bool => "!!bool",
            ScalarTag::Null => "!!null",
            ScalarTag::Timestamp => "!!timestamp",
        }
    }
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

/// Resolve and construct a scalar with PyYAML's implicit resolvers: a plain
/// scalar, or any scalar tagged with the non-specific `!`. `Err` carries the
/// reason for a value PyYAML rejects or that is outside the value model.
pub fn implicit_scalar(text: &str) -> Result<Value, String> {
    // PyYAML tries the resolvers registered for the first character in
    // registration order; each pattern only matches its own first characters,
    // so trying them all in that order is the same.
    let probe = dollar(text);
    if BOOL.is_match(probe) {
        construct_bool(text)
    } else if FLOAT.is_match(probe) {
        construct_float(text)
    } else if INT.is_match(probe) {
        construct_int(text)
    } else if MERGE.is_match(probe) {
        Err("merge keys (<<) are not supported".into())
    } else if NULL.is_match(probe) {
        Ok(Value::Null)
    } else if TIMESTAMP.is_match(probe) {
        construct_timestamp(text)
    } else if VALUE.is_match(probe) {
        Err("could not determine a constructor for the tag 'tag:yaml.org,2002:value'".into())
    } else {
        Ok(Value::Str(text.to_string()))
    }
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

/// Python's `int(text, radix)` for the radixes PyYAML uses: surrounding
/// whitespace, one sign and (radix 2, 8, 16) the matching `0b`/`0o`/`0x`
/// prefix are accepted. Only ASCII digits (Python also takes other Unicode
/// decimal digits).
fn py_int(text: &str, radix: u32, original: &str) -> Result<BigInt, String> {
    let invalid = || format!("invalid integer '{original}'");
    let (negative, digits) = sign(text.trim_matches(is_py_space));
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
/// and `nan` are accepted.
fn py_float(text: &str, original: &str) -> Result<f64, String> {
    // For ASCII text Rust's `f64` grammar is Python's `float()` grammar.
    text.trim_matches(is_py_space)
        .parse::<f64>()
        .map_err(|_| format!("invalid float '{original}'"))
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
