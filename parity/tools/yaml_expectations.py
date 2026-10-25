"""Print the expected YAML 1.1 values for the Kluris 3 reader's scalar table.

Each case is loaded the way v2 reads frontmatter (PyYAML ``SafeLoader``) as the
value of ``v: <case>`` and printed as a Rust row for
``crates/kluris-core/tests/yaml_read.rs`` (``SCALARS``). A case PyYAML rejects,
or one outside the v3 value model, prints ``err()``.

Run it with the oracle venv (PyYAML 6.0.3):

    parity/.oracle/venv/bin/python -I parity/tools/yaml_expectations.py
"""

from __future__ import annotations

import datetime
import sys

import yaml

assert yaml.__version__ == "6.0.3", yaml.__version__

BIG = "1" + "0" * 30

CASES = [
    # bool: only the listed case variants of yes/no/true/false/on/off
    "yes", "Yes", "YES", "no", "No", "NO", "true", "True", "TRUE",
    "false", "False", "FALSE", "on", "On", "ON", "off", "Off", "OFF",
    "yEs", "tRUE", "y", "Y", "n", "N",
    # int
    "0", "-0", "+0", "1", "-17", "+17", "0755", "-0755", "0_", "00", "08",
    "0o17", "0b1010", "-0b11", "0b_", "0x1F", "0xff", "-0x_1", "0x_", "0X1F",
    "1_000", "1__0", "_1", "1:30", "-1:30", "190:20:30", "1:60", "1:5",
    "0:30", BIG, "-" + BIG, "0x" + "F" * 32, "1:" + "0" * 2,
    # float
    "1.0", "1.", "1.5", "-1.5", "+1.5", ".5", "-.5", "+.5", "1.0e+3",
    "1.0E-3", "1e3", "1.e5", "1.0e3", "2.10", "1_0.5", "1._", "1:30.5",
    "-1:30.5", ".inf", ".Inf", ".INF", "-.inf", "+.inf", ".iNf", ".nan",
    ".NaN", ".NAN", "-.nan", "NaN", "nan", "inf", "-0.0", "0.1", "1e+300",
    "1.0e+400", "12:30",
    # null
    "~", "null", "Null", "NULL", "nULL", "", "None",
    # timestamp
    "2026-01-02", "2026-1-2", "2026-01-02 10:00", "2026-01-02T10:00:00Z",
    "2026-01-02t10:00:00Z", "2026-01-02T10:00:00z", "2026-01-02 10:00:00",
    "2026-1-2 10:00:00", "2026-01-02T10:00:00.5", "2026-01-02T10:00:00.1234567",
    "2026-01-02T10:00:00.", "2026-01-02 10:00:00 +05:30",
    "2026-01-02T10:00:00-5", "2026-01-02T10:00:00+00:00",
    "2026-01-02T10:00:00-00:00", "2026-01-02    10:00:00", "2026-01-02T1:02:03",
    "2024-02-29", "2023-02-29", "2026-02-30", "0000-01-01", "2026-13-01",
    "2026-01-02T24:00:00", "2026-01-02T10:00:60", "2026-01-02T10:00:00+25:00",
    "2026-01-02T10:00:00+23:59", "2026-01-02T10:00:00+5:99",
    # strings
    "x:y", "a#b", "-a", "?a", ":a", "a]", "a,b", "a b", "é", "日本",
    "./map.md", "'yes'", '"0755"', "'1:30'", "'~'", "''", '""', '"2026-01-02"',
    '"a\\tb"', "'it''s'",
    # outside the value model
    "=", "<<",
]


def rust_str(text: str) -> str:
    if any(ord(ch) < 0x20 for ch in text):
        escaped = "".join(
            f"\\u{{{ord(ch):x}}}" if ord(ch) < 0x20 or ch in '"\\' else ch for ch in text
        )
        return f'"{escaped}"'
    if '"#' in text:
        raise SystemExit(f"cannot write {text!r} as a raw string")
    return f'r#"{text}"#'


def rust_value(value: object) -> str:
    if value is None:
        return "null()"
    if isinstance(value, bool):
        return f"b({str(value).lower()})"
    if isinstance(value, int):
        return f'int("{value}")'
    if isinstance(value, float):
        return f'flt("{value!r}")'
    if isinstance(value, str):
        return f"s({rust_str(value)})"
    if isinstance(value, datetime.datetime):
        offset = value.utcoffset()
        off = "None" if offset is None else f"Some({int(offset.total_seconds())})"
        return (
            f"dt({value.year}, {value.month}, {value.day}, {value.hour}, "
            f"{value.minute}, {value.second}, {value.microsecond}, {off})"
        )
    if isinstance(value, datetime.date):
        return f"date({value.year}, {value.month}, {value.day})"
    raise SystemExit(f"unexpected value {value!r}")


def main() -> int:
    for case in CASES:
        try:
            value = yaml.safe_load(f"v: {case}")["v"]
        except Exception:  # noqa: BLE001 -- any PyYAML failure is an error value
            expected = "err()"
        else:
            expected = rust_value(value)
        print(f"    ({rust_str(case)}, {expected}),")
    return 0


if __name__ == "__main__":
    sys.exit(main())
