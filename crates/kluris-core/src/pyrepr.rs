//! Python's text forms that v2's output carries: `repr(x)` of a float (PyYAML's
//! float representer builds on it) and `json.dumps(x)`, `str()` and `repr()`
//! of the values YAML gives, and `str(Path(text))`.

use std::sync::LazyLock;

use regex::Regex;

use crate::yaml::{Date, DateTime, Value, date_iso, datetime_iso};

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

/// The characters Python's `str.isprintable()` refuses: Unicode categories
/// Cc, Cf, Cs, Co, Cn, Zl, Zp and Zs (the space excepted, which is ASCII).
/// The table is the `regex` crate's Unicode version, not Python 3.11's 14.0:
/// a character assigned since then is printable here and escaped by v2.
static NON_PRINTABLE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[\p{Cc}\p{Cf}\p{Co}\p{Cn}\p{Zl}\p{Zp}\p{Zs}]$").ok());

fn is_printable(c: char) -> bool {
    if c.is_ascii() {
        return !c.is_ascii_control();
    }
    let mut buf = [0u8; 4];
    !NON_PRINTABLE
        .as_ref()
        .is_some_and(|re| re.is_match(c.encode_utf8(&mut buf)))
}

/// Python's `repr()` of a `str`: single quotes unless the text has a `'` and
/// no `"`; `\\`, the quote, `\t`, `\n`, `\r` escaped; other non-printable
/// characters as `\xhh`, `\uhhhh` or `\Uhhhhhhhh`.
pub fn py_repr_str(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_printable(c) => out.push(c),
            c => {
                let code = u32::from(c);
                if code <= 0xff {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// `repr(datetime.date(...))`.
fn date_repr(date: &Date) -> String {
    format!(
        "datetime.date({}, {}, {})",
        date.year(),
        date.month(),
        date.day()
    )
}

/// `repr(datetime.timezone(timedelta(seconds=offset)))`.
fn timezone_repr(offset: i32) -> String {
    if offset == 0 {
        return "datetime.timezone.utc".into();
    }
    let days = offset.div_euclid(86_400);
    let seconds = offset.rem_euclid(86_400);
    let mut parts = Vec::new();
    if days != 0 {
        parts.push(format!("days={days}"));
    }
    if seconds != 0 {
        parts.push(format!("seconds={seconds}"));
    }
    format!(
        "datetime.timezone(datetime.timedelta({}))",
        parts.join(", ")
    )
}

/// `repr(datetime.datetime(...))`: a zero microsecond is left out, and then a
/// zero second.
fn datetime_repr(dt: &DateTime) -> String {
    let date = dt.date();
    let mut fields = vec![
        date.year(),
        date.month(),
        date.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        dt.microsecond(),
    ];
    for _ in 0..2 {
        if fields.last() == Some(&0) {
            fields.pop();
        }
    }
    let fields: Vec<String> = fields.iter().map(u32::to_string).collect();
    match dt.utc_offset() {
        None => format!("datetime.datetime({})", fields.join(", ")),
        Some(offset) => format!(
            "datetime.datetime({}, tzinfo={})",
            fields.join(", "),
            timezone_repr(offset)
        ),
    }
}

/// Python's `repr()` of a value as PyYAML's `SafeLoader` gives it. Nested
/// values are written with an explicit stack.
pub fn py_repr(value: &Value) -> String {
    enum Task<'a> {
        Value(&'a Value),
        Text(&'static str),
        Key(&'a str),
    }
    let mut out = String::new();
    let mut tasks = vec![Task::Value(value)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Text(text) => out.push_str(text),
            Task::Key(key) => {
                out.push_str(&py_repr_str(key));
                out.push_str(": ");
            }
            Task::Value(value) => match value {
                Value::Null => out.push_str("None"),
                Value::Bool(true) => out.push_str("True"),
                Value::Bool(false) => out.push_str("False"),
                Value::Int(n) => out.push_str(&n.to_string()),
                Value::Float(x) => out.push_str(&float_repr(*x)),
                Value::Str(s) => out.push_str(&py_repr_str(s)),
                Value::Date(d) => out.push_str(&date_repr(d)),
                Value::DateTime(dt) => out.push_str(&datetime_repr(dt)),
                Value::Seq(items) => {
                    out.push('[');
                    tasks.push(Task::Text("]"));
                    for (i, item) in items.iter().enumerate().rev() {
                        tasks.push(Task::Value(item));
                        if i > 0 {
                            tasks.push(Task::Text(", "));
                        }
                    }
                }
                Value::Map(map) => {
                    out.push('{');
                    tasks.push(Task::Text("}"));
                    for (i, (key, item)) in map.iter().enumerate().rev() {
                        tasks.push(Task::Value(item));
                        tasks.push(Task::Key(key));
                        if i > 0 {
                            tasks.push(Task::Text(", "));
                        }
                    }
                }
            },
        }
    }
    out
}

/// Python's `str()` of a top-level frontmatter value after v2's
/// `_normalize_metadata`, which turns a top-level date or datetime into its
/// `isoformat()` text: a string is itself, everything else its `repr()`.
pub fn py_str(value: &Value) -> String {
    match value {
        Value::Str(s) => s.clone(),
        Value::Date(d) => date_iso(d),
        Value::DateTime(dt) => datetime_iso(dt, 'T'),
        other => py_repr(other),
    }
}

/// `str(PurePosixPath(raw))` in Python 3.11: empty and `.` parts and repeated
/// `/` dropped, a trailing `/` removed, exactly two leading `/` kept; `.` for
/// an empty path.
pub fn py_posix_path_str(raw: &str) -> String {
    let stripped = raw.trim_start_matches('/');
    let root = match raw.len() - stripped.len() {
        0 => "",
        2 => "//",
        _ => "/",
    };
    let parts: Vec<&str> = stripped
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let text = format!("{root}{}", parts.join("/"));
    if text.is_empty() { ".".into() } else { text }
}

/// `str(PureWindowsPath(raw))` in Python 3.11: `/` read as `\`, a drive
/// (`C:`) or UNC share (`\\server\share`) kept, empty and `.` parts dropped;
/// `.` for an empty path.
pub fn py_windows_path_str(raw: &str) -> String {
    let path = raw.replace('/', "\\");
    let (drive, rest, unc) = windows_drive(&path);
    let stripped = rest.trim_start_matches('\\');
    let root = if unc || stripped.len() < rest.len() {
        "\\"
    } else {
        ""
    };
    let parts: Vec<&str> = stripped
        .split('\\')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let text = format!("{drive}{root}{}", parts.join("\\"));
    if text.is_empty() { ".".into() } else { text }
}

/// pathlib's Windows `splitroot` drive: `\\server\share` (always rooted) or
/// `X:`, then the rest with its root separator, and whether it is UNC.
fn windows_drive(path: &str) -> (&str, &str, bool) {
    let bytes = path.as_bytes();
    if bytes.starts_with(b"\\\\")
        && bytes.get(2) != Some(&b'\\')
        && let Some(server_end) = path[2..].find('\\').map(|i| i + 2)
    {
        let share_end = path[server_end + 1..]
            .find('\\')
            .map_or(path.len(), |i| i + server_end + 1);
        if share_end != server_end + 1 {
            return (&path[..share_end], &path[share_end..], true);
        }
    }
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return (&path[..2], &path[2..], false);
    }
    ("", path, false)
}

/// `str(Path(raw))` on this OS.
pub fn py_path_str(raw: &str) -> String {
    if cfg!(windows) {
        py_windows_path_str(raw)
    } else {
        py_posix_path_str(raw)
    }
}
