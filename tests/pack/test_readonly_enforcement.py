"""TEST-PACK-38 — read-only enforcement on tool dispatchers + runtime.

Greps :mod:`kluris.pack.tools` and :mod:`kluris_runtime` for filesystem
write APIs. Zero hits required.
"""

from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
TOOLS_ROOT = REPO_ROOT / "src" / "kluris" / "pack" / "tools"
RUNTIME_ROOT = REPO_ROOT / "src" / "kluris_runtime"

_PATTERNS = [
    re.compile(r'open\([^)]*["\'][awx]'),
    re.compile(r"\.write_text\("),
    re.compile(r"\.write_bytes\("),
    re.compile(r"\.touch\("),
    re.compile(r"\.unlink\("),
    re.compile(r"\.rmdir\("),
    re.compile(r"\.mkdir\("),
    re.compile(r"os\.unlink\("),
    re.compile(r"os\.makedirs\("),
    re.compile(r"shutil\.rmtree\("),
    re.compile(r"shutil\.copy\("),
    re.compile(r"shutil\.move\("),
]


def _scan(root: Path) -> list[tuple[Path, int, str]]:
    offenders: list[tuple[Path, int, str]] = []
    for py in root.rglob("*.py"):
        for lineno, raw in enumerate(
            py.read_text(encoding="utf-8").splitlines(), start=1
        ):
            stripped = raw.strip()
            if stripped.startswith("#"):
                continue
            for pat in _PATTERNS:
                if pat.search(raw):
                    offenders.append((py, lineno, stripped))
                    break
    return offenders


def test_tools_have_no_write_apis():
    offenders = _scan(TOOLS_ROOT)
    assert not offenders, (
        "kluris.pack.tools must contain zero write-API calls: "
        + ", ".join(f"{p}:{ln}: {line}" for p, ln, line in offenders)
    )


def test_runtime_has_no_write_apis():
    offenders = _scan(RUNTIME_ROOT)
    assert not offenders, (
        "kluris_runtime must contain zero write-API calls: "
        + ", ".join(f"{p}:{ln}: {line}" for p, ln, line in offenders)
    )
