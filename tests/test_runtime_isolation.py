"""Runtime isolation guard.

Asserts that ``kluris_runtime`` stays a self-contained read-only package:

- Importing any submodule does NOT pull in Click, Rich, git helpers, or
  any ``kluris.core.*`` write-side module.
- The runtime source tree contains zero absolute ``kluris.`` imports.
- The runtime source tree contains zero file-write APIs.

A regression here means the Docker stager would have to ship more than
``src/kluris_runtime/`` to make the chat server work — defeating the
"minimal read-only runtime" architectural rule.
"""

from __future__ import annotations

import importlib
import re
import subprocess
import sys
from pathlib import Path

RUNTIME_ROOT = Path(__file__).resolve().parent.parent / "src" / "kluris_runtime"
RUNTIME_MODULES = [
    "kluris_runtime",
    "kluris_runtime.frontmatter",
    "kluris_runtime.neuron_index",
    "kluris_runtime.deprecation",
    "kluris_runtime.search",
    "kluris_runtime.wake_up",
    "kluris_runtime.neuron_excerpt",
]

FORBIDDEN_IMPORT_MODULES = {
    "click",
    "rich",
    "kluris.cli",
    "kluris.core.config",
    "kluris.core.agents",
    "kluris.core.maps",
    "kluris.core.mri",
    "kluris.core.git",
    "kluris.core.companions",
    "kluris.core.brain",
}


def test_runtime_does_not_pull_in_cli_or_write_modules():
    """Importing every runtime submodule in a clean subprocess must not
    leave any of the forbidden modules importable, because that would
    mean the runtime indirectly required them.
    """
    repo_root = Path(__file__).resolve().parent.parent
    src_root = repo_root / "src"
    code = (
        "import sys\n"
        f"sys.path.insert(0, {str(src_root)!r})\n"
        "for mod in " + repr(RUNTIME_MODULES) + ":\n"
        "    __import__(mod)\n"
        "loaded = set(sys.modules)\n"
        "forbidden = " + repr(FORBIDDEN_IMPORT_MODULES) + "\n"
        "leaked = sorted(loaded & forbidden)\n"
        "if leaked:\n"
        "    raise SystemExit('LEAKED: ' + ','.join(leaked))\n"
    )
    result = subprocess.run(
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, (
        f"runtime leaked CLI/core modules: stdout={result.stdout!r} "
        f"stderr={result.stderr!r}"
    )


def test_runtime_source_has_no_absolute_kluris_imports():
    """No ``import kluris.X`` or ``from kluris.X import Y`` allowed in
    the runtime — only ``kluris_runtime`` cross-imports and stdlib /
    third-party deps.
    """
    bad: list[tuple[Path, int, str]] = []
    pattern = re.compile(r"^\s*(?:from|import)\s+kluris(?!_runtime)(?:\.|\s|$)")
    for py_file in RUNTIME_ROOT.rglob("*.py"):
        for lineno, raw_line in enumerate(
            py_file.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if pattern.match(raw_line):
                bad.append((py_file, lineno, raw_line.strip()))
    assert not bad, (
        "runtime source must not import from kluris.* (only kluris_runtime.*): "
        + ", ".join(f"{p}:{ln}: {line}" for p, ln, line in bad)
    )


_WRITE_PATTERNS = [
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


def test_runtime_has_no_write_apis():
    """The runtime must not contain any filesystem-write API call.

    The kluris brain inside the Docker image is read-only; the runtime
    code paths must reflect that at the source level too.
    """
    offenders: list[tuple[Path, int, str]] = []
    for py_file in RUNTIME_ROOT.rglob("*.py"):
        text = py_file.read_text(encoding="utf-8")
        for lineno, line in enumerate(text.splitlines(), start=1):
            stripped = line.strip()
            if stripped.startswith("#"):
                continue
            for pat in _WRITE_PATTERNS:
                if pat.search(line):
                    offenders.append((py_file, lineno, stripped))
                    break
    assert not offenders, (
        "runtime source must not contain write APIs: "
        + ", ".join(f"{p}:{ln}: {line}" for p, ln, line in offenders)
    )


def test_runtime_modules_actually_import():
    """Sanity: every advertised runtime submodule imports cleanly in
    the current test process.
    """
    for name in RUNTIME_MODULES:
        importlib.import_module(name)
