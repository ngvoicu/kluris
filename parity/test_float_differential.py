"""Differential test of Python float formatting: ``kluris_core::pyrepr`` (through
``cargo xtask float-repr``) against ``repr(x)`` and ``json.dumps(x)`` in the
oracle venv, for a fixed corpus of IEEE 754 bit patterns: random patterns,
subnormals, exact half-way ties at the shortest length (where Rust's shortest
formatting rounds up and Python rounds half to even), few-bit mantissas at
every exponent, powers of 2 and 10 with their neighbours, the positional and
exponent boundaries (1e-4/1e-5, 1e16), and the specials.

One process per side reads the whole corpus. ``KLURIS_FLOAT_CORPUS`` sets the
corpus size (default 25000).
"""

from __future__ import annotations

import math
import os
import random
import struct
import subprocess
from decimal import Decimal

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris

WORKSPACE = PARITY_DIR.parent
CORPUS = int(os.environ.get("KLURIS_FLOAT_CORPUS", "25000"))
MIN_TIES = 100


def bits(x: float) -> int:
    return struct.unpack(">Q", struct.pack(">d", x))[0]


def from_bits(b: int) -> float:
    return struct.unpack(">d", struct.pack(">Q", b))[0]


def is_tie(x: float) -> bool:
    """The exact value is half-way between two shortest-length candidates."""
    if not math.isfinite(x) or x == 0:
        return False
    shortest = repr(abs(x)).split("e")[0].replace(".", "").lstrip("0")
    exact = format(Decimal(abs(x)), "f").replace(".", "").strip("0")
    return len(exact) == len(shortest) + 1 and exact.endswith("5")


def corpus(size: int) -> list[int]:
    rng = random.Random(20261026)
    patterns: set[int] = set()
    specials = [0.0, -0.0, math.inf, -math.inf, math.nan, 5e-324, -5e-324, 2.2250738585072014e-308,
                2.225073858507201e-308, 1.7976931348623157e308, 70368744177664.12,
                2.9802322387695312e-08, 0.0018625259399414062]
    patterns.update(bits(x) for x in specials)
    patterns.update({0x7FF0000000000001, 0xFFF8000000000000, 0x7FFFFFFFFFFFFFFF})  # NaN payloads
    # Powers of 2 and 10 and their neighbours, and the boundaries.
    anchors = [2.0**e for e in range(-1074, 1024)] + [10.0**e for e in range(-323, 309)]
    anchors += [1e-4, 1e-5, 1e15, 1e16, 1e17, 2.0**53, 9999999999999998.0]
    for x in anchors:
        b = bits(x)
        patterns.update({b - 1, b, b + 1})
    # Subnormals: small and random mantissas.
    patterns.update(range(1, 200))
    patterns.update(rng.randrange(1, 1 << 52) for _ in range(size // 10))
    # Few-bit mantissas at every exponent: short exact decimals, so ties.
    for _ in range(size // 3):
        mantissa = rng.randrange(1, 1 << rng.randrange(1, 24))
        x = math.ldexp(mantissa, rng.randrange(-1074, 1000))
        if math.isfinite(x):
            patterns.add(bits(x) | (rng.randrange(2) << 63))
    # Full mantissas between 2**30 and 2**55: where ties at 16-17 digits cluster.
    for _ in range(size // 10):
        x = math.ldexp(rng.randrange(1 << 52, 1 << 53), rng.randrange(30, 56) - 52)
        patterns.add(bits(x) | (rng.randrange(2) << 63))
    # Short decimal literals.
    for _ in range(size // 10):
        text = f"{rng.randrange(1, 10**rng.randrange(1, 17))}e{rng.randrange(-330, 310)}"
        x = float(text)
        if math.isfinite(x):
            patterns.add(bits(x))
    # Random bit patterns fill the rest.
    while len(patterns) < size:
        patterns.add(rng.getrandbits(64))
    return sorted(patterns)


def run(args: list[str], lines: list[str]) -> list[str]:
    done = subprocess.run(args, cwd=WORKSPACE, input="".join(lines), capture_output=True, text=True,
                          encoding="utf-8")
    assert done.returncode == 0, f"{' '.join(args)} failed:\n{done.stderr}"
    return done.stdout.splitlines()


def test_float_formatting_matches_python():
    patterns = corpus(CORPUS)
    ties = sum(is_tie(from_bits(b)) for b in patterns)
    assert len(patterns) >= 20000 or CORPUS < 20000, len(patterns)
    assert ties >= MIN_TIES, f"the corpus holds only {ties} ties"

    lines = [f"{b:016x}\n" for b in patterns]
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    expected = run([str(python), "-I", str(PARITY_DIR / "tools" / "float_oracle.py")], lines)
    build = subprocess.run(["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE,
                           capture_output=True, text=True)
    assert build.returncode == 0, build.stderr
    got = run(["cargo", "--quiet", "xtask", "float-repr"], lines)

    assert len(expected) == len(got) == len(patterns)
    mismatches = [
        f"{b:016x} ({from_bits(b)!r}): Python {want!r}, Rust {have!r}"
        for b, want, have in zip(patterns, expected, got)
        if want != have
    ]
    assert not mismatches, (
        f"{len(mismatches)} of {len(patterns)} floats ({ties} ties) differ:\n" + "\n".join(mismatches[:40])
    )
