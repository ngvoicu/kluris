//! Properties of the YAML writer and reader and of Python float formatting
//! that hold for any value, with nothing to compare against: what the writer
//! writes reads back to the value and writes again to the same bytes, the
//! reader never panics, and `float_repr` gives the shortest digits that read
//! back, positional between 1e-4 and 1e16.

use kluris_core::pyrepr::float_repr;
use kluris_core::yaml::{Date, DateTime, Map, Profile, Value, emit, load};
use num_bigint::BigInt;
use proptest::prelude::*;
use proptest::sample::select;

const PROFILES: [Profile; 3] = [Profile::Md, Profile::Yml, Profile::Config];

// --- the values ------------------------------------------------------------

/// YAML 1.1 keywords and look-alikes a string must be quoted to stay a string.
const KEYWORDS: &[&str] = &[
    "yes",
    "No",
    "on",
    "OFF",
    "true",
    "False",
    "y",
    "n",
    "null",
    "~",
    "",
    "<<",
    "=",
    "NaN",
    ".inf",
    "-.inf",
    "1e3",
    "0o17",
    "0x1F",
    "0b101",
    "1_000",
    "1:30",
    "012",
    "+1",
    "-0",
    "1.0",
    ".5",
    "2026-01-02",
    "2026-01-02 10:00:00 +05:30",
    "2026-02-30",
    "---",
    "...",
    "- ",
    "? ",
    ": ",
    " #",
    "# ",
    "./map.md",
    "a b",
];
/// YAML indicators, whitespace, line breaks, controls and non-ASCII text.
const SPECIAL: &[char] = &[
    '-',
    '?',
    ':',
    ',',
    '[',
    ']',
    '{',
    '}',
    '#',
    '&',
    '*',
    '!',
    '|',
    '>',
    '\'',
    '"',
    '%',
    '@',
    '`',
    '.',
    '~',
    '\\',
    ' ',
    '\t',
    '\n',
    '\r',
    '\0',
    '\u{7}',
    '\u{1b}',
    '\u{7f}',
    '\u{85}',
    '\u{9f}',
    '\u{2028}',
    '\u{2029}',
    '\u{feff}',
    '\u{fffe}',
    'é',
    '日',
    '\u{a0}',
    '\u{200b}',
    '🚀',
    '\u{10ffff}',
    '\u{e000}',
];
/// Repeated into lines past 80 columns.
const CHUNKS: &[&str] = &[
    "word ",
    "word",
    " ",
    "  ",
    "x",
    "é",
    "日本 ",
    "🚀 ",
    "\t",
    "\n",
    "- ",
    "# ",
    ": ",
    "'",
    "\"",
    "\\",
    "\u{85}",
    "https://example.com/a/very/long/path/segment",
];

fn text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        select(KEYWORDS).prop_map(String::from),
        prop::collection::vec(select(SPECIAL), 0..4).prop_map(|cs| cs.into_iter().collect()),
        prop::collection::vec(any::<char>(), 0..4).prop_map(|cs| cs.into_iter().collect()),
        (select(CHUNKS), 1..12usize).prop_map(|(chunk, n)| chunk.repeat(n)),
    ];
    prop::collection::vec(piece, 0..6).prop_map(|pieces| pieces.concat())
}

fn date() -> impl Strategy<Value = Date> {
    (1..=9999u32, 1..=12u32, 1..=31u32)
        .prop_filter_map("a real date", |(y, m, d)| Date::new(y, m, d))
}

/// Offsets in whole minutes: a datetime whose offset has seconds is written
/// with an explicit `!!timestamp` tag that PyYAML's timestamp pattern does
/// not read back (the writer corpus holds such rows as reread errors).
fn datetime() -> impl Strategy<Value = DateTime> {
    let offset = prop_oneof![
        Just(None),
        Just(Some(0)),
        (-(24 * 60 - 1)..24 * 60).prop_map(|minutes: i32| Some(minutes * 60)),
    ];
    (
        date(),
        0..24u32,
        0..60u32,
        0..60u32,
        prop_oneof![Just(0), 0..1_000_000u32],
        offset,
    )
        .prop_filter_map("a real datetime", |(d, h, m, s, us, off)| {
            DateTime::new(d, h, m, s, us, off)
        })
}

/// Zeros, the positional/exponent boundaries, the extremes and the specials.
const BOUNDARY_FLOATS: &[f64] = &[
    0.0,
    -0.0,
    1e15,
    1e16,
    1e17,
    1e-4,
    1e-5,
    5e-324,
    f64::MAX,
    f64::INFINITY,
    f64::NAN,
];

fn float() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<u64>().prop_map(f64::from_bits),
        select(BOUNDARY_FLOATS),
    ]
}

/// Ints of any size up to 400 digits, far inside the reader's 4,300-digit limit.
fn int() -> impl Strategy<Value = BigInt> {
    prop_oneof![
        any::<i64>().prop_map(BigInt::from),
        "-?[1-9][0-9]{18,400}".prop_map(|digits| digits.parse().unwrap_or_default()),
    ]
}

fn scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        1 => Just(Value::Null),
        1 => any::<bool>().prop_map(Value::Bool),
        1 => int().prop_map(Value::Int),
        1 => float().prop_map(Value::Float),
        1 => date().prop_map(Value::Date),
        1 => datetime().prop_map(Value::DateTime),
        3 => text().prop_map(Value::Str),
    ]
}

fn map_of(value: impl Strategy<Value = Value>) -> impl Strategy<Value = Map> {
    prop::collection::vec((text(), value), 0..5).prop_map(|entries| entries.into_iter().collect())
}

fn value() -> impl Strategy<Value = Value> {
    scalar().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Seq),
            map_of(inner).prop_map(Value::Map),
        ]
    })
}

fn document() -> impl Strategy<Value = Map> {
    map_of(value())
}

/// The value with every mapping's keys in code-point order, as the md and
/// yml profiles write them.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Map(map) => Value::Map(sorted_map(map)),
        Value::Seq(items) => Value::Seq(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn sorted_map(map: &Map) -> Map {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
        .into_iter()
        .map(|(k, v)| (k.clone(), sorted(v)))
        .collect()
}

/// What reading a profile's text back gives: the mapping, sorted where the
/// profile sorts.
fn read_back(map: &Map, profile: Profile) -> Value {
    if profile == Profile::Config {
        Value::Map(map.clone())
    } else {
        Value::Map(sorted_map(map))
    }
}

// --- the texts -------------------------------------------------------------

/// Fragments of YAML syntax, glued in any order.
const FRAGMENTS: &[&str] = &[
    "a",
    "key",
    "é",
    "日本",
    "1",
    "-1",
    "1.5",
    "~",
    "yes",
    "2026-01-02",
    ": ",
    ":",
    "- ",
    "-",
    "? ",
    "?",
    ", ",
    ",",
    "[",
    "]",
    "{",
    "}",
    "'",
    "''",
    "\"",
    "\\",
    "\\x",
    "\\u00e9",
    "\\U0001F680",
    "#",
    " #c",
    "&a ",
    "&",
    "*a",
    "*",
    "!!str ",
    "!!int ",
    "!!map ",
    "!",
    "!<tag:x>",
    "|",
    "|-",
    ">+",
    "|2",
    "---",
    "...",
    "--- ",
    "%YAML 1.1\n",
    "%TAG ! !x\n",
    "<<",
    "=",
    "\n",
    "\r\n",
    "\r",
    "  ",
    " ",
    "\t",
    "\u{85}",
    "\u{2028}",
    "\u{2029}",
    "\u{feff}",
    "\0",
    "\u{7}",
    "🚀",
];

fn yaml_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::collection::vec(select(FRAGMENTS), 0..40).prop_map(|parts| parts.concat()),
        1 => any::<String>(),
    ]
}

// --- floats ----------------------------------------------------------------

/// How many significant digits a number's text has: no sign, point or
/// exponent, zeros stripped at both ends. At an exact tie `float_repr` takes
/// the even neighbour where Rust's `{:e}` rounds up, so the digits may differ
/// in the last place but never in number.
fn significant_digits(text: &str) -> usize {
    let mantissa = text.trim_start_matches('-').split('e').next().unwrap_or("");
    mantissa.replace('.', "").trim_matches('0').len()
}

fn finite() -> impl Strategy<Value = f64> {
    float().prop_filter("finite", |x| x.is_finite())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn what_each_profile_writes_reads_back_to_the_value(map in document()) {
        for profile in PROFILES {
            let text = emit(&map, profile);
            match load(&text) {
                Ok(back) => prop_assert_eq!(back, read_back(&map, profile), "[{:?}] {:?}", profile, text),
                Err(e) => prop_assert!(false, "[{profile:?}] {text:?} does not read: {e}"),
            }
        }
    }

    #[test]
    fn writing_what_was_read_gives_the_same_bytes(map in document()) {
        for profile in PROFILES {
            let text = emit(&map, profile);
            let back = load(&text).ok().and_then(|value| value.into_map().ok());
            prop_assert!(back.is_some(), "[{profile:?}] {text:?} does not read to a mapping");
            let again = back.map(|back| emit(&back, profile));
            prop_assert_eq!(again.as_deref(), Some(text.as_str()), "[{:?}]", profile);
        }
    }

    #[test]
    fn the_reader_never_panics(text in yaml_text()) {
        let _ = load(&text);
    }

    #[test]
    fn float_repr_reads_back_with_the_shortest_digits(x in finite()) {
        let repr = float_repr(x);
        let back: Result<f64, _> = repr.parse();
        prop_assert_eq!(back.map(f64::to_bits), Ok(x.to_bits()), "{}", repr);
        prop_assert_eq!(significant_digits(&repr), significant_digits(&format!("{:e}", x)), "{}", repr);
    }

    #[test]
    fn float_repr_uses_the_exponent_form_outside_1e_minus_4_to_1e16(x in finite()) {
        let repr = float_repr(x);
        let exponent = x != 0.0 && (x.abs() < 1e-4 || x.abs() >= 1e16);
        prop_assert_eq!(repr.contains('e'), exponent, "{}", repr);
    }
}
