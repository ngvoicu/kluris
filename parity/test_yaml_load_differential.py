"""Differential fuzzing of the Kluris 3 YAML reader against PyYAML 6.0.3 on raw
YAML text.

Hypothesis builds YAML documents from fragments: block mappings and sequences,
flow collections (also glued: no blanks around `?`, `:`, `,`, brackets,
anchors, tags and block-scalar indicators), plain, quoted and block scalars
(every chomping indicator, indentation indicators, with and without a final
line break), YAML 1.1 look-alikes, `=`, anchors (repeated and with names
PyYAML refuses), aliases, core and unsupported tags, comments, non-ASCII text,
CRLF, characters PyYAML's reader treats specially (U+0085, U+2028 and
U+2029 break lines; tabs, controls, BOMs), and numbers with the whitespace
Python's int() and float() skip or refuse, or around their 4,300-digit limit. Each text goes to
``cargo xtask yaml-load`` (the Rust reader) and to ``tools/yaml_load_oracle.py``
in the oracle venv.

The oracle is ``yaml.safe_load`` (merge keys refused, as v3 does): v3 reads
every YAML the way PyYAML's pure-Python ``SafeLoader`` does. The Rust reader
must give the same value (exact types, key order), or an error where it
fails; error messages are not compared. Texts where libyaml's ``CSafeLoader``
(which v2 used for markdown frontmatter) reads otherwise are counted, not
failed: the count is printed at the end, and
``KLURIS_YAML_LOAD_DISAGREEMENTS=<file>`` appends those texts to a file.

Cases: ``KLURIS_YAML_LOAD_FUZZ_EXAMPLES`` (default 5000).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris
from test_yaml_differential import LineProcess
from tools.tagged_json import decode, same

WORKSPACE = PARITY_DIR.parent
MAX_EXAMPLES = int(os.environ.get("KLURIS_YAML_LOAD_FUZZ_EXAMPLES", "5000"))


@pytest.fixture(scope="module")
def loaders():
    venv = oracle_kluris().parent
    python = venv / ("python.exe" if IS_WINDOWS else "python")
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE, capture_output=True, text=True
    )
    assert build.returncode == 0, build.stderr
    pyyaml = LineProcess([str(python), "-I", str(PARITY_DIR / "tools" / "yaml_load_oracle.py")], WORKSPACE)
    rust = LineProcess(["cargo", "--quiet", "xtask", "yaml-load"], WORKSPACE)
    yield pyyaml, rust
    pyyaml.close()
    rust.close()


# --- the fragments -----------------------------------------------------------

WORDS = [
    "a", "b", "key", "x y", "é", "日本", "\U0001F680", "café", "naïve text", "yes", "No", "on", "OFF", "y",
    "n", "~", "null", "NULL", "true", "=", "<<", "1", "-1", "+1", "0", "012", "0755", "08", "0o17", "0x1F",
    "0b101", "1_000", "1:30", "-1:30", "1:60", "190:20:30", "1.5", "1.", ".5", "-.5", "1e3", "1.0e+3",
    "2.10", ".inf", "-.Inf", ".nan", "NaN", "2026-01-02", "2026-1-2", "2026-01-02 10:00",
    "2026-01-02T10:00:00Z", "2026-1-2 3:04:05.5 +01", "2026-02-30", "0000-01-01", "a#b", "x:y", "-a",
    "./map.md", "٣", "１２", "q&r", "*star", "a &x", "%pct", "@at",
]
words = st.sampled_from(WORDS)
plain = st.lists(words, min_size=1, max_size=3).map(" ".join).filter(
    lambda w: not w.startswith(("*", "%", "@", "<<", "-a", "&")) or w in ("<<", "-a")
)
QUOTABLE = WORDS + ["", " lead", "trail ", "it's", 'say "hi"', "\\", "line\\nbreak", "tab\\there", "\\u00e9"]
single = st.sampled_from(QUOTABLE).map(lambda w: "'" + w.replace("'", "''") + "'")
double = st.sampled_from(QUOTABLE).map(lambda w: '"' + w.replace('"', '\\"') + '"')
# Anchors are placeholders numbered in document order (``number_anchors``):
# mostly a fresh name, sometimes the previous name again (an error), a few names
# PyYAML refuses. An alias names an anchor already defined, or none.
FRESH, REPEAT = "\x00A", "\x00R"
# PyYAML ends a name at the first character outside [0-9A-Za-z_-]; one of
# ?:,]}%@` may follow at once (`&a: x` anchors an empty key).
FOLLOWERS = [":", ":", "?", ",", "]", "}", "%", "@", "`"]
anchors = st.one_of(
    st.sampled_from([FRESH] * 40 + [REPEAT, REPEAT, "&\u00e9", "&a.", "&a:"]),
    st.sampled_from([FRESH] * 40 + [FRESH + f for f in FOLLOWERS]),
)
# Mostly tags inside the value model.
TAGS = [
    "!!str", "!!int", "!!float", "!!bool", "!!null", "!!timestamp", "!", "!!map", "!!seq", "!!value",
    "!<tag:yaml.org,2002:str>", "!!binary", "!foo", "!!set",
]
tags = st.one_of(st.sampled_from(TAGS[:9]), st.sampled_from(TAGS))
props = st.one_of(
    st.just(""),
    st.just(""),
    st.just(""),
    st.just(""),
    anchors.map(lambda a: a + " "),
    anchors.map(lambda a: a + " "),
    tags.map(lambda t: t + " "),
    st.tuples(tags, anchors).map(lambda p: f"{p[0]} {p[1]} "),
)
# Collections mostly take their own tag; a scalar tag on a mapping reads its
# `=` value.
def collection_props(own: str, others: list[str]):
    tags = st.sampled_from([own, "!"] * 6 + others)
    return st.one_of(
        st.just(""),
        st.just(""),
        st.just(""),
        anchors.map(lambda a: a + " "),
        tags.map(lambda t: t + " "),
        st.tuples(tags, anchors).map(lambda p: f"{p[0]} {p[1]} "),
    )


map_props = collection_props("!!map", ["!!str", "!!int", "!!null", "!!seq"])
seq_props = collection_props("!!seq", ["!!str", "!!map"])
alias = st.sampled_from([f"\x00L{i}" for i in range(4)] * 8 + ["*nope", "\x00L0:", "\x00L1:", "\x00L0?"])
flow_scalar = st.one_of(plain, single, double)
# Keys glued to their `:` (PyYAML ends the name there): an alias to a string,
# or an empty `!!str` scalar with an anchor; in a flow mapping the `:` may
# have no blank after it.
GLUED_KEYS = ["\x00L0", "\x00L1", "!!str \x00A"]
flow_keys = st.one_of(
    flow_scalar.map(lambda key: (key, ": ")),
    flow_scalar.map(lambda key: (key, ": ")),
    st.tuples(st.sampled_from(GLUED_KEYS), st.sampled_from([":", ": "])),
)
flow = st.recursive(
    flow_scalar,
    lambda inner: st.one_of(
        st.lists(st.tuples(props, inner).map("".join), max_size=3).map(lambda xs: "[" + ", ".join(xs) + "]"),
        st.lists(st.tuples(flow_keys, props, inner), max_size=3).map(
            lambda kvs: "{" + ", ".join(f"{k}{sep}{p}{v}" for (k, sep), p, v in kvs) + "}"
        ),
    ),
    max_leaves=4,
)
CHOMPING = ["|", "|-", "|+", ">", ">-", ">+", "|2", ">1-", "|+1"]
BLOCK_LINES = ["abc", "é", "日本 \U0001F680", "", "  indented", "yes", "a: b", "# not a comment", "&a"]
block = st.tuples(
    st.sampled_from(CHOMPING),
    st.lists(st.sampled_from(BLOCK_LINES), min_size=1, max_size=4),
    st.sampled_from(["", "", "\n", "\n  "]),
)
# Mostly string keys; `=` keys (a string, or the value a scalar tag on the
# mapping takes) and a few keys outside the value model.
KEYS = ["a", "b", "c", "key", "é", "日本", "x y", "'1'", '"q"', "k\U0001F680", "=", "=", "'='", "! ="]
BAD_KEYS = ["<<", "1", "yes", "~", "2026-01-02"]
keys = st.one_of(
    st.tuples(
        props, st.one_of(st.sampled_from(KEYS), st.sampled_from(KEYS), st.sampled_from(KEYS + BAD_KEYS))
    ).map("".join),
    st.sampled_from(GLUED_KEYS),
)


def scalar_node():
    return st.one_of(
        st.tuples(st.just("inline"), props, flow),
        st.tuples(st.just("inline"), props, flow),
        st.tuples(st.just("inline"), st.just(""), alias),
        st.tuples(st.just("block"), props, block),
    )


nodes = st.recursive(
    scalar_node(),
    lambda inner: st.one_of(
        st.tuples(st.just("map"), map_props, st.lists(st.tuples(keys, inner), min_size=1, max_size=4)),
        st.tuples(st.just("seq"), seq_props, st.lists(inner, min_size=1, max_size=4)),
    ),
    max_leaves=8,
)


def render(node, indent: int) -> str:
    """The node's text after a `key:` or `-` indicator: a leading space and an
    inline value, or a line break and indented lines."""
    kind, prefix, body = node
    pad = " " * indent
    if kind == "inline":
        return " " + prefix + body
    if kind == "block":
        indicator, lines, tail = body
        text = "\n".join(pad + "  " + line if line else "" for line in lines)
        return " " + prefix + indicator + "\n" + text + tail
    head = (" " + prefix.rstrip()) if prefix else ""
    if kind == "map":
        items = [f"{pad}  {key}:{render(value, indent + 2)}" for key, value in body]
    else:
        items = [f"{pad}  -{render(value, indent + 2)}" for value in body]
    return head + "\n" + "\n".join(items)


def number_anchors(text: str) -> str:
    """Name the anchor and alias placeholders in document order."""
    defined: list[str] = []
    out = []
    parts = text.split("\x00")
    out.append(parts[0])
    for part in parts[1:]:
        if part[0] == "A" or (part[0] == "R" and not defined):
            defined.append(f"n{len(defined)}")
            out.append("&" + defined[-1] + part[1:])
        elif part[0] == "R":
            out.append("&" + defined[-1] + part[1:])
        elif defined:
            index = int(part[1])
            out.append("*" + defined[-1 - index % len(defined)] + part[2:])
        else:
            out.append("a" + part[2:])
    return "".join(out)


def document(root, comment: bool, final_break: bool, crlf: bool) -> str:
    kind, prefix, body = root
    if kind == "map":
        lines = [f"{key}:{render(value, 0)}" for key, value in body]
    elif kind == "seq":
        lines = [f"-{render(value, 0)}" for value in body]
    else:
        lines = ["v:" + render(root, 0)]
    text = "\n".join(lines)
    if prefix and kind in ("map", "seq"):
        text = prefix.rstrip() + "\n" + text
    if comment:
        text = "# é comment &a\n" + text
    if final_break:
        text += "\n"
    text = number_anchors(text)
    return text.replace("\n", "\r\n") if crlf else text


# Flow collections with nothing between the indicators and what follows them.
GLUED_ATOMS = [
    "a", "b", "x y", "é", "1", "~", "?x", "?", "-a", "- a", ":", ":x", "a:b", "a?b", "'q'", '"q"', "[]", "{}",
    "!!str", "!!str a", "!!int 1", "!", "!a", FRESH, FRESH + " a", FRESH + "?x", FRESH + ":", "\x00L0",
    "\x00L1:", "|", ">", "|-", "#c", "a #c", "a#b", "=", "<<", "",
]
glued_flow = st.recursive(
    st.sampled_from(GLUED_ATOMS),
    lambda inner: st.one_of(
        st.lists(inner, max_size=3).map(lambda xs: "[" + ",".join(xs) + "]"),
        st.lists(
            st.tuples(inner, st.sampled_from([":", ":", ": ", " :", ""]), inner), max_size=3
        ).map(lambda kvs: "{" + ",".join(f"{k}{sep}{v}" for k, sep, v in kvs) + "}"),
        st.tuples(st.sampled_from(["[", "{", "[?", "{?", "[-"]), inner, st.sampled_from(["]", "}", ",]", ",}"])).map(
            "".join
        ),
    ),
    max_leaves=6,
)
glued_texts = st.builds(
    lambda prefix, flow, rest: number_anchors(prefix + flow + rest),
    st.sampled_from(["", "v: ", "- ", "k:\n  ", "&a ", "!!map ", "? "]),
    glued_flow,
    st.sampled_from(["", "\n", "\nw: 1", ": x", "\n- b"]),
)
# Characters PyYAML's reader breaks lines at, refuses, or skips.
SPECIAL_CHARS = ["\x85", "\u2028", "\u2029", "\t", "\x07", "\x00", "\x0b", "\ufeff", "\ufffe", "\x1c", "\r"]
special = st.builds(
    lambda text, at, ch: text[: at % (len(text) + 1)] + ch + text[at % (len(text) + 1) :],
    st.builds(document, nodes, st.booleans(), st.booleans(), st.just(False)),
    st.integers(0, 10_000),
    st.sampled_from(SPECIAL_CHARS),
)
# Numbers Python's int() and float() convert: whitespace they skip (ASCII
# \t\n\v\f\r and space, Unicode spaces) or keep (U+001C..U+001F, which
# str.strip() would strip), written as double-quoted escapes.
NUMERIC_SPACES = ["", "", "\\t", "\\x0b", "\\x0c", "\\r", " ", "\\x1c", "\\x1d", "\\x1e", "\\x1f", "\\x85",
                  "\\xa0", "\\u2000", "\\u200b", "\\u2028", "\\u3000", "\\ufeff", "\\u180e"]
NUMERIC_BODIES = ["1", "-1", "1.5", ".5", "1:30", "1:30.5", "0x1F", "0b1", "0755", "1_0", "\\u0661", "1e3", "nan"]
spaced_numbers = st.builds(
    lambda tag, before, body, after, inner: f'v: {tag}"{before}{body.replace(":", inner + ":", 1)}{after}"',
    st.sampled_from(["!!int ", "!!float ", "!!int ", "!!float ", "! ", ""]),
    st.sampled_from(NUMERIC_SPACES),
    st.sampled_from(NUMERIC_BODIES),
    st.sampled_from(NUMERIC_SPACES),
    st.sampled_from(NUMERIC_SPACES),
)
# Decimal strings around Python 3.11's limit of 4,300 digits per int().
long_numbers = st.builds(
    lambda prefix, count, suffix: prefix + "1" * count + suffix,
    st.sampled_from(["v: ", "v: -", "v: !!int ", "v: !!float ", "v: 1_", "v: 0x", "v: 0", "v: !!int 1:", "%YAML 1."]),
    st.integers(4295, 4305),
    st.sampled_from(["", ":30", ".5", "\n--- a"]),
)
texts = st.one_of(
    spaced_numbers,
    long_numbers,
    st.builds(document, nodes, st.booleans(), st.booleans(), st.just(False)),
    st.builds(document, nodes, st.booleans(), st.booleans(), st.just(True)),
    glued_texts,
    glued_texts,
    special,
)


DISAGREEMENTS = os.environ.get("KLURIS_YAML_LOAD_DISAGREEMENTS")


@pytest.fixture(scope="module")
def libyaml_disagreements(request):
    """Texts where CSafeLoader reads otherwise than SafeLoader; their count is
    printed when the module ends."""
    texts: list[str] = []
    yield texts
    reporter = request.config.pluginmanager.get_plugin("terminalreporter")
    if reporter is not None:
        reporter.write_line(f"yaml-load differential: {len(texts)} texts where CSafeLoader reads otherwise")


def agree(a: dict, b: dict) -> bool:
    if "error" in a or "error" in b:
        return "error" in a and "error" in b
    # Reading back an int over 4,300 digits must not hit Python's str() limit:
    # this process only compares, PyYAML loaded in the oracle.
    sys.set_int_max_str_digits(0)
    try:
        return same(decode(a["value"]), decode(b["value"]))
    finally:
        sys.set_int_max_str_digits(sys.int_info.default_max_str_digits)


# The drawn trees are deep; Hypothesis warns that their repr is large.
@pytest.mark.filterwarnings("ignore:Generating overly large repr")
@settings(
    max_examples=MAX_EXAMPLES,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.data_too_large],
)
@given(text=texts)
def test_yaml_reader_matches_pyyaml_on_raw_text(text, loaders, libyaml_disagreements):
    pyyaml, rust = loaders
    pyyaml.send(text)
    rust.send(text)
    oracle = pyyaml.receive()
    got = rust.receive()
    if not agree(oracle["safe"], oracle["c"]):
        libyaml_disagreements.append(text)
        if DISAGREEMENTS:
            with open(DISAGREEMENTS, "a", encoding="utf-8") as out:
                out.write(json.dumps({"text": text, **oracle}) + "\n")
    want = oracle["safe"]
    if not agree(want, got):
        sys.set_int_max_str_digits(0)
        reads = f"reads {want['value']!r}" if "value" in want else f"fails ({want['error']})"
        rust = f"reads {got['value']!r}" if "value" in got else f"fails ({got['error']})"
        pytest.fail(f"text {text!r}:\nSafeLoader {reads}, Rust {rust}")
