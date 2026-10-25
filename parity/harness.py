"""Parity harness: run the frozen Kluris v2.28.11 oracle and the Kluris 3 binary
side by side in identical, isolated environments and compare what they do.

The oracle is a venv built from ``oracle.lock`` (exact versions, sha256 hashes)
once per session and reused while the lock is unchanged. The v3 binary comes
from ``KLURIS_V3_BIN``; a missing binary fails the test, it never skips.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import pytest

PARITY_DIR = Path(__file__).resolve().parent
ORACLE_LOCK = PARITY_DIR / "oracle.lock"
ORACLE_PYTHON = "3.11"
ORACLE_DIR = Path(os.environ.get("KLURIS_PARITY_ORACLE_DIR") or PARITY_DIR / ".oracle").resolve()

# Resolved once, against the directory pytest was started from.
_V3_BIN_RAW = os.environ.get("KLURIS_V3_BIN")
V3_BIN = Path(_V3_BIN_RAW).resolve() if _V3_BIN_RAW else None

PINNED_GIT_DATE = "2026-01-01T12:00:00+00:00"
IS_WINDOWS = sys.platform == "win32"

# Filled by oracle_kluris(); reported by conftest's terminal summary.
ORACLE_STATS: dict[str, object] = {}

_VERSION_RE = re.compile(r"kluris \d+\.\d+\.\d+(?:[-+.][0-9A-Za-z.+-]*)?")


@dataclass
class Result:
    code: int
    stdout: str
    stderr: str
    tree: dict[str, bytes] | None = field(default=None, repr=False)


# --- the oracle ------------------------------------------------------------


def _venv_exe(venv: Path, name: str) -> Path:
    if IS_WINDOWS:
        return venv / "Scripts" / f"{name}.exe"
    return venv / "bin" / name


def _uv_env() -> dict[str, str]:
    """uv's environment for building the oracle: the real one, with an explicit
    cache dir that can never land inside a test HOME."""
    env = dict(os.environ)
    if not env.get("UV_CACHE_DIR"):
        out = subprocess.run(["uv", "cache", "dir"], capture_output=True, text=True, check=True)
        env["UV_CACHE_DIR"] = out.stdout.strip()
    env.pop("VIRTUAL_ENV", None)
    return env


def _lock_digest() -> str:
    return hashlib.sha256(ORACLE_LOCK.read_bytes()).hexdigest()


_ORACLE_BIN: Path | None = None


def oracle_kluris() -> Path:
    """Return the oracle's ``kluris`` executable, building the venv if the lock
    changed since the last build (or it was never built)."""
    global _ORACLE_BIN
    if _ORACLE_BIN is not None:
        return _ORACLE_BIN

    venv = ORACLE_DIR / "venv"
    marker = ORACLE_DIR / "oracle.lock.sha256"
    exe = _venv_exe(venv, "kluris")
    python = _venv_exe(venv, "python")
    digest = _lock_digest()

    reused = marker.is_file() and marker.read_text(encoding="utf-8").strip() == digest and exe.is_file()
    started = time.monotonic()
    if not reused:
        env = _uv_env()
        ORACLE_DIR.mkdir(parents=True, exist_ok=True)
        if marker.exists():
            marker.unlink()
        if venv.exists():
            shutil.rmtree(venv)
        subprocess.run(
            ["uv", "venv", "--quiet", "--no-project", "--python", ORACLE_PYTHON, str(venv)],
            env=env, check=True,
        )
        subprocess.run(
            ["uv", "pip", "sync", "--quiet", "--require-hashes", "--python", str(python), str(ORACLE_LOCK)],
            env=env, check=True,
        )
        marker.write_text(digest + "\n", encoding="utf-8")
    ORACLE_STATS.update(reused=reused, seconds=time.monotonic() - started, venv=str(venv))

    check = subprocess.run(
        [str(python), "-c", "import sys, yaml; print(sys.version_info[:2] == (3, 11) and yaml.__with_libyaml__)"],
        capture_output=True, text=True,
    )
    if check.returncode != 0 or check.stdout.strip() != "True":
        pytest.fail(
            "oracle venv is not Python 3.11 with libyaml-backed PyYAML "
            f"(yaml.__with_libyaml__ check printed {check.stdout.strip()!r}, stderr {check.stderr.strip()!r})"
        )
    if not exe.is_file():
        pytest.fail(f"oracle venv has no kluris executable at {exe}")
    _ORACLE_BIN = exe
    return exe


# --- the isolated environment ----------------------------------------------


# Inherited only so the binaries and git can start; everything else is pinned.
_PASSTHROUGH = ("PATH", "SYSTEMROOT", "SYSTEMDRIVE", "COMSPEC", "PATHEXT", "WINDIR")


class Sandbox:
    """A fresh temp root for one test.

    Everything a run touches lives under ``work/`` (HOME, config, brains), so
    the absolute paths v2 stores in config.yml stay valid when the same state
    is replayed for the second binary: ``freeze()`` moves the prepared ``work/``
    aside, and each binary then runs on its own identical copy, checked out at
    the same absolute path.
    """

    def __init__(self, root: Path):
        self.root = root.resolve()
        self.work = self.root / "work"
        self.home = self.work / "home"
        self.config = self.home / ".kluris" / "config.yml"
        self._pristine = self.root / "pristine"
        (self.root / "tmp").mkdir(parents=True, exist_ok=True)
        self._gitconfig = self.root / "gitconfig-empty"
        self._gitconfig.write_bytes(b"")
        self.home.mkdir(parents=True)
        self.env = self._make_env()

    def _make_env(self) -> dict[str, str]:
        env = {k: os.environ[k] for k in _PASSTHROUGH if k in os.environ}
        tmp = str(self.root / "tmp")
        env.update(
            HOME=str(self.home),
            USERPROFILE=str(self.home),
            KLURIS_CONFIG=str(self.config),
            KLURIS_NO_PROMPT="1",
            GIT_CONFIG_GLOBAL=str(self._gitconfig),
            GIT_CONFIG_NOSYSTEM="1",
            GIT_AUTHOR_NAME="Kluris Parity",
            GIT_AUTHOR_EMAIL="parity@kluris.invalid",
            GIT_COMMITTER_NAME="Kluris Parity",
            GIT_COMMITTER_EMAIL="parity@kluris.invalid",
            GIT_AUTHOR_DATE=PINNED_GIT_DATE,
            GIT_COMMITTER_DATE=PINNED_GIT_DATE,
            TZ="UTC",
            LC_ALL="C.UTF-8",
            PYTHONUTF8="1",
            PYTHONIOENCODING="utf-8",
            NO_COLOR="1",
            TERM="dumb",
            COLUMNS="120",
            TMPDIR=tmp,
            TEMP=tmp,
            TMP=tmp,
            UV_CACHE_DIR=str(self.root / "uv-cache"),
        )
        return env

    def freeze(self) -> None:
        """Keep the current ``work/`` as the state every binary starts from."""
        if self._pristine.exists():
            raise RuntimeError("sandbox state is already frozen")
        self.work.rename(self._pristine)

    def checkout(self) -> None:
        """Copy the frozen state to ``work/`` (freezing the current ``work/``
        first when nothing was frozen yet)."""
        if not self._pristine.exists():
            self.freeze()
        if self.work.exists():
            raise RuntimeError("work/ is in use; stash() the previous run first")
        shutil.copytree(self._pristine, self.work, symlinks=True)

    def stash(self, label: str) -> Path:
        """Move the finished ``work/`` to ``<root>/<label>`` and return it."""
        dest = self.root / label
        self.work.rename(dest)
        return dest


def _run(exe: Path, args: list[str], env: dict[str, str], cwd: Path | None) -> Result:
    proc = subprocess.run(
        [str(exe), *args], env=env, cwd=str(cwd) if cwd else None, capture_output=True,
    )
    return Result(
        proc.returncode,
        proc.stdout.decode("utf-8", errors="replace"),
        proc.stderr.decode("utf-8", errors="replace"),
    )


def run_v2(args: list[str], env: dict[str, str], cwd: Path | None = None) -> Result:
    return _run(oracle_kluris(), args, env, cwd)


def run_v3(args: list[str], env: dict[str, str], cwd: Path | None = None) -> Result:
    if V3_BIN is None:
        pytest.fail(
            "KLURIS_V3_BIN is not set. Build Kluris 3 (`cargo build --release`) and run "
            "`KLURIS_V3_BIN=target/release/kluris uv run --project parity pytest parity -q`."
        )
    if not V3_BIN.is_file():
        pytest.fail(f"KLURIS_V3_BIN points at {V3_BIN}, which does not exist. Run `cargo build --release`.")
    return _run(V3_BIN, args, env, cwd)


def run_both(sb: Sandbox, args: list[str], cwd_rel: str | None = None) -> dict[str, Result]:
    """Run ``args`` with each binary on its own copy of the frozen state.

    Each result carries a snapshot of ``work/`` after the run; the finished
    trees are kept at ``<root>/v2`` and ``<root>/v3``.
    """
    results: dict[str, Result] = {}
    for side, runner in (("v2", run_v2), ("v3", run_v3)):
        sb.checkout()
        try:
            result = runner(args, sb.env, sb.work / cwd_rel if cwd_rel else None)
            result.tree = snapshot_tree(sb.work)
        finally:
            sb.stash(side)
        results[side] = result
    return results


# --- comparison helpers ------------------------------------------------------


def snapshot_tree(path: Path) -> dict[str, bytes]:
    """{posix relative path: bytes} of every file under ``path``, ``.git/`` excluded."""
    out: dict[str, bytes] = {}
    for p in sorted(path.rglob("*")):
        rel = p.relative_to(path)
        if ".git" in rel.parts:
            continue
        if p.is_file() and not p.is_symlink():
            out[rel.as_posix()] = p.read_bytes()
        elif p.is_symlink():
            out[rel.as_posix()] = b"<symlink> " + os.readlink(p).encode("utf-8")
    return out


def _root_forms(root: Path) -> list[str]:
    forms: set[str] = set()
    for r in {str(root), str(root.resolve())}:
        p = Path(r)
        for s in (r, p.as_posix()):
            forms.add(s)
            forms.add(json.dumps(s)[1:-1])  # JSON-escaped (backslashes, non-ASCII)
    # Longest first, so a resolved form is not half-replaced by a shorter alias.
    return sorted((f for f in forms if f), key=len, reverse=True)


def normalize(text: str, root: Path) -> str:
    """Replace the temp root (raw, JSON-escaped and ``C:/`` forms) with
    ``<ROOT>`` and any ``kluris <version>`` with ``kluris <VERSION>``."""
    for form in _root_forms(root):
        text = text.replace(form, "<ROOT>")
    return _VERSION_RE.sub("kluris <VERSION>", text)


def assert_json_equal(a: str, b: str, root: Path) -> None:
    """Assert two JSON stdouts are equal: parsed, then raw bytes, both after
    normalization (``a`` is v2, ``b`` is v3)."""
    na, nb = normalize(a, root), normalize(b, root)
    try:
        ja = json.loads(na)
    except json.JSONDecodeError as e:
        pytest.fail(f"v2 output is not JSON ({e}): {a!r}")
    try:
        jb = json.loads(nb)
    except json.JSONDecodeError as e:
        pytest.fail(f"v3 output is not JSON ({e}): {b!r}")
    assert jb == ja, "parsed JSON differs (left v3, right v2)"
    assert nb == na, "raw JSON bytes differ after normalization (left v3, right v2)"
