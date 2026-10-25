"""The harness's own behaviour, independent of the two binaries."""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest

import harness
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
