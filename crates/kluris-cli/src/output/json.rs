//! JSON output byte-identical to Python's `json.dumps(obj)` with its defaults,
//! which is what Kluris v2 prints for every `--json` command.

/// A JSON value. Objects keep their keys in insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    #[allow(dead_code, reason = "no command emits numbers yet")]
    Int(i64),
    #[allow(dead_code, reason = "no command emits numbers yet")]
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Serialize like `json.dumps(value)`, without a trailing newline.
pub fn to_string(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

/// Serialize like `click.echo(json.dumps(value))`: one line ending in `\n`.
pub fn to_line(value: &Json) -> String {
    let mut out = to_string(value);
    out.push('\n');
    out
}

fn write_value(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(n) => out.push_str(&n.to_string()),
        Json::Float(x) => write_float(out, *x),
        Json::Str(s) => write_str(out, s),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Json::Object(pairs) => {
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_str(out, key);
                out.push_str(": ");
                write_value(out, item);
            }
            out.push('}');
        }
    }
}

/// `ensure_ascii=True`: printable ASCII as is, everything else escaped.
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// Python `float.__repr__`: the shortest round-trip digits, positional when
/// the decimal exponent is in -4..16, else `d.ddde+XX`.
fn write_float(out: &mut String, x: f64) {
    if x.is_nan() {
        out.push_str("NaN");
    } else if x.is_infinite() {
        out.push_str(if x > 0.0 { "Infinity" } else { "-Infinity" });
    } else {
        // `{:e}` gives the shortest round-trip digits as `<mantissa>e<exp>`.
        let sci = format!("{x:e}");
        let (mantissa, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        if (-4..16).contains(&exp) {
            let fixed = x.to_string();
            out.push_str(&fixed);
            if !fixed.contains('.') {
                out.push_str(".0");
            }
        } else {
            let sign = if exp < 0 { '-' } else { '+' };
            out.push_str(&format!("{mantissa}e{sign}{:02}", exp.abs()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Json {
        Json::Str(text.to_string())
    }

    fn obj(pairs: &[(&str, Json)]) -> Json {
        Json::Object(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    // Every expected string below is the output of
    // `python3 -c 'import json; print(json.dumps(<value>))'`.

    #[test]
    fn separators_and_scalars() {
        let v = obj(&[
            ("a", Json::Int(1)),
            ("b", Json::Array(vec![Json::Int(1), Json::Int(2)])),
            ("c", Json::Null),
            ("d", Json::Bool(true)),
            ("e", Json::Bool(false)),
        ]);
        assert_eq!(
            to_string(&v),
            r#"{"a": 1, "b": [1, 2], "c": null, "d": true, "e": false}"#
        );
    }

    #[test]
    fn keys_keep_insertion_order() {
        let v = obj(&[("z", Json::Int(1)), ("a", Json::Int(2))]);
        assert_eq!(to_string(&v), r#"{"z": 1, "a": 2}"#);
    }

    #[test]
    fn empty_and_nested_containers() {
        assert_eq!(to_string(&Json::Object(vec![])), "{}");
        assert_eq!(to_string(&Json::Array(vec![])), "[]");
        let v = obj(&[
            ("x", obj(&[("y", Json::Array(vec![]))])),
            ("k", Json::Array(vec![Json::Object(vec![])])),
        ]);
        assert_eq!(to_string(&v), r#"{"x": {"y": []}, "k": [{}]}"#);
    }

    #[test]
    fn non_ascii_is_escaped_lowercase() {
        assert_eq!(to_string(&s("\u{2014}")), r#""\u2014""#);
        assert_eq!(to_string(&s("\u{e9}")), r#""\u00e9""#);
        assert_eq!(to_string(&s("\u{ffff}")), r#""\uffff""#);
    }

    #[test]
    fn astral_chars_are_surrogate_pairs() {
        assert_eq!(to_string(&s("\u{1f680}")), r#""\ud83d\ude80""#);
        assert_eq!(to_string(&s("\u{10ffff}")), r#""\udbff\udfff""#);
    }

    #[test]
    fn slash_is_not_escaped() {
        assert_eq!(to_string(&s("a/b")), r#""a/b""#);
    }

    #[test]
    fn short_escapes() {
        assert_eq!(to_string(&s("\n\t\r\u{8}\u{c}\"\\")), r#""\n\t\r\b\f\"\\""#);
    }

    #[test]
    fn other_controls_and_del_are_u00xx() {
        assert_eq!(
            to_string(&s("\u{0}\u{1f}\u{7f}")),
            r#""\u0000\u001f\u007f""#
        );
        assert_eq!(to_string(&s("\u{b}\u{1b}")), r#""\u000b\u001b""#);
    }

    #[test]
    fn keys_are_escaped_too() {
        let v = obj(&[("\u{e9}", s("\u{fc}"))]);
        assert_eq!(to_string(&v), r#"{"\u00e9": "\u00fc"}"#);
    }

    #[test]
    fn integers() {
        assert_eq!(to_string(&Json::Int(0)), "0");
        assert_eq!(to_string(&Json::Int(i64::MIN)), "-9223372036854775808");
        assert_eq!(to_string(&Json::Int(i64::MAX)), "9223372036854775807");
    }

    #[test]
    fn floats_use_python_repr() {
        let cases: &[(f64, &str)] = &[
            (1.0, "1.0"),
            (0.1, "0.1"),
            (100.0, "100.0"),
            (1234.5, "1234.5"),
            (123456789.123, "123456789.123"),
            (0.30000000000000004, "0.30000000000000004"),
            (1e15, "1000000000000000.0"),
            (9999999999999998.0, "9999999999999998.0"),
            (1e16, "1e+16"),
            (12345678901234567.0, "1.2345678901234568e+16"),
            (1e20, "1e+20"),
            (1e22, "1e+22"),
            (1.5e300, "1.5e+300"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (0.0001, "0.0001"),
            (1e-5, "1e-05"),
            (2.5e-10, "2.5e-10"),
            (5e-324, "5e-324"),
            (-0.0, "-0.0"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
        ];
        for (value, expected) in cases {
            assert_eq!(
                to_string(&Json::Float(*value)),
                *expected,
                "float {value:?}"
            );
        }
    }

    #[test]
    fn to_line_appends_one_newline() {
        let v = obj(&[("ok", Json::Bool(true)), ("brains", Json::Array(vec![]))]);
        assert_eq!(to_line(&v), "{\"ok\": true, \"brains\": []}\n");
    }
}
