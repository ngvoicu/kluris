"""Print the expected YAML 1.1 values for the Kluris 3 reader's scalar table.

Each case is loaded the way v2 reads frontmatter (PyYAML ``SafeLoader``) as the
value of ``v: <case>`` and printed as a Rust row for
``crates/kluris-core/tests/yaml_read.rs``: ``scalars()`` by default, ``tagged()``
(explicit tags) with ``--tagged``. With ``--documents`` each case is a whole
YAML text (``documents()``), loaded as it is. A case PyYAML rejects, or one
outside the v3 value model, prints ``err()``. The oracle is the pure-Python
``SafeLoader`` (v3 reads every YAML the way it does); a case where libyaml's
``CSafeLoader`` reads otherwise is still printed, with a ``// CSafeLoader:``
comment.

Run it with the oracle venv (PyYAML 6.0.3):

    parity/.oracle/venv/bin/python -I parity/tools/yaml_expectations.py [--tagged | --documents | --long]

With ``--long`` the texts repeat a digit thousands of times (``long_rows()``):
each row is ``((prefix, digit, count, suffix), expected)``.
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

# Explicit tags. The YAML core tags inside the value model construct like
# SafeLoader (content that does not fit the tag raises); the non-specific `!`
# resolves the content like a plain scalar, even when it is quoted; every other
# tag is outside the value model.
# Whitespace around numbers: Python's int() and float() skip ASCII \t\n\v\f\r
# and space and the non-ASCII Unicode spaces, but not U+001C..U+001F (which
# str.strip() does strip); in a sexagesimal number each part is converted.
NUMERIC_SPACES = ["\\x09", "\\x0b", "\\x0c", "\\x0d", "\\x1c", "\\x1d", "\\x1e", "\\x1f", "\\x20", "\\x85", "\\xa0", "\\u1680", "\\u180e", "\\u2000", "\\u200a", "\\u200b", "\\u2028", "\\u2029", "\\u202f", "\\u205f", "\\u3000", "\\ufeff"]
NUMERIC_SPACE_CASES = [
    case
    for space in NUMERIC_SPACES
    for case in (
        f'!!int "{space}1"', f'!!int "1{space}"', f'!!float "{space}1.5"', f'!!float "1.5{space}"',
        f'!!int "1{space}:30"', f'!!int "1:{space}30"', f'!!float "1:30.5{space}"', f'!!float "{space}1:30.5"',
        f'!!int "{space}0x1F"', f'!!int "0b1{space}"', f'!!int "0755{space}"',
    )
]

TAGGED = [
    # !!str
    "!!str 12", "!!str yes", "!!str ~", "!!str", "!!str 2026-01-02", '!!str "x"',
    "!!str 1.5", "!!str <<", "!!str [a]", "!!str {a: 1}",
    # !!int
    "!!int 12", '!!int "0x1F"', "!!int ' 12 '", "!!int 0o17", "!!int '0O17'",
    "!!int 0755", "!!int 1_000", "!!int 1:30", "!!int '1: 30'", "!!int 0b101",
    "!!int --5", "!!int -0x10", "!!int abc", "!!int 1e3", "!!int 1.5",
    "!!int ''", "!!int -", "!!int 0x_", "!!int 08", "!!int 0x 1F", "!!int [1]",
    "!!int {a: 1}", "!!int " + BIG,
    # !!float
    "!!float 1", "!!float 1e3", '!!float "1.5"', "!!float .inf", "!!float -.inf",
    "!!float -.nan", "!!float inf", "!!float nan", "!!float infinity",
    "!!float ' 2.5 '", "!!float ' -.5 '", "!!float 1:30", "!!float 1_0.5",
    "!!float 1.E5", "!!float abc", "!!float ''", "!!float 0x10", "!!float [1]",
    # !!bool
    "!!bool yes", "!!bool YES", "!!bool 'on'", "!!bool True", "!!bool oFf",
    "!!bool maybe", "!!bool y", "!!bool ''", "!!bool [yes]",
    # !!null
    "!!null foo", "!!null ''", "!!null ~", "!!null", "!!null [a]",
    # !!timestamp
    "!!timestamp 2026-01-02", "!!timestamp 2026-1-2", "!!timestamp '2026-01-02T10:00:00Z'",
    "!!timestamp 2026-1-2 3:04:05.5 +01", "!!timestamp 2026-01-02 10:00",
    "!!timestamp foo", "!!timestamp 2026-02-30", "!!timestamp [2026-01-02]",
    # !!seq and !!map
    "!!seq [a, 1]", "!!seq []", "!!seq {a: 1}", "!!seq a", "!!map {a: 1, b: [x]}",
    "!!map {}", "!!map [a]", "!!map a",
    # verbatim core tags
    "!<tag:yaml.org,2002:str> 5", "!<tag:yaml.org,2002:int> '7'",
    # non-specific `!`
    "! 123", '! "123"', "! '1:30'", "! yes", "! 'x'", "! ''", "! '2026-01-02'",
    "! [a, 1]", "! {a: '1'}", "!<!> 5", "! <<", "! =",
    # `$` also matches before a final newline; PyYAML adds none at end of input
    "! |\n  123", "! |\n  123\n", "! |\n  yes", "! |\n  yes\n",
    # outside the value model
    "!!binary aGk=", "!!set {x}", "!!omap [{x: 1}]", "!!pairs [{x: 1}]",
    "!!python/tuple [1]", "!!python/name:os.system x", "!!python/str x",
    "!foo x", "!foo [a]", "!foo {a: 1}", "!!merge x", "!!value =",
    "!<tag:example.com,2000:str> x",
    # Python's int()/float() read any Unicode decimal digit (category Nd) and
    # skip Unicode whitespace; other numeric characters are not digits
    '!!int "\u0663"', '!!float "\u0663.\u0661"', '!!int "\uff11\uff12"', '!!int "-\u0663"',
    '!!int "\u0663_\u0661"', '!!float "1e\u0663"', '!!int "0x\u0663"', '!!int "0\u0663"',
    '!!int "0b\u0661"', '!!int "\u0663:\u0663\u0660"', '!!float "\u0663:\u0663\u0660"',
    '!!int "\U0001D7D9"', '!!int "\u0660"', '!!float "\uff11.\uff15"', '!!float "\u0663e\u0661"',
    '!!int " \u0663 "', '!!int "\u2003\u0663\u3000"', '!!float "\u3000\u0663.5\u2003"',
    '!!float "\u00b2"', '!!int "\u00b2"', '!!int "\u216b"', '!!int "\uff41"', '!!int "0x\uff41"',
    '!!float "\u0663\u066b\u0665"', '!!int "\u0e53\u0e52"', '!!int "\u1040"', '! "\u0663"',
] + NUMERIC_SPACE_CASES

# Whole documents (``--documents``).
CHOMPING = ["|", "|-", "|+", ">", ">-", ">+"]
BLOCK_PREFIXES = ["", "x: \u00e9\n", "\u65e5\u672c: \u00e9\u00e9\n"]
BLOCK_CONTENT = ["abc", "\u00e9", "a\n  \u00e9 b\n\n  \U0001F680"]
BLOCK_ENDINGS = ["", "\n", "\n\n", "\n  ", "\n\n  \n  ", "\nw: 1", "\n# \u00e9"]
BLOCKS = [
    f"{prefix}v: {indicator}\n  {content}{ending}"
    for prefix in BLOCK_PREFIXES
    for indicator in CHOMPING
    for content in BLOCK_CONTENT
    for ending in BLOCK_ENDINGS
] + [
    "- |\n  \u00e9", "- >-\n  \u00e9\n  \u00e9", "- |+\n  \u00e9\n\n",
    "v: |\r\n  \u00e9", "v: |+\r\n  \u00e9\r\n\r\n", "v: >\r\n  \u00e9\r\n  b",
    "v: |2\n   \u00e9", "v: |-2\n   \u00e9\n", "v: \u00e9\nw: |\n  x",
    "v: '\u00e9'\nw: >\n  x", 'v: "\u00e9"\nw: |-\n  x\n', "v: |\n  \u00e9\n...",
    # empty block scalars, and a last line shorter or longer than the indentation
    "v: |\nw: 1", "v: |+\n\nw: 1", "- |\n- x", "v: |+\n  \n\nw: 1", "v: |\n", "v: |+\n",
    "v: |+\n\n", "v: |\n  \n", "v: |+\n  \n", "v: |-\n\n", "v: |\n\n  ", "v: |+", "v: >+\n  ",
    "v: |+\r\n  ", "v: |\n  abc\n ", "v: |\n  abc\n   ", "v: |+\n  abc\n\n ",
    "v: |2\n    abc\n   ", "v: |2\n    abc\n  ", "v: |2\n    abc\n ", "v: |1\n   ", "v: |1+\n   \n  ",
    "- |\n abc\n ", "k:\n  - |\n    abc\n    ", "k:\n  - |\n    abc\n   ", "k:\n- |\n x\n ",
    "x:\n  v: |\n    abc\n  ", "x:\n  v: |\n    abc\n    ", "v: |\n  abc\n\n\n  ",
    "v: >\n  a\n\n  b\n   c\n  d\n ", "v: >-\n   a\n  b", "v: | # c | >\n  abc",
    "v: # c\n  |\n  abc", "v: !!str &a |\n  abc\nw: *a", "|\n abc", "--- >\n abc\n  ",
    "? |\n  k\n: v", "v: |\n\n\n  abc\n", "v: |\n   \n  abc",
    "v: !<tag:yaml.org,2002:str> &n0 |-\n  abc\n", "v: !<tag:yaml.org,2002:str> >\n  a\n  b",
]
DOCUMENTS = BLOCKS + [
    # `=` (the value tag): a mapping key becomes the string, anywhere else it
    # has no constructor
    "=: x", "{=: x}", "? =\n: 1", "! =: x", "'=': x", "a:\n  =: x", "a: [{=: 1}]",
    "!!value =: x", "!!value foo: x", "=: =", "a: =", "[=]", "- =", "=", "a: {=: 1, b: =}",
    "=:\n  =: 1",
    # SafeConstructor.construct_scalar: a scalar tag on a mapping takes the text
    # of its first `=` value (its other entries are never constructed); a
    # mapping is filled after the scalars beside it, which can still see `=`
    "!!str {=: a, 1: x, b: !!binary zz}", "!!int {=: 0x1F}", "!!str {=: !!int abc}", "!!str {=: =}",
    "!!str {=: {=: deep}}", "!!str {=: [1]}", "!!null {=: x}", "!!null {a: 1}",
    "!!timestamp {=: 2026-01-02}", "!!value {=: x}", "? !!str {=: k}\n: v", "!!seq {=: x}",
    "!!map {=: x}", "! {=: x}", "!!str {a: 1, =: x, =: y}", "x: !!int\n  =: 7\n  1: 2",
    "!!bool {=: yes}", "!!float {=: 1:30.5}", "!!str {<<: {a: 1}, =: q}",
    "a: !!str {=: x, b: &n !!int abc}\nc: 1", "a: !!str {=: x, b: &n !!int abc}\nc: *n",
    "a: !!str {=: x, b: &n 5}\nc: *n", "a: !!str {=: x, b: &r [*r]}",
    "a: {&k =: 1}\nb: !!str {*k : z}", "a: {&k =: 1}\nb: *k", "- {&k =: 1}\n- [*k]",
    "a: [{&k =: 1}]\nb: [*k]", "a: [{&k =: 1}]\nb: [[*k]]", "a: {&k =: 1}\nb: [*k]",
    "a: {&k =: 1, *k : 2}", "!!str\n=: a",
    # PyYAML builds non-string keys, bytes, sets and lists of pairs; v3 cannot
    # hold them, so they are an error only if they stay in the value (a
    # repeated key drops them). Unhashable keys and bad content fail at once.
    "a:\n  - {~: 'a'}\na: a", "a: !!binary 1_000\na: 1", "a: !!binary x\na: 1",
    "a: !!binary abc\na: 1", "a: !!binary a===\na: 1", "a: !!binary ab=c\na: 1",
    "a: !!binary \u00e9\na: 1", "a: {1: x, 1.0: y, true: z}\na: 2", "a: {[1]: x}\na: 2",
    "? !!binary aGk=\n: x", "? !!set {a}\n: x", "? !!omap []\n: x", "a: !!set {[1]: x}\na: 1",
    "a: !!set [1]\na: 1", "a: !!set {a: !!int x}\na: 1", "a: !!set {a}\na: 1",
    "a: !!omap [{[1]: 2}]\na: 1", "a: !!omap [{a: 1, b: 2}]\na: 1", "a: !!omap [a]\na: 1",
    "a: !!omap [{a: !!int x}]\na: 1", "a: !!pairs {a: 1}\na: 1", "a: !!pairs [{a: 1}]\na: 1",
    "a: !!binary {=: aGk=}\na: 1", "a: !!binary aGk=", "a: {1: x}", "a: !!set {a}",
    # a recursive alias is an error only if the cycle stays in the value
    "&n0\na: *n0\na: []", "&n0\na: *n0", "a: &x [*x]\na: 1", "- &x [*x, 1]\n- 2",
    # anchors: a name defined twice is an error, even without an alias
    "a: &x [1]\nb: &x [2]\nc: *x", "a: &x 1\nb: &x 1", "a: &x [&x 1]",
    "a: &x 1\nb: *x\nc: &x 2", "&x\n- 1\n- &x 2", "a: &x !!int '5'\nb: &x 6",
    "a: !!str &x 5\nb: &x 6", "{a: &x 1, b: &x 2}", "[&x 1, &x 2]", "? &k a\n: &k b",
    "a: &x\nb: &x", "- &x\n- &x",
    # anchor-like text that is not an anchor
    "a: &x 1\nb: &y 2\nc: *x", "a: &x 1 # &x\nb: 2", "a: &x '&x'\nb: \"&x\"",
    "a: b &x\nc: &x 1", "a: &x |\n  &x\nb: 1", "a: b\n  &x c\nd: &x 1",
    "a: &x 1\n# &x\nb: *x", "- &a 1\n- &b [*a]\n- *b", "a: &x-y_9 1\nb: *x-y_9",
    "&x a: *x", "--- &x\na: 1", "a: &x\n  b: 1\nc: *x",
    # PyYAML's anchor and alias names end at the first character outside
    # [0-9A-Za-z_-]; one of ?:,]}%@` may follow at once (`&a: x` anchors an
    # empty key), anything else is an error
    "!!str &a: []", "- []\n- !!str &a: []", "&a: []", "&a: 1", "!!str &a: x", "- &a: 1", "&a:x",
    "&a: &b: 1", "!!int &a: 1", "{&a: 1}", "{&a:1}", "[&a: 1]", "x: &a:\n  b: 1",
    "!!str &a:\n", "&a:", "x: !!str &k: v", "&a?x", "&a? x", "&a%", "&a@", "&a`", "&a,", "[&a, 1]",
    "[&a]", "{&a}", "&a :x", "&a : x", "a: &x 1\nb: *x: 2", "a: &x 1\n*x: 2",
    "a: &x 1\nb: [*x:]", "a: &x 1\nb: {*x: 2}", "a: &x 1\nb: {*x:2}", "a: &x 1\nb: [*x, *x:]", "! &a: []", "! &a:", "[! &a]", "{! &a}",
    "a: &x 1\nb: [*x?]", "a: &x 1\nb: *x?", "!!str &a: !!str &b: x", "- !!str &a: a\n  !!str &b: b",
    "!!str &a: x\n!!str &b: y", "!!str &a: x\n!!str &a: y", "!!str &a: &n x\nb: *a",
    "!!str &a-b_9: x", "!!str &a: |\n  abc", "!!str &a: |\n  \u00e9", "  !!str &a: x\n  b: c",
    "x:\n  !!str &a: |-\n    abc\n    \u00e9",
    "a: &x k\nb: {*x: 2}", "a: &x k\nb: {*x:2}", "a: &x k\n*x: 2", "a: &x k\nb: [*x:]",
    "a: &x k\nb: [*x?]", "a: &x k\nb:\n  *x: 2\n  c: 3", "a: &x k\n- *x:",
    "&n0\n!!str &n1: []", "&n0\n!!str &n1: *n0", "&n0 !!str &n1: x", "- &n0 a\n- *n0:",
    "&n0\n!!str &n1: *n0\n*n1:\n  - []", "a: &n0 k\nb: {*n0:1, *n0: 2}",
    "!!map &n0\n!!str &n1: []", "!!map &n0\n# c &n9:\n!!str &n1: []", "- !!seq &n0\n  - !!str &n1: x",
    # flow indicators glued to what follows: PyYAML's scanner, not YAML 1.2
    "v: {a:[]}", "{a:{}}", "{a:[b]}", "[a:[]]", "{a:b}", "{a: b}", "{a:,b}", "{\"a\":1}", "{'a':1}",
    "[?x]", "{?x: y}", "[? x]", "{? x}", "[?]", "{?}", "[a?b]", "{a?b: c}", "[-a]", "[- a]", "{-a: b}",
    "[&a?x]", "[&a ?x]", "{!!str &a: |}", "{!!str &a: >}", "[|]", "[>]", "{a: |}", "[!!str |]",
    "[a,]", "[,a]", "[a,,b]", "{a: 1,}", "{,}", "[]]", "[[]", "{a: [}]}", "[a:b]", "[a: b]", "[a :b]",
    "[a: [b]]", "{a: {b: [c, {d:e}]}}", "[{a: b}, c]", "[*a]", "&a [*a]", "[&a [*a]]", "{a:#c\n b}",
    "{a: b #c\n}", "[a #c\n, b]", "[a\n,b]", "{a\n: b}", "[\"a\"b]", "['a'b]", "[\"a\":b]",
    "[!!str a:b]", "{!!str : x}", "{: x}", "[: x]", "{a: !!str}", "[!!str]", "{!!str}", "[!a]",
    "[!<!>a]", "!<tag:yaml.org,2002:str>a", "[!!str,]", "{a: !!str,}", "-[a]", "-  [a]", "- [a]",
    "? [a]: b", "[a]: b", "{a}: b", "a: [b]: c", "[a, b]: c\nd: e",
    # line breaks PyYAML also breaks at (U+0085, U+2028, U+2029) and characters
    # its reader refuses
    "a: b\u0085c: d", "a: b\u2028c: d", "a: b\u2029c: d", "a: 'b\u2028c'", 'a: "b\u0085c"',
    "a: |\n  b\u2028  c", "a: |\n  b\u0085  c\n", "a: >\n  b\u2029  c", "- a\u2028- b",
    "a: b\u2028\u2028c: d", "a: b \u0085 c", "# c\u0085a: 1", "a: \u0085", "a:\u2028  b: 1",
    "a: \x07", "a: \x00", "a: \x7f", "a: \x80", "a: \x9f", "a: \ufffe", "a: \uffff", "a: \x1b[0m",
    "a: \x0b", "a: \x0c", "a: \x1c", "a: \x1e", "a: \x1f", "a: b\x0c", 'a: "\\x07"', 'a: "\\x00"',
    'a: "\\ud800"', 'a: "\\U0010ffff"', 'a: "\\U00110000"', "a: \ufeffb", "\ufeffa: b",
    "a: b\n\ufeffc: d", "a: \ue000", "a: \U0001f680", "a: \t", "a:\tb", "a: b\tc", "\ta: b",
    "a:\n\tb: c", "a: |\n\tb", "a: |\n  b\n\tc",
    # nesting depth
    "[" * 50 + "]" * 50, "a:\n" + "".join("  " * i + "- \n" for i in range(1, 40)) + "  " * 40 + "x",
    # anchor names PyYAML does not accept
    "a: &\u00e9 1", "a: &x\u00e9 1", "a: &x. 1", "a: &x: 1", "&x: 1",
]


# Python 3.11 converts at most 4,300 decimal digits (sys.int_info.
# default_max_str_digits); leading zeros count, a sign and whitespace do not;
# octal, binary and hexadecimal are not limited, nor are floats.
LONG = [
    (prefix, digit, count, suffix)
    for count in (4300, 4301)
    for prefix, digit, suffix in (
        ("v: ", "1", ""), ("v: ", "0", ""), ("v: -", "1", ""), ("v: 1_", "1", ""), ("v: 0", "7", ""),
        ("v: 0b", "1", ""), ("v: 0x", "f", ""), ("v: ", "1", ":30"), ('v: !!int "1:', "0", '"'),
        ('v: !!int "', "1", ':30"'), ('v: !!int "', "\u0661", '"'), ('v: !!int " ', "1", ' "'),
        ('v: !!int "0x', "1", '"'), ("v: !!float ", "1", ""), ("v: ", "1", ".5"), ("v: ", "1", ":30.5"),
        ("v: !!str ", "1", ""), ("v: '", "1", "'"), ("%YAML 1.", "1", "\n--- a"),
        ("%YAML ", "1", ".1\n--- a"), ("v: 2026-01-02 10:00:00.", "1", ""),
    )
]


class OutsideModel(Exception):
    """A value PyYAML builds that the v3 value model does not hold."""


def rust_str(text: str) -> str:
    if any(ord(ch) < 0x20 for ch in text) or not text.isprintable():
        escaped = "".join(
            f"\\u{{{ord(ch):x}}}" if not ch.isprintable() or ch in '"\\' else ch for ch in text
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
        digits = unlimited(lambda: str(abs(value)))
        if len(digits) > 60:
            sign = "-" if value < 0 else ""
            return f'big("{sign}", {len(digits)}, "{digits[:10]}", "{digits[-10:]}")'
        return f'int("{value}")'
    if isinstance(value, float):
        return f'flt("{value!r}")'
    if isinstance(value, str):
        if any(0xD800 <= ord(ch) <= 0xDFFF for ch in value):
            raise OutsideModel("a lone surrogate")  # a Rust string cannot hold one
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
    if isinstance(value, list):
        return "sq(vec![" + ", ".join(rust_value(v) for v in value) + "])"
    if isinstance(value, dict) and all(isinstance(k, str) for k in value):
        rows = ", ".join(f"({rust_str(k)}, {rust_value(v)})" for k, v in value.items())
        return f"mp(vec![{rows}])"
    raise OutsideModel(repr(value))


DISAGREE: list[str] = []


def unlimited(describe):
    """``describe()`` with Python's int/str digit limit lifted: describing a
    result PyYAML built must not fail where PyYAML did not. Loading always
    runs with the default limit."""
    sys.set_int_max_str_digits(0)
    try:
        return describe()
    finally:
        sys.set_int_max_str_digits(sys.int_info.default_max_str_digits)


def load_both(text: str) -> object:
    """``text`` loaded by ``SafeLoader``; a different libyaml reading is noted."""
    results = []
    for loader in (yaml.SafeLoader, yaml.CSafeLoader):
        try:
            results.append(("value", yaml.load(text, Loader=loader)))
        except Exception:  # noqa: BLE001 -- any PyYAML failure is an error value
            results.append(("error", None))
    if unlimited(lambda: repr(results[0]) != repr(results[1])):
        DISAGREE.append("error" if results[1][0] == "error" else repr(results[1][1]))
    kind, value = results[0]
    if kind == "error":
        raise ValueError(text)
    return value


def main() -> int:
    if "--long" in sys.argv[1:]:
        for prefix, digit, count, suffix in LONG:
            text = prefix + digit * count + suffix
            try:
                value = load_both(text)
            except ValueError:
                expected = "err()"
            else:
                value = value["v"] if isinstance(value, dict) and "v" in value else value
                if isinstance(value, str) and len(value) > 60:
                    expected = f"long_str({len(value)})"
                else:
                    expected = rust_value(value)
            parts = f"({rust_str(prefix)}, {rust_str(digit)}, {count}, {rust_str(suffix)})"
            print(f"    ({parts}, {expected}),")
        return 0
    documents = "--documents" in sys.argv[1:]
    cases = DOCUMENTS if documents else TAGGED if "--tagged" in sys.argv[1:] else CASES
    for case in cases:
        try:
            value = load_both(case) if documents else load_both(f"v: {case}")["v"]
        except ValueError:
            expected = "err()"
        else:
            try:
                expected = rust_value(value)
            except (OutsideModel, RecursionError):  # a cycle is outside the model too
                expected = "err()"
        note = f"  // CSafeLoader: {DISAGREE.pop()}" if DISAGREE else ""
        print(f"    ({rust_str(case)}, {expected}),{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
