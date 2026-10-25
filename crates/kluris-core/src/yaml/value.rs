//! A YAML value exactly as PyYAML's `SafeLoader` (YAML 1.1) builds it, limited
//! to the types Kluris reads and writes back.

use indexmap::IndexMap;
use num_bigint::BigInt;

/// A YAML mapping: string keys in insertion order (a repeated key keeps its
/// first position and takes the last value, like a Python dict).
pub type Map = IndexMap<String, Value>;

/// A calendar date (Python `datetime.date`): year 1-9999.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

impl Date {
    /// A valid Gregorian date, or `None` where Python's `date()` raises.
    pub fn new(year: u32, month: u32, day: u32) -> Option<Date> {
        if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
            return None;
        }
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date {
            year: u16::try_from(year).ok()?,
            month: u8::try_from(month).ok()?,
            day: u8::try_from(day).ok()?,
        })
    }

    pub fn year(&self) -> u32 {
        u32::from(self.year)
    }

    pub fn month(&self) -> u32 {
        u32::from(self.month)
    }

    pub fn day(&self) -> u32 {
        u32::from(self.day)
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A date and time (Python `datetime.datetime`), naive or with a fixed UTC
/// offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime {
    date: Date,
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
    utc_offset: Option<i32>,
}

impl DateTime {
    /// A valid datetime, or `None` where Python's `datetime()` or `timezone()`
    /// raises. `utc_offset` is in seconds east of UTC and must be strictly
    /// within one day; `None` is a naive datetime.
    pub fn new(
        date: Date,
        hour: u32,
        minute: u32,
        second: u32,
        microsecond: u32,
        utc_offset: Option<i32>,
    ) -> Option<DateTime> {
        if hour > 23 || minute > 59 || second > 59 || microsecond > 999_999 {
            return None;
        }
        if utc_offset.is_some_and(|o| o.unsigned_abs() >= 86_400) {
            return None;
        }
        Some(DateTime {
            date,
            hour: u8::try_from(hour).ok()?,
            minute: u8::try_from(minute).ok()?,
            second: u8::try_from(second).ok()?,
            microsecond,
            utc_offset,
        })
    }

    pub fn date(&self) -> Date {
        self.date
    }

    pub fn hour(&self) -> u32 {
        u32::from(self.hour)
    }

    pub fn minute(&self) -> u32 {
        u32::from(self.minute)
    }

    pub fn second(&self) -> u32 {
        u32::from(self.second)
    }

    pub fn microsecond(&self) -> u32 {
        self.microsecond
    }

    /// Seconds east of UTC; `None` for a naive datetime. `Z` is `Some(0)`.
    pub fn utc_offset(&self) -> Option<i32> {
        self.utc_offset
    }
}

/// One YAML value (YAML 1.1 types, as PyYAML resolves them).
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    /// Arbitrary precision, like a Python int.
    Int(BigInt),
    /// Includes `.inf`, `-.inf` and `.nan`.
    Float(f64),
    Str(String),
    Date(Date),
    DateTime(DateTime),
    Seq(Vec<Value>),
    Map(Map),
}

/// Structural equality. Floats compare by bit pattern, except that every NaN
/// equals every NaN: `0.0` and `-0.0` differ (Python writes them differently).
impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => {
                (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
            }
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (Value::Seq(a), Value::Seq(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
            }
            _ => false,
        }
    }
}

impl Value {
    /// The type name used in messages (`str`, `int`, `date`, ...).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Date(_) => "date",
            Value::DateTime(_) => "datetime",
            Value::Seq(_) => "list",
            Value::Map(_) => "mapping",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Value]> {
        match self {
            Value::Seq(items) => Some(items),
            _ => None,
        }
    }

    /// Python truthiness: `None`, `False`, `0`, `0.0`, `""`, `[]` and `{}`
    /// are false (what `yaml.safe_load(...) or {}` replaces).
    pub fn is_falsy(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Bool(b) => !b,
            Value::Int(i) => *i == BigInt::ZERO,
            Value::Float(f) => *f == 0.0,
            Value::Str(s) => s.is_empty(),
            Value::Seq(items) => items.is_empty(),
            Value::Map(map) => map.is_empty(),
            Value::Date(_) | Value::DateTime(_) => false,
        }
    }
}
