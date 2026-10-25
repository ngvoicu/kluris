"""Differential fuzzing of the Kluris 3 YAML writer against PyYAML 6.0.3.

Hypothesis generates mappings of the v3 value model (strings built from YAML
indicators, YAML 1.1 keywords and number/date look-alikes, control characters,
astral Unicode and lines past 80 columns; nested lists and maps; dates and
datetimes; floats with inf/nan; big ints). Each case is written in every
profile and compared, byte for byte, with what v2 writes
(``tools/yaml_oracle.py`` in the oracle venv) and
``cargo xtask yaml-emit --profile md|yml|config``; ``cargo xtask yaml-load``
(the Rust reader) must then read PyYAML's output to the value PyYAML reads.

One persistent process per side and profile, one JSON line per case, the
three profiles in parallel. Cases (each checked in all three profiles):
``KLURIS_YAML_FUZZ_EXAMPLES`` (default 10000).
"""

from __future__ import annotations

import datetime
import json
import os
import subprocess
import tempfile
from pathlib import Path

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris
from tools.tagged_json import decode, encode, same

WORKSPACE = PARITY_DIR.parent
MAX_EXAMPLES = int(os.environ.get("KLURIS_YAML_FUZZ_EXAMPLES", "10000"))
PROFILES = ["md", "yml", "config"]

# Known reader gap, decided 2026-10-25 (crates/kluris-core/src/frontmatter.rs,
# `universal_newlines`): PyYAML breaks lines at U+2028/U+2029, the Rust reader
# does not. The md profile writes them raw (libyaml counts them printable), so
# its round trip is not compared when they occur; the emitted bytes still are.
READER_GAP = ("\u2028", "\u2029")


class LineProcess:
    """A child process answering one JSON line per JSON line. ``send`` and
    ``receive`` are split so several processes work on a case at once; each
    has at most one request outstanding."""

    def __init__(self, args: list[str], cwd: Path):
        self.args = args
        self.stderr = tempfile.TemporaryFile()
        self.proc = subprocess.Popen(
            args,
            cwd=cwd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            text=True,
            encoding="utf-8",
        )

    def _stopped(self) -> None:
        self.stderr.seek(0)
        err = self.stderr.read().decode("utf-8", "replace")
        pytest.fail(f"{' '.join(self.args)} stopped answering:\n{err}")

    def send(self, request: object) -> None:
        assert self.proc.stdin is not None
        try:
            self.proc.stdin.write(json.dumps(request) + "\n")
            self.proc.stdin.flush()
        except OSError:
            self._stopped()

    def receive(self) -> object:
        assert self.proc.stdout is not None
        line = self.proc.stdout.readline()
        if not line:
            self._stopped()
        return json.loads(line)

    def close(self) -> None:
        try:
            if self.proc.stdin is not None:
                self.proc.stdin.close()
        except OSError:
            pass  # it already stopped; the test that saw it has failed
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.stderr.close()


@pytest.fixture(scope="module")
def pyyaml():
    venv = oracle_kluris().parent
    python = venv / ("python.exe" if IS_WINDOWS else "python")
    script = str(PARITY_DIR / "tools" / "yaml_oracle.py")
    procs = {p: LineProcess([str(python), "-I", script], WORKSPACE) for p in PROFILES}
    yield procs
    for proc in procs.values():
        proc.close()


@pytest.fixture(scope="module")
def rust():
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE, capture_output=True, text=True
    )
    assert build.returncode == 0, build.stderr
    procs = {}
    for p in PROFILES:
        procs[p] = LineProcess(["cargo", "--quiet", "xtask", "yaml-emit", "--profile", p], WORKSPACE)
        procs[f"load-{p}"] = LineProcess(["cargo", "--quiet", "xtask", "yaml-load"], WORKSPACE)
    yield procs
    for proc in procs.values():
        proc.close()


# --- the values ------------------------------------------------------------

KEYWORDS = [
    "yes", "Yes", "NO", "on", "Off", "true", "False", "y", "n", "null", "Null", "~", "",
    "<<", "=", "NaN", "nan", ".inf", "-.inf", ".NaN", "1e3", "1.0e+3", "0o17", "0x1F",
    "0b101", "1_000", "1:30", "190:20:30", "012", "0755", "+1", "-0", "1.0", "1.", "2.10",
    ".5", "2026-01-02", "2026-1-2", "2026-01-02 10:00", "2026-01-02T10:00:00Z",
    "2026-01-02 10:00:00 +05:30", "2026-02-30", "0000-01-01", "---", "...", "- ", "? ",
    ": ", " #", "# ", "http://x.y/z?q=1#f", "./map.md", "word", "a b",
]
SPECIAL = (
    "-?:,[]{}#&*!|>'\"%@`.~=<+_\\ \t\n\r"
    "\x00\x07\x1b\x7f\x85\x9f\x0b\x0c\u2028\u2029\ufeff\ufffe\uffff"
    "é日\xa0\u200b—\U0001F680\U0010ffff\U00010000\ue000\ufffd"
)
# Rust strings cannot hold lone surrogates; nothing Kluris reads produces them.
ANY_CHAR = st.characters(exclude_categories=["Cs"])

pieces = st.one_of(
    st.sampled_from(KEYWORDS),
    st.text(st.sampled_from(SPECIAL), max_size=4),
    st.text(ANY_CHAR, max_size=4),
    st.sampled_from([" ", "  ", "word ", "x" * 20, "é" * 10, "\U0001F680 "]),
)
short_strings = st.lists(pieces, max_size=6).map("".join)
# Lines past 80 columns from a few repeated chunks (cheap to draw).
CHUNKS = [
    "word ", "word", " ", "  ", "longerword ", "x" * 10, "é", "日本 ", "\U0001F680 ", "\t", "\x07", "\n",
    "- ", "# ", ": ", "'", '"', "\\", "\x85", " ", "https://example.com/a/very/long/path/segment",
]
long_strings = st.lists(
    st.tuples(st.sampled_from(CHUNKS), st.integers(1, 12)).map(lambda chunk: chunk[0] * chunk[1]),
    min_size=2,
    max_size=8,
).map("".join)
# Text that stays plain or single-quoted, folded past 80 columns.
WORDS = [
    "word", "a", "longerword", "x" * 25, "x:y", "a#b", "-a", "it's", "C++", "./map.md", "é", "日本語",
    "yes", "#tag", "a:", "https://example.com/a/very/long/path/segment", "", "\n",
]
words = st.lists(st.sampled_from(WORDS), min_size=1, max_size=30).map(" ".join)
strings = st.one_of(short_strings, words, words, long_strings)
# Keys around the simple-key limits: 128 characters (PyYAML) and 128 bytes (libyaml).
long_keys = st.text(st.sampled_from("kxé日 \n\r\U0001F680"), min_size=40, max_size=140)
boundary_keys = st.builds(
    lambda n, fill, m: "k" * n + fill * m,
    st.integers(100, 130),
    st.sampled_from(["é", "日", "\U0001F680", " k", "k"]),
    st.integers(0, 12),
)
keys = st.one_of(short_strings, short_strings, long_keys, boundary_keys)

UTC = datetime.timezone.utc


def offset(seconds: int) -> datetime.timezone:
    return datetime.timezone(datetime.timedelta(seconds=seconds))


timezones = st.one_of(
    st.none(),
    st.just(UTC),
    st.integers(-(24 * 60) + 1, 24 * 60 - 1).map(lambda minutes: offset(minutes * 60)),
    st.integers(-86399, 86399).map(offset),
)
floats = st.one_of(
    st.floats(),
    st.sampled_from([0.0, -0.0, 1e15, 1e16, 1e17, 1e-4, 1e-5, 5e-324, 1.7976931348623157e308, 0.1 + 0.2]),
)
ints = st.one_of(st.integers(), st.integers(-(10**40), 10**40), st.sampled_from([2**63, -(2**63) - 1]))
scalars = st.one_of(
    st.none(),
    st.booleans(),
    ints,
    floats,
    st.dates(),
    st.datetimes(timezones=timezones),
    strings,
    strings,
    strings,
)
values = st.recursive(
    scalars,
    lambda children: st.one_of(st.lists(children, max_size=4), st.dictionaries(keys, children, max_size=4)),
    max_leaves=6,
)
documents = st.dictionaries(keys, values, max_size=6)


@settings(
    max_examples=MAX_EXAMPLES,
    deadline=None,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.data_too_large],
)
@given(document=documents)
def test_yaml_writer_matches_pyyaml(document, pyyaml, rust):
    value = encode(document)
    for p in PROFILES:
        pyyaml[p].send({"profile": p, "value": value})
        rust[p].send(value)
    oracle = {p: pyyaml[p].receive() for p in PROFILES}
    written = {p: rust[p].receive() for p in PROFILES}
    for p in PROFILES:
        rust[f"load-{p}"].send(oracle[p]["yaml"])
    reread = {p: rust[f"load-{p}"].receive() for p in PROFILES}

    problems = []
    for p in PROFILES:
        expected = oracle[p]["yaml"]
        if written[p] != expected:
            problems.append(f"[{p}] PyYAML writes {expected!r}\n[{p}] Rust writes   {written[p]!r}")
        if "reread_error" in oracle[p]:
            if "error" not in reread[p]:
                problems.append(f"[{p}] PyYAML cannot read {expected!r} ({oracle[p]['reread_error']}), Rust can")
        elif p == "md" and any(ch in expected for ch in READER_GAP):
            pass
        elif "value" not in reread[p]:
            problems.append(f"[{p}] Rust cannot read {expected!r}: {reread[p].get('error')}")
        elif not same(decode(reread[p]["value"]), decode(oracle[p]["reread"])):
            problems.append(
                f"[{p}] Rust reads {expected!r} as {reread[p]['value']!r}, PyYAML as {oracle[p]['reread']!r}"
            )
    assert not problems, "\n".join(problems)
