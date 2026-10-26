//! `yaml-emit` and `yaml-load`: the Rust side of the YAML differential test
//! (`parity/test_yaml_differential.py`), one JSON line in, one JSON line out.

use std::io::{self, BufRead, Write};

use kluris_core::yaml::{self, Date, DateTime, Map, Profile, Value};
use num_bigint::BigInt;
use serde_json::{Value as Json, json};

pub fn emit(args: &[String]) -> Result<(), String> {
    let profile = match args {
        [flag, profile] if flag == "--profile" => match profile.as_str() {
            "md" => Profile::Md,
            "yml" => Profile::Yml,
            "config" => Profile::Config,
            other => return Err(format!("unknown profile '{other}'")),
        },
        _ => return Err("yaml-emit needs --profile md|yml|config".into()),
    };
    each_line(|request| {
        let Ok(map) = decode(&request)?.into_map() else {
            return Err("yaml-emit writes mappings only".into());
        };
        Ok(Json::String(yaml::emit(&map, profile)))
    })
}

pub fn load(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("yaml-load takes no arguments".into());
    }
    each_line(|request| {
        let text = request.as_str().ok_or("yaml-load reads JSON strings")?;
        Ok(match yaml::load(text) {
            Ok(value) => json!({ "value": encode(&value) }),
            Err(error) => json!({ "error": error.to_string() }),
        })
    })
}

/// Answer each stdin line, flushing every answer: the caller waits for it.
fn each_line(answer: impl Fn(Json) -> Result<Json, String>) -> Result<(), String> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("cannot read stdin: {e}"))?;
        let request: Json =
            serde_json::from_str(&line).map_err(|e| format!("bad JSON line: {e}"))?;
        let reply = answer(request)?;
        writeln!(out, "{reply}")
            .and_then(|()| out.flush())
            .map_err(|e| format!("cannot write stdout: {e}"))?;
    }
    Ok(())
}

fn decode(json: &Json) -> Result<Value, String> {
    Ok(match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Int(parse_int(&n.to_string())?),
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::Seq(items.iter().map(decode).collect::<Result<_, _>>()?),
        Json::Object(object) => {
            if let [(tag, inner)] = object.iter().collect::<Vec<_>>()[..]
                && tag.starts_with('$')
            {
                return decode_tagged(tag, inner);
            }
            let mut map = Map::new();
            for (key, value) in object {
                map.insert(key.clone(), decode(value)?);
            }
            Value::Map(map)
        }
    })
}

fn decode_tagged(tag: &str, inner: &Json) -> Result<Value, String> {
    let text = || {
        inner
            .as_str()
            .ok_or_else(|| format!("{tag} needs a string"))
    };
    Ok(match tag {
        "$int" => Value::Int(parse_int(text()?)?),
        "$float" => Value::Float(
            text()?
                .parse::<f64>()
                .map_err(|e| format!("bad $float: {e}"))?,
        ),
        "$date" => Value::Date(parse_date(text()?)?),
        "$datetime" => Value::DateTime(parse_datetime(text()?)?),
        "$map" => {
            let pairs = inner.as_array().ok_or("$map needs [[key, value], ...]")?;
            let mut map = Map::new();
            for pair in pairs {
                match pair.as_array().map(Vec::as_slice) {
                    Some([Json::String(key), value]) => {
                        map.insert(key.clone(), decode(value)?);
                    }
                    _ => return Err("$map needs [[key, value], ...]".into()),
                }
            }
            Value::Map(map)
        }
        other => return Err(format!("unknown tag {other}")),
    })
}

fn parse_int(text: &str) -> Result<BigInt, String> {
    text.parse().map_err(|_| format!("bad integer '{text}'"))
}

fn number(text: &str, what: &str) -> Result<u32, String> {
    text.parse().map_err(|_| format!("bad {what} '{text}'"))
}

/// `YYYY-MM-DD`.
fn parse_date(text: &str) -> Result<Date, String> {
    let bad = || format!("bad date '{text}'");
    let mut parts = text.splitn(3, '-');
    let (Some(y), Some(m), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad());
    };
    Date::new(number(y, "year")?, number(m, "month")?, number(d, "day")?).ok_or_else(bad)
}

/// Python's `datetime.isoformat()`: `YYYY-MM-DDTHH:MM:SS[.ffffff]` and an
/// optional `+HH:MM[:SS]` offset.
fn parse_datetime(text: &str) -> Result<DateTime, String> {
    let bad = || format!("bad datetime '{text}'");
    let (date, rest) = text.split_once('T').ok_or_else(bad)?;
    let date = parse_date(date)?;
    let (time, offset) = match rest.find(['+', '-']) {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    let (hms, fraction) = time.split_once('.').unwrap_or((time, ""));
    if fraction.len() > 6 {
        return Err(bad());
    }
    let micro = format!("{fraction:0<6}");
    let hms: Vec<&str> = hms.split(':').collect();
    let [h, m, s] = hms[..] else {
        return Err(bad());
    };
    let utc_offset = match offset {
        None => None,
        Some(offset) => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let mut seconds = 0;
            for (part, unit) in offset[1..].split(':').zip([3600, 60, 1]) {
                seconds += i32::try_from(number(part, "offset")?).map_err(|_| bad())? * unit;
            }
            Some(sign * seconds)
        }
    };
    DateTime::new(
        date,
        number(h, "hour")?,
        number(m, "minute")?,
        number(s, "second")?,
        number(&micro, "microsecond")?,
        utc_offset,
    )
    .ok_or_else(bad)
}

fn encode(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => json!({ "$int": i.to_string() }),
        // Rust's `{:?}` (`1e16`, `NaN`, `inf`, `-0.0`) is what Python's
        // `float()` reads back.
        Value::Float(f) => json!({ "$float": format!("{f:?}") }),
        Value::Str(s) => Json::String(s.clone()),
        Value::Date(d) => {
            json!({ "$date": format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()) })
        }
        Value::DateTime(dt) => json!({ "$datetime": datetime_iso(dt) }),
        Value::Seq(items) => Json::Array(items.iter().map(encode).collect()),
        Value::Map(map) => {
            let pairs: Vec<Json> = map.iter().map(|(k, v)| json!([k, encode(v)])).collect();
            if map.len() == 1 && map.keys().all(|k| k.starts_with('$')) {
                json!({ "$map": pairs })
            } else {
                Json::Object(map.iter().map(|(k, v)| (k.clone(), encode(v))).collect())
            }
        }
    }
}

fn datetime_iso(dt: &DateTime) -> String {
    let d = dt.date();
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        d.year(),
        d.month(),
        d.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    );
    if dt.microsecond() != 0 {
        out.push_str(&format!(".{:06}", dt.microsecond()));
    }
    if let Some(offset) = dt.utc_offset() {
        let abs = offset.unsigned_abs();
        let sign = if offset < 0 { '-' } else { '+' };
        out.push_str(&format!(
            "{sign}{:02}:{:02}:{:02}",
            abs / 3600,
            abs % 3600 / 60,
            abs % 60
        ));
    }
    out
}
