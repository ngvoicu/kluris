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
from collections.abc import Callable
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

_VERSION = r"\d+\.\d+\.\d+(?:[-+.][0-9A-Za-z.+-]*)?"
_VERSION_RE = re.compile(_VERSION)
_VERSION_OUTPUT_RE = re.compile(rf"kluris {_VERSION}\n")


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
            remove_tree(venv)
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


# --- filesystem moves and removals -------------------------------------------
#
# Windows holds short-lived locks on a just-exited process's tree (a lingering
# handle, Defender, the search indexer), so a rename, copy or rmtree right
# after a run can fail with PermissionError (WinError 5) and go through a
# moment later. Every sandbox directory goes through these helpers: retry on
# OSError with growing pauses (10 tries, ~5 s in all), and a move that still
# fails falls back to copy + remove.

_LOCK_TRIES = 10


def _retry_locked(fn: Callable[[], None], what: str) -> None:
    """Run ``fn`` until it stops raising ``OSError``; raise the last failure,
    with ``what`` naming the path, when it never does."""
    error: OSError | None = None
    for attempt in range(_LOCK_TRIES):
        try:
            fn()
            return
        except OSError as e:
            error = e
            if attempt + 1 < _LOCK_TRIES:
                time.sleep(min(0.05 * 2**attempt, 0.7))
    assert error is not None
    if error.errno is None:
        raise OSError(f"{what}: {error}") from error
    raise OSError(error.errno, f"{what}: {error.strerror or error}") from error


def _fix_readonly(fn, path: str, _exc) -> None:
    """rmtree's onerror/onexc: a read-only entry (a git object, say) cannot be
    removed on Windows; make it writable and try that one entry again."""
    os.chmod(path, 0o700)
    fn(path)


def _rmtree(path: Path) -> None:
    if sys.version_info >= (3, 12):
        shutil.rmtree(path, onexc=_fix_readonly)
    else:
        shutil.rmtree(path, onerror=_fix_readonly)


def remove_tree(path: Path) -> None:
    """Remove the tree ``path`` (a file or symlink is unlinked), riding out
    the short-lived locks Windows holds on a just-exited process's tree. A
    path that is not there is fine."""

    def rm() -> None:
        if path.is_dir() and not path.is_symlink():
            _rmtree(path)
        elif path.is_symlink() or path.exists():
            path.unlink()

    _retry_locked(rm, f"could not remove {path}")


def copy_tree(src: Path, dst: Path) -> None:
    """Copy the tree (or file) ``src`` to a fresh ``dst``, with the same
    retries. A partial copy a failed try left behind is removed before the
    next; a ``dst`` that was already there is never touched."""
    fresh = not (dst.exists() or dst.is_symlink())

    def cp() -> None:
        if fresh and dst.exists():
            remove_tree(dst)  # a previous try's partial copy
        if src.is_dir() and not src.is_symlink():
            shutil.copytree(src, dst, symlinks=True)
        else:
            shutil.copy2(src, dst)

    _retry_locked(cp, f"could not copy {src} to {dst}")


def move_tree(src: Path, dst: Path) -> None:
    """Move ``src`` to ``dst`` as one rename, with the same retries; a lock
    that outlasts them falls back to copy + remove, each with its own."""
    try:
        _retry_locked(lambda: os.rename(src, dst), f"could not rename {src} to {dst}")
        return
    except OSError:
        pass  # a stubborn lock: copy the tree over, then remove the old one
    copy_tree(src, dst)
    remove_tree(src)


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
        move_tree(self.work, self._pristine)

    def checkout(self) -> None:
        """Copy the frozen state to ``work/`` (freezing the current ``work/``
        first when nothing was frozen yet)."""
        if not self._pristine.exists():
            self.freeze()
        if self.work.exists():
            raise RuntimeError("work/ is in use; stash() the previous run first")
        copy_tree(self._pristine, self.work)

    def stash(self, label: str) -> Path:
        """Move the finished ``work/`` to ``<root>/<label>`` and return it."""
        dest = self.root / label
        move_tree(self.work, dest)
        return dest


def _decode(stream: bytes, name: str = "output") -> str:
    """A stream as text, with CRLF read as LF: Python's text streams write CRLF
    on Windows and Rust's ``println!`` writes LF (allowlist PAR-004).

    Decoding is strict: invalid UTF-8 fails the test and names the bytes, so
    two different invalid streams can never compare equal."""
    try:
        text = stream.decode("utf-8")
    except UnicodeDecodeError as e:
        pytest.fail(
            f"{name} is not valid UTF-8: {e.reason} at byte {e.start}, "
            f"bytes {stream[e.start:e.end]!r} in {stream[max(0, e.start - 20):e.end + 20]!r}"
        )
    return text.replace("\r\n", "\n")


def _run(exe: Path, args: list[str], env: dict[str, str], cwd: Path | None) -> Result:
    proc = subprocess.run(
        [str(exe), *args], env=env, cwd=str(cwd) if cwd else None, capture_output=True,
    )
    what = f"{exe.name} {' '.join(args)}".strip()
    return Result(
        proc.returncode,
        _decode(proc.stdout, f"stdout of `{what}`"),
        _decode(proc.stderr, f"stderr of `{what}`"),
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
    ``<ROOT>``. Version text is left alone: it may be user data."""
    for form in _root_forms(root):
        text = text.replace(form, "<ROOT>")
    return text


def normalize_version_output(text: str) -> str:
    """``kluris <version>\\n`` (the whole of a ``--version`` stdout) becomes
    ``kluris <VERSION>\\n``; any other text is returned unchanged."""
    if _VERSION_OUTPUT_RE.fullmatch(text):
        return "kluris <VERSION>\n"
    return text


V2_BRAIN_ALL_REFUSED = "--brain all is only supported on dream, status, mri, and companion commands."
V3_BRAIN_ALL_REFUSED = "--brain all is only supported on dream, status, mri, sync, and companion commands."


def par084(v2_output: str) -> str:
    """v2's output with the ``--brain all`` refusal worded as v3 words it
    (PAR-084: ``sync`` takes ``--brain all`` too); any other text unchanged."""
    return v2_output.replace(V2_BRAIN_ALL_REFUSED, V3_BRAIN_ALL_REFUSED)


JsonPath = tuple[str | int, ...]


def _mark_versions(doc: object, paths: list[JsonPath], side: str) -> None:
    """Replace the version string at each of ``paths`` in ``doc`` with
    ``<VERSION>``; a missing path or a value that is not a version is a
    difference (AssertionError)."""
    for path in paths:
        parent = doc
        try:
            for key in path[:-1]:
                parent = parent[key]  # type: ignore[index]
            value = parent[path[-1]]  # type: ignore[index]
        except (KeyError, IndexError, TypeError):
            raise AssertionError(f"{side} output has no version field at {path!r}") from None
        assert isinstance(value, str), f"{side} version field {path!r} is not a string: {value!r}"
        replaced, count = _VERSION_RE.subn("<VERSION>", value)
        assert count == 1 and replaced in ("<VERSION>", "kluris <VERSION>"), (
            f"{side} version field {path!r} does not hold a version: {value!r}"
        )
        parent[path[-1]] = replaced  # type: ignore[index]


def assert_json_equal(
    a: str, b: str, root: Path, *, version_fields: list[JsonPath] = (),
) -> None:
    """Assert two JSON stdouts are equal: parsed, then raw bytes, both after
    root normalization (``a`` is v2, ``b`` is v3).

    ``version_fields`` lists the JSON paths (keys and list indexes) where the
    application prints its own version; only those values are normalized.
    With such fields each side's bytes must be exactly Python's ``json.dumps``
    of its own parse (plus its final newline, if any); the marked values are
    then replaced with ``<VERSION>`` and the two re-serialized byte strings,
    final newline included, are compared. Key order, ``true`` vs ``1``,
    ``1`` vs ``1.0`` and ``-0.0`` vs ``0.0`` all still differ."""
    na, nb = normalize(a, root), normalize(b, root)
    try:
        ja = json.loads(na)
    except json.JSONDecodeError as e:
        pytest.fail(f"v2 output is not JSON ({e}): {a!r}")
    try:
        jb = json.loads(nb)
    except json.JSONDecodeError as e:
        pytest.fail(f"v3 output is not JSON ({e}): {b!r}")
    if version_fields:
        marked = []
        for side, text, doc in (("v2", na, ja), ("v3", nb, jb)):
            newline = "\n" * text.endswith("\n")
            assert text == json.dumps(doc) + newline, (
                f"{side} raw JSON is not json.dumps of its parse: {text!r}"
            )
            _mark_versions(doc, list(version_fields), side)
            marked.append(json.dumps(doc) + newline)
        na, nb = marked
    assert jb == ja, "parsed JSON differs (left v3, right v2)"
    assert nb == na, "raw JSON bytes differ after normalization (left v3, right v2)"
