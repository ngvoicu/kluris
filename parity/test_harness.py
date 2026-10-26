"""The harness's own behaviour, independent of the two binaries."""

from __future__ import annotations

import os
import shutil
import sys
import time
from pathlib import Path

import pytest

import harness
import runs
from harness import _run


def test_run_turns_crlf_into_lf(tmp_path: Path):
    # Python's text streams write CRLF on Windows, Rust's println! writes LF:
    # both sides are compared with LF (allowlist PAR-004).
    script = tmp_path / "say.py"
    script.write_text(
        "import sys\n"
        "sys.stdout.buffer.write(b'a\\r\\nb\\r\\nlone\\rcr\\n')\n"
        "sys.stderr.buffer.write(b'e\\r\\n')\n",
        encoding="utf-8",
    )
    r = _run(Path(sys.executable), [str(script)], dict(os.environ), None)
    assert r.code == 0, r.stderr
    assert r.stdout == "a\nb\nlone\rcr\n"
    assert r.stderr == "e\n"


# --- comparison helpers must not hide real differences (review T-54, finding 7)

ROOT = Path("/nonexistent/parity-root")


def _assert_differs(a: str, b: str, **kwargs) -> AssertionError:
    with pytest.raises(AssertionError) as info:
        harness.assert_json_equal(a, b, ROOT, **kwargs)
    return info.value


def test_version_text_in_user_data_is_not_normalized():
    v2 = '{"ok": true, "description": "guide to kluris 2.28.11"}'
    v3 = '{"ok": true, "description": "guide to kluris 9.9.9"}'
    _assert_differs(v2, v3)


def test_normalize_leaves_version_text_alone():
    text = "guide to kluris 2.28.11 at /nonexistent/parity-root/x"
    assert harness.normalize(text, ROOT) == "guide to kluris 2.28.11 at <ROOT>/x"


def test_version_output_is_normalized_only_as_a_whole():
    nv = harness.normalize_version_output
    assert nv("kluris 2.28.11\n") == "kluris <VERSION>\n"
    assert nv("kluris 3.0.0-dev\n") == "kluris <VERSION>\n"
    assert nv("guide to kluris 2.28.11\n") == "guide to kluris 2.28.11\n"
    assert nv("kluris 2.28.11\nkluris 2.28.11\n") == "kluris 2.28.11\nkluris 2.28.11\n"


def test_marked_version_field_is_normalized():
    v2 = '{"ok": true, "version": "2.28.11", "brains": [{"v": "kluris 2.28.11"}]}'
    v3 = '{"ok": true, "version": "3.0.0-dev", "brains": [{"v": "kluris 3.0.0-dev"}]}'
    harness.assert_json_equal(v2, v3, ROOT, version_fields=[("version",), ("brains", 0, "v")])


def test_marked_version_field_does_not_hide_other_fields():
    v2 = '{"version": "2.28.11", "description": "kluris 2.28.11"}'
    v3 = '{"version": "3.0.0-dev", "description": "kluris 3.0.0-dev"}'
    _assert_differs(v2, v3, version_fields=[("version",)])


def test_marked_version_field_keeps_the_raw_byte_check():
    v2 = '{"version": "2.28.11", "ok": true}'
    v3 = '{"version":"3.0.0-dev", "ok": true}'
    _assert_differs(v2, v3, version_fields=[("version",)])


def test_marked_version_field_must_hold_a_version():
    v2 = '{"version": "2.28.11"}'
    v3 = '{"version": "not a version"}'
    _assert_differs(v2, v3, version_fields=[("version",)])


def test_raw_bytes_are_compared():
    _assert_differs('{"a": 1}', '{"a":1}')
    _assert_differs('{"a": "\\u00e9"}', '{"a": "\u00e9"}')


@pytest.mark.parametrize("stream", [b'{"a": "\xff"}\n', b'{"a": "\xfe"}\n', b"ok\n\xc3"])
def test_invalid_utf8_is_a_failure_naming_the_bytes(stream: bytes):
    with pytest.raises(pytest.fail.Exception) as info:
        harness._decode(stream)
    bad = stream[next(i for i in range(len(stream)) if stream[i] >= 0x80) :]
    assert repr(bad[:1])[2:-1] in str(info.value), str(info.value)


def test_run_fails_on_invalid_utf8_output(tmp_path: Path):
    script = tmp_path / "bad.py"
    script.write_text("import sys\nsys.stdout.buffer.write(b'{\"a\": \"\\xff\"}\\n')\n", encoding="utf-8")
    with pytest.raises(pytest.fail.Exception) as info:
        _run(Path(sys.executable), [str(script)], dict(os.environ), None)
    assert "\\xff" in str(info.value) and "stdout" in str(info.value)


# --- with marked version fields, everything else is still compared byte for
# byte (re-review T-62, finding 1)


@pytest.mark.parametrize(
    ("v2", "v3"),
    [
        ('{"version": "2.28.11", "n": 1}\n', '{"version": "3.0.0", "n": 1}'),
        ('{"version": "2.28.11", "a": 1, "b": 2}\n', '{"version": "3.0.0", "b": 2, "a": 1}\n'),
        ('{"version": "2.28.11", "n": true}\n', '{"version": "3.0.0", "n": 1}\n'),
        ('{"version": "2.28.11", "n": 1}\n', '{"version": "3.0.0", "n": 1.0}\n'),
        ('{"version": "2.28.11", "n": -0.0}\n', '{"version": "3.0.0", "n": 0.0}\n'),
    ],
    ids=["final-newline", "key-order", "true-vs-1", "1-vs-1.0", "-0.0-vs-0.0"],
)
def test_marked_version_field_hides_nothing_else(v2: str, v3: str):
    _assert_differs(v2, v3, version_fields=[("version",)])


# --- sandbox moves and removals ride out the short-lived locks Windows holds
# on a just-exited process's tree (T-86 and T-90: WinError 5 on the rename in
# Sandbox.stash right after the v2 run; the same tree passed on the rerun)


def _no_sleeps(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(time, "sleep", lambda _seconds: None)


def _flaky_rename(monkeypatch: pytest.MonkeyPatch, times: int) -> dict:
    """``os.rename`` raises PermissionError on its first ``times`` calls, then
    works; returns the call counter."""
    real = os.rename
    calls = {"n": 0}

    def flaky(src, dst, *args, **kwargs):
        calls["n"] += 1
        if calls["n"] <= times:
            raise PermissionError(13, "Access is denied", str(src))
        return real(src, dst, *args, **kwargs)

    monkeypatch.setattr(os, "rename", flaky)
    _no_sleeps(monkeypatch)
    return calls


def test_stash_retries_a_transiently_locked_work_dir(tmp_path: Path, monkeypatch):
    sb = harness.Sandbox(tmp_path / "sb")
    (sb.work / "marker.txt").write_text("x", encoding="utf-8")
    calls = _flaky_rename(monkeypatch, 3)
    dest = sb.stash("v2")
    assert calls["n"] == 4
    assert (dest / "marker.txt").read_text(encoding="utf-8") == "x"
    assert not sb.work.exists()


def test_freeze_retries_a_transiently_locked_work_dir(tmp_path: Path, monkeypatch):
    sb = harness.Sandbox(tmp_path / "sb")
    calls = _flaky_rename(monkeypatch, 2)
    sb.freeze()
    assert calls["n"] == 3
    assert (sb.root / "pristine" / "home").is_dir()
    assert not sb.work.exists()


def test_a_stuck_lock_falls_back_to_copy_and_remove(tmp_path: Path, monkeypatch):
    # A lock that outlasts every rename retry still gets the tree moved.
    sb = harness.Sandbox(tmp_path / "sb")
    (sb.work / "brains").mkdir()
    (sb.work / "brains" / "n.md").write_text("neuron\n", encoding="utf-8")
    calls = _flaky_rename(monkeypatch, 10**9)
    dest = sb.stash("v2")
    assert calls["n"] == harness._LOCK_TRIES
    assert (dest / "brains" / "n.md").read_text(encoding="utf-8") == "neuron\n"
    assert not sb.work.exists()


def test_a_stuck_lock_reports_the_path_when_everything_fails(tmp_path: Path, monkeypatch):
    sb = harness.Sandbox(tmp_path / "sb")
    _flaky_rename(monkeypatch, 10**9)

    def no_copy(src, dst, *args, **kwargs):
        raise PermissionError(13, "Access is denied", str(dst))

    monkeypatch.setattr(shutil, "copytree", no_copy)
    with pytest.raises(OSError) as info:
        sb.stash("v2")
    assert str(sb.root / "v2") in str(info.value)
    assert sb.work.is_dir()  # the source is still there


def test_checkout_retries_a_transiently_locked_copy(tmp_path: Path, monkeypatch):
    sb = harness.Sandbox(tmp_path / "sb")
    (sb.work / "m.txt").write_text("x", encoding="utf-8")
    sb.freeze()
    real = shutil.copytree
    calls = {"n": 0}

    def flaky(src, dst, *args, **kwargs):
        # copytree recurses into subdirectories through its module-level name;
        # count only the top-level call (DirEntry is PathLike, so test for it).
        if isinstance(src, os.DirEntry):
            return real(src, dst, *args, **kwargs)
        calls["n"] += 1
        if calls["n"] <= 2:
            Path(dst).mkdir(parents=True)  # the partial copy a locked try leaves
            (Path(dst) / "partial").write_text("x", encoding="utf-8")
            raise PermissionError(13, "Access is denied", str(dst))
        return real(src, dst, *args, **kwargs)

    monkeypatch.setattr(shutil, "copytree", flaky)
    _no_sleeps(monkeypatch)
    sb.checkout()
    assert calls["n"] == 3
    assert (sb.work / "m.txt").read_text(encoding="utf-8") == "x"
    assert not (sb.work / "partial").exists()  # the partial copy was removed first


def test_clear_runs_retries_a_transiently_locked_tree(tmp_path: Path, monkeypatch):
    sb = harness.Sandbox(tmp_path / "sb")
    for d in ("v2", "v3"):
        (sb.root / d / "inner").mkdir(parents=True)
    real = shutil.rmtree
    calls = {"n": 0}

    def flaky(path, *args, **kwargs):
        calls["n"] += 1
        if calls["n"] <= 2:
            raise PermissionError(13, "Access is denied", str(path))
        return real(path, *args, **kwargs)

    monkeypatch.setattr(shutil, "rmtree", flaky)
    _no_sleeps(monkeypatch)
    runs.clear_runs(sb)
    assert calls["n"] == 4  # two failures on v2, then v2 and v3 each removed
    assert not (sb.root / "v2").exists() and not (sb.root / "v3").exists()
