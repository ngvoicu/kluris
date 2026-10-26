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
///
/// `Clone`, `PartialEq` and `Drop` walk the value with an explicit stack, so a
/// value nested as deep as the reader accepts needs no deep call stack. (With
/// `Drop`, a payload is taken out with [`Value::into_map`] or
/// `std::mem::take`, not moved out of a pattern.)
#[derive(Debug)]
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
/// Maps compare in order (unlike Python dicts): key order is part of what the
/// writer reproduces.
impl Drop for Value {
    fn drop(&mut self) {
        let nested = |value: &Value| matches!(value, Value::Seq(_) | Value::Map(_));
        let mut stack: Vec<Value> = match self {
            Value::Seq(items) if items.iter().any(nested) => std::mem::take(items),
            Value::Map(map) if map.values().any(nested) => {
                std::mem::take(map).into_values().collect()
            }
            _ => return,
        };
        while let Some(mut value) = stack.pop() {
            match &mut value {
                Value::Seq(items) => stack.append(items),
                Value::Map(map) => stack.extend(std::mem::take(map).into_values()),
                _ => {}
            }
            // `value` drops here with no children left.
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        let mut pending = vec![(self, other)];
        while let Some((a, b)) = pending.pop() {
            let same = match (a, b) {
                (Value::Null, Value::Null) => true,
                (Value::Bool(a), Value::Bool(b)) => a == b,
                (Value::Int(a), Value::Int(b)) => a == b,
                (Value::Float(a), Value::Float(b)) => {
                    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
                }
                (Value::Str(a), Value::Str(b)) => a == b,
                (Value::Date(a), Value::Date(b)) => a == b,
                (Value::DateTime(a), Value::DateTime(b)) => a == b,
                (Value::Seq(a), Value::Seq(b)) => {
                    pending.extend(a.iter().zip(b));
                    a.len() == b.len()
                }
                (Value::Map(a), Value::Map(b)) => {
                    pending.extend(a.values().zip(b.values()));
                    a.len() == b.len() && a.keys().eq(b.keys())
                }
                _ => false,
            };
            if !same {
                return false;
            }
        }
        true
    }
}

impl Clone for Value {
    fn clone(&self) -> Value {
        /// A collection being copied: what is left of the source, the copy
        /// so far, and (in a mapping) the key of the item being copied.
        enum Frame<'a> {
            Seq(std::slice::Iter<'a, Value>, Vec<Value>),
            Map(indexmap::map::Iter<'a, String, Value>, Map, Option<String>),
        }
        fn open(value: &Value) -> Result<Value, Frame<'_>> {
            match value {
                Value::Seq(items) => Err(Frame::Seq(items.iter(), Vec::with_capacity(items.len()))),
                Value::Map(map) => Err(Frame::Map(map.iter(), Map::with_capacity(map.len()), None)),
                Value::Null => Ok(Value::Null),
                Value::Bool(b) => Ok(Value::Bool(*b)),
                Value::Int(i) => Ok(Value::Int(i.clone())),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Str(s) => Ok(Value::Str(s.clone())),
                Value::Date(d) => Ok(Value::Date(*d)),
                Value::DateTime(t) => Ok(Value::DateTime(*t)),
            }
        }
        let mut stack = match open(self) {
            Ok(leaf) => return leaf,
            Err(frame) => vec![frame],
        };
        loop {
            let next = match stack.last_mut() {
                Some(Frame::Seq(items, _)) => items.next(),
                Some(Frame::Map(entries, _, key)) => entries.next().map(|(k, v)| {
                    *key = Some(k.clone());
                    v
                }),
                None => unreachable!("the loop returns when the stack empties"),
            };
            let copy = match next {
                Some(value) => match open(value) {
                    Ok(leaf) => leaf,
                    Err(frame) => {
                        stack.push(frame);
                        continue;
                    }
                },
                None => {
                    let done = match stack.pop() {
                        Some(Frame::Seq(_, items)) => Value::Seq(items),
                        Some(Frame::Map(_, map, _)) => Value::Map(map),
                        None => unreachable!("checked above"),
                    };
                    if stack.is_empty() {
                        return done;
                    }
                    done
                }
            };
            match stack.last_mut() {
                Some(Frame::Seq(_, items)) => items.push(copy),
                Some(Frame::Map(_, map, key)) => {
                    if let Some(key) = key.take() {
                        map.insert(key, copy);
                    }
                }
                None => unreachable!("checked above"),
            }
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

    /// The mapping, taken out of the value; any other value back as it is.
    pub fn into_map(mut self) -> Result<Map, Value> {
        match &mut self {
            Value::Map(map) => Ok(std::mem::take(map)),
            _ => Err(self),
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
