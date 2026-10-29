"""Freeze the HOMEs Kluris v2.28.11 sets up for ``test_skill_install.py``,
``test_doctor.py`` and ``test_migration.py`` into plain files for
``crates/kluris-cli/tests/golden_upgrade.rs``, and the oracle's answers to
``tools/posix_path_oracle.py`` as rows.

    uv run --project parity python parity/tools/freeze_upgrade_home.py \
        crates/kluris-cli/tests/fixtures

writes ``<out>/upgrade-home/skill-install`` (``test_skill_install._v2_home``),
``<out>/upgrade-home/migration`` (``test_migration._v2_world``), the same two
as the oracle writes them on Windows (``skill-install-crlf``,
``migration-crlf``) and ``<out>/posix-path-rows.jsonl`` (``[path, answer,
the answer's anchor]``).

Each world holds ``home/`` and, per brain, ``brains/<name>/commit-<n>/``
(the files commit n adds or changes) and ``brains/<name>/worktree/`` (the
files on disk that differ from the last commit: untracked, ignored or
changed). The sandbox's work folder is written as ``@WORK@``, and every
name that starts with ``.`` is stored as ``dot-<name>`` (``.claude`` as
``dot-claude``, ``.gitignore`` as ``dot-gitignore``), so that no agent working
in this repository finds the skills and no ``.gitignore`` applies to the
fixture.

On Windows the oracle opens text files with Python's text mode, which writes
each ``\n`` as ``\r\n``. A ``-crlf`` world is made on this machine with the
oracle's text-mode writes doing the same (``newline="\r\n"``); its
``crlf-files.txt`` lists, by stored path, the files that came out with CRLF.
"""

from __future__ import annotations

import itertools
import json
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

PARITY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PARITY))

import harness  # noqa: E402
from harness import IS_WINDOWS, Sandbox, oracle_kluris  # noqa: E402
import test_migration  # noqa: E402
import test_skill_install  # noqa: E402

PLACEHOLDER = b"@WORK@"
WORLDS = {
    "skill-install": test_skill_install._v2_home,
    "migration": test_migration._v2_world,
}


def git(repo: Path, *args: str) -> bytes:
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True).stdout


def lines(out: bytes) -> list[str]:
    return [line for line in out.decode("utf-8").split("\0") if line]


def stored(rel: str) -> str:
    """``rel`` with each part that starts with ``.`` as ``dot-<part>``."""
    parts = rel.split("/")
    assert not any(p.startswith("dot-") for p in parts), rel
    return "/".join("dot-" + p[1:] if p.startswith(".") else p for p in parts)


def copy(src: Path, dst: Path, rels: list[str], work: Path) -> None:
    for rel in rels:
        data = (src / rel).read_bytes()
        assert PLACEHOLDER not in data, rel
        target = dst / stored(rel)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data.replace(str(work).encode("utf-8"), PLACEHOLDER))


def files(root: Path) -> list[str]:
    return sorted(p.relative_to(root).as_posix() for p in root.rglob("*")
                  if p.is_file() and ".git" not in p.relative_to(root).parts)


def freeze_brain(brain: Path, out: Path, work: Path) -> list[str]:
    """The brain as commits and a worktree; returns the commit messages."""
    messages = []
    if (brain / ".git").exists():
        commits = git(brain, "rev-list", "--reverse", "HEAD").decode().split()
        for n, commit in enumerate(commits, 1):
            fields = lines(git(brain, "diff-tree", "-r", "-z", "--root", "--no-renames", "--no-commit-id",
                               "--name-status", commit))
            pairs = list(zip(fields[0::2], fields[1::2]))
            assert all(status in ("A", "M") for status, _ in pairs), pairs
            git(brain, "checkout", "-q", commit)
            copy(brain, out / f"commit-{n}", [rel for _, rel in pairs], work)
            messages.append(git(brain, "log", "-1", "--format=%s", commit).decode().strip())
        git(brain, "checkout", "-q", "main")
        tracked = set(lines(git(brain, "ls-files", "-z")))
        differ = set(lines(git(brain, "ls-files", "-z", "--others", "--modified")))
        differ |= set(lines(git(brain, "ls-files", "-z", "--others", "--ignored", "--exclude-standard")))
        assert set(files(brain)) == tracked | differ, "a file is neither committed nor in the worktree"
        copy(brain, out / "worktree", sorted(differ), work)
    else:
        copy(brain, out / "worktree", files(brain), work)
    return messages


# Runs the oracle with its text-mode writes as Windows makes them.
WINDOWS_TEXT = """
import builtins, io, sys

_open = io.open

def windows_open(file, mode="r", buffering=-1, encoding=None, errors=None, newline=None,
                 closefd=True, opener=None):
    if "b" not in mode and any(m in mode for m in "wax+") and newline is None:
        newline = "\\r\\n"
    return _open(file, mode, buffering, encoding, errors, newline, closefd, opener)

builtins.open = io.open = windows_open
from kluris.cli import cli
sys.argv[0] = "kluris"
sys.exit(cli())
"""


def windows_oracle(tmp: Path):
    """An executable that runs the oracle with Windows' text-mode writes."""
    python = oracle_kluris().parent / "python"
    script = tmp / "windows_text.py"
    script.write_text(WINDOWS_TEXT, encoding="utf-8")
    exe = tmp / "kluris-windows-text"
    exe.write_text(f'#!/bin/sh\nexec "{python}" -I "{script}" "$@"\n', encoding="utf-8")
    exe.chmod(0o755)
    return exe


def freeze_world(name: str, make, out: Path, windows_text: bool) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        original = harness.oracle_kluris
        if windows_text:
            exe = windows_oracle(Path(tmp))
            harness.oracle_kluris = lambda: exe
        try:
            sb = Sandbox(Path(tmp) / "sandbox")
            make(sb)
        finally:
            harness.oracle_kluris = original
        work = sb.work
        copy(sb.home, out / "home", files(sb.home), work)
        for brain in sorted((work / "brains").iterdir()):
            messages = freeze_brain(brain, out / "brains" / brain.name, work)
            head = git(brain, "rev-parse", "HEAD").decode().strip() if messages else None
            print(f"{name}/{brain.name}: {messages} {head}")
    crlf = sorted(rel for rel in files(out) if b"\r\n" in (out / rel).read_bytes())
    if windows_text:
        (out / "crlf-files.txt").write_text("".join(f"{rel}\n" for rel in crlf), encoding="utf-8")
        print(f"{name}: {len(crlf)} CRLF files")
    else:
        assert not crlf, crlf


# --- posix_path ---------------------------------------------------------------

PIECES = ["/", "\\", "//", "\\\\", ":", "C:", "c:", "1:", "?", "\\\\?\\", "UNC", "\\\\?\\UNC\\",
          ".", "..", "a", "brain", "Users", " ", "é", "日本", "-", "~", "server", "share"]
SINGLES = ["", ".", "..", "/", "//", "///", "////", "C:", "C:\\", "C:/", "c:a", "\\\\?\\", "\\\\?\\C:\\a",
           "\\\\?\\UNC\\server\\share\\a", "\\\\server\\share", "\\\\server\\share\\", "//server/share/a/./b/../c",
           "/a/./b/", "a//b", "./a", "a/.", "C:\\Users\\me\\brains\\acme", "/Users/me/brains/acme"]


# The anchor (drive and root) of each answer, as v2's `_posix_path` parses
# the path (`PureWindowsPath` or `PurePosixPath`), in its POSIX form.
ANCHOR = """
import json, sys
from pathlib import PurePosixPath, PureWindowsPath
for line in sys.stdin:
    p = json.loads(line)
    windows = "\\\\" in p or (len(p) >= 2 and p[1] == ":")
    anchor = PureWindowsPath(p).anchor.replace("\\\\", "/") if windows else PurePosixPath(p).anchor
    print(json.dumps(anchor))
"""


def oracle_lines(args: list[str], inputs: list[str]) -> list[str]:
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    proc = subprocess.run(
        [str(python), "-I", *args], input="".join(json.dumps(i) + "\n" for i in inputs),
        capture_output=True, text=True, encoding="utf-8", check=True,
    )
    answers = [json.loads(line) for line in proc.stdout.splitlines()]
    assert len(answers) == len(inputs)
    return answers


def posix_rows(out: Path) -> None:
    rng = random.Random(0)
    sampled = ["".join(rng.choices(PIECES, k=rng.randint(3, 10))) for _ in range(1000)]
    inputs = list(dict.fromkeys(SINGLES + PIECES + ["".join(p) for p in itertools.product(PIECES, repeat=2)] + sampled))
    answers = oracle_lines([str(PARITY / "tools" / "posix_path_oracle.py")], inputs)
    anchors = oracle_lines(["-c", ANCHOR], inputs)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        for row in zip(inputs, answers, anchors):
            f.write(json.dumps(list(row), ensure_ascii=False) + "\n")
    print(f"posix-path: {len(inputs)} rows")


def main() -> None:
    out = Path(sys.argv[1]).resolve()
    for name, make in WORLDS.items():
        for windows_text in (False, True):
            world = f"{name}-crlf" if windows_text else name
            dest = out / "upgrade-home" / world
            if dest.exists():
                shutil.rmtree(dest)
            freeze_world(world, make, dest, windows_text)
    posix_rows(out / "posix-path-rows.jsonl")


if __name__ == "__main__":
    main()
