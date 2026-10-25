//! PyYAML 6.0.3's implicit resolvers (`resolver.py`) and scalar constructors
//! (`constructor.py`, `SafeConstructor`) for plain scalars.
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

/// Resolve and construct a plain (unquoted, untagged) scalar the way
/// `SafeLoader` does. `Err` carries the reason for a value PyYAML rejects or
/// that is outside the value model.
pub fn plain_scalar(text: &str) -> Result<Value, String> {
    // PyYAML tries the resolvers registered for the first character in
    // registration order; each pattern only matches its own first characters,
    // so trying them all in that order is the same.
    if BOOL.is_match(text) {
        Ok(Value::Bool(matches!(
            text.to_ascii_lowercase().as_str(),
            "yes" | "true" | "on"
        )))
    } else if FLOAT.is_match(text) {
        construct_float(text)
    } else if INT.is_match(text) {
        construct_int(text)
    } else if MERGE.is_match(text) {
        Err("merge keys (<<) are not supported".into())
    } else if NULL.is_match(text) {
        Ok(Value::Null)
    } else if TIMESTAMP.is_match(text) {
        construct_timestamp(text)
    } else if VALUE.is_match(text) {
        Err("could not determine a constructor for the tag 'tag:yaml.org,2002:value'".into())
    } else {
        Ok(Value::Str(text.to_string()))
    }
}

/// Split a leading sign off, like the constructors do.
fn sign(value: &str) -> (bool, &str) {
    match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    }
}

fn parse_int(digits: &str, radix: u32, original: &str) -> Result<BigInt, String> {
    BigInt::parse_bytes(digits.as_bytes(), radix)
        .filter(|_| !digits.is_empty())
        .ok_or_else(|| format!("invalid integer '{original}'"))
}

/// `SafeConstructor.construct_yaml_int`.
fn construct_int(text: &str) -> Result<Value, String> {
    let cleaned = text.replace('_', "");
    let (negative, value) = sign(&cleaned);
    let magnitude = if value == "0" {
        BigInt::ZERO
    } else if let Some(bin) = value.strip_prefix("0b") {
        parse_int(bin, 2, text)?
    } else if let Some(hex) = value.strip_prefix("0x") {
        parse_int(hex, 16, text)?
    } else if value.starts_with('0') {
        parse_int(value, 8, text)?
    } else if value.contains(':') {
        let mut total = BigInt::ZERO;
        for part in value.split(':') {
            total = total * 60 + parse_int(part, 10, text)?;
        }
        total
    } else {
        parse_int(value, 10, text)?
    };
    Ok(Value::Int(if negative { -magnitude } else { magnitude }))
}

fn parse_float(part: &str, original: &str) -> Result<f64, String> {
    part.parse::<f64>()
        .map_err(|_| format!("invalid float '{original}'"))
}

/// `SafeConstructor.construct_yaml_float`.
fn construct_float(text: &str) -> Result<Value, String> {
    let cleaned = text.replace('_', "").to_ascii_lowercase();
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
            total += parse_float(part, text)? * base;
            base *= 60.0;
        }
        total
    } else {
        parse_float(value, text)?
    };
    Ok(Value::Float(if negative { -magnitude } else { magnitude }))
}

/// `SafeConstructor.construct_yaml_timestamp`; Python's range checks become
/// errors.
fn construct_timestamp(text: &str) -> Result<Value, String> {
    let bad = || format!("invalid timestamp '{text}'");
    let caps = TIMESTAMP_PARTS.captures(text).ok_or_else(bad)?;
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
