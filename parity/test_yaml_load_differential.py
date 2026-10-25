"""Differential fuzzing of the Kluris 3 YAML reader against PyYAML 6.0.3 on raw
YAML text.

Hypothesis builds YAML documents from fragments: block mappings and sequences,
flow collections, plain, quoted and block scalars (every chomping indicator,
indentation indicators, with and without a final line break), YAML 1.1
look-alikes, `=`, anchors (repeated and with names PyYAML refuses), aliases,
core and unsupported tags, comments, non-ASCII text, CRLF. Each text goes to
``cargo xtask yaml-load`` (the Rust reader) and to ``tools/yaml_load_oracle.py``
(``yaml.safe_load`` and ``CSafeLoader`` in the oracle venv, both refusing merge
keys as v3 does). The Rust reader must agree with both: the same value (exact
types, key order), or an error where they fail. Error messages are not
compared. Where the two PyYAML loaders disagree with each other (libyaml's
scanner accepts a few anchors and indentations the pure-Python one refuses),
the Rust reader must agree with ``SafeLoader``, whose scanner it follows;
``KLURIS_YAML_LOAD_DISAGREEMENTS=<file>`` appends those texts to a file.

The one accepted difference (decided 2026-10-25,
``crates/kluris-core/src/frontmatter.rs``): PyYAML breaks lines at U+0085,
U+2028 and U+2029, the Rust reader does not; texts holding them are not
compared.

Cases: ``KLURIS_YAML_LOAD_FUZZ_EXAMPLES`` (default 5000).
"""

from __future__ import annotations

import json
import os
import subprocess

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris
from test_yaml_differential import LineProcess
from tools.tagged_json import decode, same

WORKSPACE = PARITY_DIR.parent
MAX_EXAMPLES = int(os.environ.get("KLURIS_YAML_LOAD_FUZZ_EXAMPLES", "5000"))
LINE_BREAK_GAP = ("\x85", " ", " ")


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
anchors = st.sampled_from([FRESH] * 40 + [REPEAT, REPEAT, "&\u00e9", "&a.", "&a:"])
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
alias = st.sampled_from([f"\x00L{i}" for i in range(4)] * 8 + ["*nope"])
flow_scalar = st.one_of(plain, single, double)
flow = st.recursive(
    flow_scalar,
    lambda inner: st.one_of(
        st.lists(st.tuples(props, inner).map("".join), max_size=3).map(lambda xs: "[" + ", ".join(xs) + "]"),
        st.lists(st.tuples(flow_scalar, props, inner), max_size=3).map(
            lambda kvs: "{" + ", ".join(f"{k}: {p}{v}" for k, p, v in kvs) + "}"
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
keys = st.tuples(
    props, st.one_of(st.sampled_from(KEYS), st.sampled_from(KEYS), st.sampled_from(KEYS + BAD_KEYS))
).map("".join)


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


texts = st.one_of(
    st.builds(document, nodes, st.booleans(), st.booleans(), st.just(False)),
    st.builds(document, nodes, st.booleans(), st.booleans(), st.just(True)),
)


DISAGREEMENTS = os.environ.get("KLURIS_YAML_LOAD_DISAGREEMENTS")


def agree(a: dict, b: dict) -> bool:
    if "error" in a or "error" in b:
        return "error" in a and "error" in b
    return same(decode(a["value"]), decode(b["value"]))


# The drawn trees are deep; Hypothesis warns that their repr is large.
@pytest.mark.filterwarnings("ignore:Generating overly large repr")
@settings(
    max_examples=MAX_EXAMPLES,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.data_too_large],
)
@given(text=texts)
def test_yaml_reader_matches_pyyaml_on_raw_text(text, loaders):
    if any(ch in text for ch in LINE_BREAK_GAP):
        return
    pyyaml, rust = loaders
    pyyaml.send(text)
    rust.send(text)
    oracle = pyyaml.receive()
    got = rust.receive()
    loaders = ("safe", "c")
    if not agree(oracle["safe"], oracle["c"]):
        loaders = ("safe",)
        if DISAGREEMENTS:
            with open(DISAGREEMENTS, "a", encoding="utf-8") as out:
                out.write(json.dumps({"text": text, **oracle}) + "\n")
    problems = []
    for loader in loaders:
        want = oracle[loader]
        if agree(want, got):
            continue
        reads = f"reads {want['value']!r}" if "value" in want else f"fails ({want['error']})"
        rust = f"reads {got['value']!r}" if "value" in got else f"fails ({got['error']})"
        problems.append(f"[{loader}] PyYAML {reads}, Rust {rust}")
    assert not problems, f"text {text!r}:\n" + "\n".join(problems)
