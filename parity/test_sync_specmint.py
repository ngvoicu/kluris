"""``cargo xtask sync-specmint`` against v2.28.11's ``scripts/sync-specmint.py``
(a byte-identical copy in ``tools/v2_sync_specmint.py``): the per-file report
and its ``--diff`` text (Python's ``difflib.unified_diff``, ``str.splitlines``
line counts) and the ``references/``/``assets/`` path-mention regex, on random
inputs answered by both (``cargo xtask sync-specmint-oracle``).
Cases: ``KLURIS_SYNC_FUZZ_EXAMPLES`` (default 10000).
"""

from __future__ import annotations

import base64
import os
import subprocess

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris
from test_yaml_differential import LineProcess

WORKSPACE = PARITY_DIR.parent
EXAMPLES = int(os.environ.get("KLURIS_SYNC_FUZZ_EXAMPLES", "10000"))


@pytest.fixture(scope="module")
def v2():
    venv = oracle_kluris().parent
    python = venv / ("python.exe" if IS_WINDOWS else "python")
    proc = LineProcess([str(python), "-I", str(PARITY_DIR / "tools" / "sync_specmint_oracle.py")], WORKSPACE)
    yield proc
    proc.close()


@pytest.fixture(scope="module")
def v3():
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE, capture_output=True, text=True
    )
    assert build.returncode == 0, build.stderr
    proc = LineProcess(["cargo", "--quiet", "xtask", "sync-specmint-oracle"], WORKSPACE)
    yield proc
    proc.close()


def ask(v2, v3, request):
    v2.send(request)
    v3.send(request)
    return v2.receive(), v3.receive()


# Few distinct lines, so files share many: difflib's matching, its "popular"
# junk heuristic (200+ lines) and every splitlines separator.
line_pieces = st.sampled_from([
    "a\n", "b\n", "c\n", "\n", "a", "x y\n", "é\n", "\r\n", "q\r", "\x0b", "\x0c", "\x1c", " ",
    "\x85", "日本\n", "  indented\n", "\t\n",
])
content = st.one_of(
    st.lists(line_pieces, max_size=30).map("".join).map(str.encode),
    st.lists(st.sampled_from(["a\n", "b\n", "c\n", "d\n"]), min_size=150, max_size=420).map("".join).map(str.encode),
    st.binary(max_size=12),
)
paths = st.sampled_from(["SKILL.md", "references/a.md", "references/b.md", "assets/x.js", "zz.md"])
trees = st.dictionaries(paths, content, max_size=4)


def b64(tree: dict[str, bytes]) -> dict[str, str]:
    return {k: base64.b64encode(v).decode("ascii") for k, v in tree.items()}


# Long files of many distinct lines, so some occur just around difflib's
# popularity threshold (``len(b) // 100 + 1``), and edits inside them.
long_lines = st.lists(st.integers(0, 90).map(lambda i: f"l{i}\n"), min_size=190, max_size=420)
line_edits = st.lists(st.tuples(st.integers(0, 419), st.integers(0, 95).map(lambda i: f"l{i}\n")), max_size=6)


@settings(max_examples=EXAMPLES, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(lines=long_lines, edits=line_edits, full=st.booleans())
def test_report_of_an_edited_long_file_matches_v2(lines, edits, full, v2, v3):
    new = list(lines)
    for at, line in edits:
        if at % 3 == 0:
            new.insert(at % (len(new) + 1), line)
        elif at % 3 == 1 and new:
            del new[at % len(new)]
        elif new:
            new[at % len(new)] = line
    old_tree = {"SKILL.md": "".join(lines).encode()}
    new_tree = {"SKILL.md": "".join(new).encode()}
    request = {"report": {"name": "specmint-core", "old": b64(old_tree), "new": b64(new_tree), "full_diff": full}}
    expected, got = ask(v2, v3, request)
    assert got == expected, f"v2 {expected!r}\nv3 {got!r}"


@settings(max_examples=EXAMPLES, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(old=trees, new=trees, edits=st.lists(st.tuples(paths, content), max_size=2), full=st.booleans())
def test_report_matches_v2(old, new, edits, full, v2, v3):
    # Mostly-similar trees: start from ``old`` and edit a few files.
    if edits:
        new = dict(old)
        for path, data in edits:
            new[path] = data
    request = {"report": {"name": "specmint-core", "old": b64(old), "new": b64(new), "full_diff": full}}
    expected, got = ask(v2, v3, request)
    assert got == expected, f"v2 {expected!r}\nv3 {got!r}"


mention_pieces = st.sampled_from([
    "references/", "assets/", "references", "assets", "a", "b.md", ".", "/", "-", "_", "*", "x", "é",
    " ", "`", "(", "1", "\n", "Ⅻ", "٣", "ß",
])


@settings(max_examples=EXAMPLES, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(text=st.lists(mention_pieces, max_size=14).map("".join))
def test_path_mentions_match_v2(text, v2, v3):
    expected, got = ask(v2, v3, {"mentions": text})
    assert got == expected, f"{text!r}: v2 {expected!r}, v3 {got!r}"
