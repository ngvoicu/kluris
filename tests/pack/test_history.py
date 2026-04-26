"""TEST-PACK-39 — SQLite-backed conversation history."""

from __future__ import annotations

import os
import stat

import pytest

from kluris.pack.history import SessionStore


def _store(tmp_path):
    return SessionStore(tmp_path / "data" / "sessions.db")


def test_creates_db_with_idempotent_schema(tmp_path):
    s = _store(tmp_path)
    s2 = SessionStore(tmp_path / "data" / "sessions.db")
    s.close()
    s2.close()


def test_create_session_and_append_messages(tmp_path):
    s = _store(tmp_path)
    sid = s.new_session()
    s.append_message(sid, "user", "hello")
    s.append_message(sid, "assistant", "hi there")
    rows = s.replay(sid)
    assert [r["role"] for r in rows] == ["user", "assistant"]
    assert [r["content"] for r in rows] == ["hello", "hi there"]


def test_replay_in_chronological_order(tmp_path):
    s = _store(tmp_path)
    sid = s.new_session()
    for i in range(5):
        s.append_message(sid, "user" if i % 2 == 0 else "assistant", f"msg-{i}")
    rows = s.replay(sid)
    assert [r["content"] for r in rows] == [f"msg-{i}" for i in range(5)]


def test_new_conversation_cascade_delete(tmp_path):
    s = _store(tmp_path)
    sid = s.new_session()
    s.append_message(sid, "user", "x")
    s.delete_session(sid)
    assert s.replay(sid) == []
    assert s.session_exists(sid) is False


def test_session_exists(tmp_path):
    s = _store(tmp_path)
    sid = s.new_session()
    assert s.session_exists(sid) is True
    assert s.session_exists("nope") is False


@pytest.mark.skipif(os.name == "nt", reason="POSIX file mode only")
def test_db_file_mode_is_0600(tmp_path):
    s = _store(tmp_path)
    mode = stat.S_IMODE(s.db_path.stat().st_mode)
    assert mode == 0o600


def test_tool_metadata_persisted(tmp_path):
    s = _store(tmp_path)
    sid = s.new_session()
    s.append_message(
        sid,
        "tool",
        "search results",
        tool_calls_json='[{"name":"search","args":{"q":"x"}}]',
        tool_use_id="tu-123",
    )
    rows = s.replay(sid)
    assert rows[0]["tool_calls_json"].startswith("[")
    assert rows[0]["tool_use_id"] == "tu-123"
