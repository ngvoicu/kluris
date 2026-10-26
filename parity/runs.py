"""Helpers for the parity tests that need more than ``harness.run_both``: a
config written by hand, per-run environment changes, an explicit stdin (a
pipe, the null device, or a pseudo-terminal for the brain picker), and the wake-up payload
comparison."""

from __future__ import annotations

import json
import os
import re
import select
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from harness import IS_WINDOWS, Result, Sandbox, _decode, normalize, oracle_kluris, remove_tree, run_v3, snapshot_tree
from harness import run_v2 as _run_v2

def write_config(sb: Sandbox, brains: list[tuple[str, str, str]]) -> None:
    """Write ``config.yml`` with ``(name, path, description)`` brains, in
    order. JSON strings are YAML double-quoted scalars."""
    lines = ["brains:"] if brains else ["brains: {}"]
    for name, path, description in brains:
        lines += [
            f"  {json.dumps(name)}:",
            f"    path: {json.dumps(path)}",
            f"    description: {json.dumps(description)}",
        ]
    sb.config.parent.mkdir(parents=True, exist_ok=True)
    sb.config.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _env(sb: Sandbox, changes: dict[str, str | None]) -> dict[str, str]:
    env = dict(sb.env)
    for key, value in changes.items():
        if value is None:
            env.pop(key, None)
        else:
            env[key] = value
    return env


def _exe(side: str) -> list[str]:
    if side == "v2":
        return [str(oracle_kluris())]
    # run_v3 fails the test with the usual message when the binary is missing.
    from harness import V3_BIN

    if V3_BIN is None or not V3_BIN.is_file():
        run_v3([], {})
    return [str(V3_BIN)]


def run_pair(
    sb: Sandbox,
    v2_args: list[str],
    v3_args: list[str],
    *,
    env: dict[str, str | None] | None = None,
    stdin: bytes | None = b"",
) -> dict[str, Result]:
    """Run each binary on its own copy of the frozen state, with stdin a pipe
    holding ``stdin`` (never a terminal), or the null device when ``stdin`` is
    ``None`` (``NUL`` on Windows), and ``env`` applied (``None``
    removes a variable)."""
    results: dict[str, Result] = {}
    for side, args in (("v2", v2_args), ("v3", v3_args)):
        sb.checkout()
        try:
            proc = subprocess.run(
                [*_exe(side), *args], env=_env(sb, env or {}), cwd=str(sb.work),
                input=stdin, stdin=subprocess.DEVNULL if stdin is None else None,
                capture_output=True, timeout=120,
            )
            what = f"{side} {' '.join(args)}"
            results[side] = Result(
                proc.returncode, _decode(proc.stdout, f"stdout of `{what}`"),
                _decode(proc.stderr, f"stderr of `{what}`"),
            )
        finally:
            sb.stash(f"{side}-{time.monotonic_ns()}")
    return results


@dataclass
class PtyResult:
    code: int
    stdout: str
    stderr: str


def run_pty(
    sb: Sandbox,
    side: str,
    args: list[str],
    *,
    lines: list[str] | None = None,
    eof: bool = False,
    sigint: bool = False,
    env: dict[str, str | None] | None = None,
) -> PtyResult:
    """Run one binary with stdin a pseudo-terminal (stdout and stderr stay
    pipes) on a fresh copy of the frozen state. ``lines`` are typed one per
    line; then ``eof`` sends Ctrl-D, or ``sigint`` waits for the prompt and
    sends SIGINT (what Ctrl-C does). POSIX only."""
    import pty

    changes = {"KLURIS_NO_PROMPT": None}
    changes.update(env or {})
    sb.checkout()
    master, slave = pty.openpty()
    try:
        proc = subprocess.Popen(
            [*_exe(side), *args], env=_env(sb, changes), cwd=str(sb.work),
            stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        os.close(slave)
        slave = -1
        out = b""
        if sigint:
            deadline = time.monotonic() + 30
            while b"Choice: " not in out and time.monotonic() < deadline:
                ready, _, _ = select.select([proc.stdout], [], [], 1)
                if ready:
                    chunk = os.read(proc.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    out += chunk
            time.sleep(0.3)
            proc.send_signal(signal.SIGINT)
        else:
            typed = b"".join(line.encode("utf-8") + b"\n" for line in lines or [])
            try:
                os.write(master, typed + (b"\x04" if eof else b""))
            except OSError:
                pass  # the process is gone and the terminal closed
        rest, err = proc.communicate(timeout=60)
        out += rest
    finally:
        if slave >= 0:
            os.close(slave)
        os.close(master)
        sb.stash(f"{side}-pty-{time.monotonic_ns()}")
    return PtyResult(proc.returncode, _decode(out, f"stdout of {side}"), _decode(err, f"stderr of {side}"))


def transcript(stdout: str) -> str:
    """The picker's part of stdout: everything up to and with the last
    ``Choice: `` prompt."""
    at = stdout.rfind("Choice: ")
    assert at >= 0, f"no picker prompt in {stdout!r}"
    return stdout[: at + len("Choice: ")]


# --- wake-up ------------------------------------------------------------------


def issue_key(issue: dict) -> str:
    return json.dumps(issue, sort_keys=True, ensure_ascii=False)


def path_keys(entry: dict) -> tuple[str, ...]:
    """The keys of a deprecation or broken-synapse entry that hold a
    brain-relative path; a ``replaced_by`` or link ``target`` is as written."""
    if entry.get("kind") == "active_links_to_deprecated":
        return ("source", "target")
    return ("file",)


def slash_paths(v2_entries: list[dict], v3_entries: list[dict]) -> None:
    """PAR-037: Kluris 3 writes the brain-relative paths of wake-up's
    ``deprecation[]`` and dream's ``deprecation[]`` and broken synapses
    ``/``-joined on every OS; v2 writes them native. v3's must hold no
    backslash; on Windows v2's are turned into the ``/`` form (in place)
    before comparing."""
    for entry in v3_entries:
        for key in path_keys(entry):
            assert "\\" not in entry[key], f"v3 path with a backslash: {entry}"
    if not IS_WINDOWS:
        return
    for entry in v2_entries:
        for key in path_keys(entry):
            entry[key] = entry[key].replace("\\", "/")


def parse_exact(raw: str, root: Path, side: str) -> dict:
    """The parsed JSON stdout, after checking it is exactly Python's
    ``json.dumps`` of its parse plus a newline."""
    text = normalize(raw, root)
    doc = json.loads(text)
    assert text == json.dumps(doc) + "\n", f"{side} raw JSON is not json.dumps of its parse: {text!r}"
    return doc


def assert_wake_up_equal(
    v2: Result, v3: Result, root: Path, *, skipped: list[str] | None = None,
    v3_deprecation: list[dict] | None = None,
) -> dict:
    """Compare two ``wake-up --json`` outputs: byte-exact apart from
    ``deprecation[]``, compared as a multiset (PAR-008). With ``skipped``
    (the markdown neurons Kluris 3 cannot read), v2's whole-brain
    ``deprecation: []`` and ``parse_errors: 0`` stand against v3's issues of
    the other neurons (PAR-007, ``v3_deprecation`` when given) and its count
    of the skipped (PAR-009)."""
    assert v2.code == 0, f"v2 failed: {v2.stdout} {v2.stderr}"
    assert v3.code == v2.code, f"v3 exit {v3.code}: {v3.stdout} {v3.stderr}"
    j2, j3 = parse_exact(v2.stdout, root, "v2"), parse_exact(v3.stdout, root, "v3")
    slash_paths(j2["deprecation"], j3["deprecation"])
    if skipped:
        assert j2["parse_errors"] == 0
        assert j3["parse_errors"] == len(skipped), j3["parse_errors"]
        assert j2["deprecation"] == [] and j2["deprecation_count"] == 0, "v2 no longer raises"
        assert j3["deprecation_count"] == len(j3["deprecation"])
        if v3_deprecation is not None:
            # v2's rules give v2's native paths (PAR-037).
            expected = [dict(entry) for entry in v3_deprecation]
            slash_paths(expected, [])
            assert sorted(map(issue_key, j3["deprecation"])) == sorted(map(issue_key, expected))
        for doc in (j2, j3):
            doc["deprecation"], doc["deprecation_count"], doc["parse_errors"] = [], 0, 0
    else:
        assert sorted(map(issue_key, j3["deprecation"])) == sorted(map(issue_key, j2["deprecation"])), (
            f"deprecation differs as a multiset:\n  v3 {j3['deprecation']}\n  v2 {j2['deprecation']}"
        )
        j3["deprecation"] = sorted(j3["deprecation"], key=issue_key)
        j2["deprecation"] = sorted(j2["deprecation"], key=issue_key)
    assert j3 == j2, "parsed wake-up JSON differs (v3, v2):\n" + "\n".join(
        f"  {key}: {json.dumps(j3.get(key))[:2000]}\n  {' ' * len(key)}  {json.dumps(j2.get(key))[:2000]}"
        for key in dict.fromkeys([*j2, *j3]) if j2.get(key) != j3.get(key)
    )
    assert json.dumps(j3) == json.dumps(j2), "wake-up JSON differs in key order (left v3, right v2)"
    return j2


def clear_runs(sb: Sandbox) -> None:
    """Drop the finished trees of ``harness.run_both`` so it can run again."""
    for side in ("v2", "v3"):
        remove_tree(sb.root / side)


def same_words(v2_text: str, v3_text: str, root: Path) -> None:
    """Human output carries v2's wording: equal once every whitespace run is
    removed (Rich wraps lines, even mid-word, and its blank lines differ)."""
    # Squash first: Rich may break the temp root itself across lines.
    squash = lambda text: normalize(re.sub(r"\s+", "", text), root)  # noqa: E731
    assert squash(v3_text) == squash(v2_text), f"wording differs:\nv3: {v3_text!r}\nv2: {v2_text!r}"


__all__ = [
    "IS_WINDOWS", "PtyResult", "assert_wake_up_equal", "clear_runs", "issue_key", "parse_exact", "slash_paths",
    "run_pair", "run_pty", "same_words", "transcript", "write_config",
]


# --- fresh reads (PAR-073) ----------------------------------------------------


def git(sb: Sandbox, cwd: Path, *args: str) -> str:
    out = subprocess.run(["git", *args], cwd=cwd, env=sb.env, capture_output=True, text=True, encoding="utf-8")
    assert out.returncode == 0, f"git {args}: {out.stderr}"
    return out.stdout.strip()


def _write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")


def team_brain(sb: Sandbox, *, teammate_pushed: bool) -> Path:
    """A bare remote under ``work/``, a teammate's clone, and the clone that
    is registered as the brain ``team`` (its branch tracks the remote's
    ``main``). With ``teammate_pushed`` the teammate pushes ``k/zebra.md``
    after the brain was cloned, so only a fresh read sees it. Not frozen."""
    remote, teammate, brain = sb.work / "remote.git", sb.work / "teammate", sb.work / "brains" / "team"
    git(sb, sb.work, "init", "-q", "--bare", "-b", "main", str(remote))
    git(sb, sb.work, "clone", "-q", str(remote), str(teammate))
    _write(teammate, "brain.md", "# team\n\n## Lobes\n\n- [k](k/map.md)\n")
    _write(teammate, "glossary.md", "# Glossary\n\n**JWT** - JSON Web Token\n")
    _write(teammate, "k/map.md", "# k\n")
    _write(teammate, "k/jwt.md", "---\nparent: map.md\nupdated: 2026-02-03\n---\n# JWT tokens\n\nWe sign every JWT.\n")
    git(sb, teammate, "add", "-A")
    git(sb, teammate, "commit", "-q", "-m", "seed")
    git(sb, teammate, "push", "-q", "-u", "origin", "main")
    brain.parent.mkdir(parents=True, exist_ok=True)
    git(sb, sb.work, "clone", "-q", str(remote), str(brain))
    if teammate_pushed:
        _write(teammate, "k/zebra.md", "---\nparent: map.md\nupdated: 2026-03-01\n---\n# Zebra\n\nAll about zebras.\n")
        git(sb, teammate, "add", "-A")
        git(sb, teammate, "commit", "-q", "-m", "zebra")
        git(sb, teammate, "push", "-q")
    write_config(sb, [("team", str(brain), "")])
    return brain


def run_v2_after_pull(sb: Sandbox, args: list[str]) -> Result:
    """v2 on a copy of the frozen state whose brain was first brought up to
    date by hand (``git pull --ff-only``): what a v3 fresh read must show."""
    sb.checkout()
    try:
        git(sb, sb.work / "brains" / "team", "pull", "-q", "--ff-only")
        result = _run_v2(args, sb.env)
        result.tree = snapshot_tree(sb.work)
    finally:
        sb.stash("v2-pulled")
    return result
