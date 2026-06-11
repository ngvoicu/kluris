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


def test_list_sessions_excludes_empty_sessions(tmp_path):
    """``list_sessions`` is the picker source — it must omit sessions with
    no messages (created on every page load) and keep only real ones."""
    s = _store(tmp_path)
    empty = s.new_session()
    full = s.new_session()
    s.append_message(full, "user", "hi")
    ids = {row["id"] for row in s.list_sessions()}
    assert full in ids
    assert empty not in ids


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


# --- retention (2.28.0) --------------------------------------------------------


def test_prune_old_sessions_removes_expired_and_keeps_recent(tmp_path):
    import sqlite3 as _sqlite3
    import time as _time

    from kluris.pack.history import SessionStore

    store = SessionStore(tmp_path / "sessions.db")
    old_sid = store.new_session()
    store.append_message(old_sid, "user", "ancient question")
    new_sid = store.new_session()
    store.append_message(new_sid, "user", "fresh question")
    # Age the first session 100 days into the past, directly in the DB.
    cutoff = int(_time.time()) - 100 * 86400
    con = _sqlite3.connect(tmp_path / "sessions.db")
    con.execute("UPDATE sessions SET created_at = ? WHERE id = ?", (cutoff, old_sid))
    con.commit()
    con.close()

    pruned = store.prune_old_sessions(90)
    assert pruned == 1
    assert not store.session_exists(old_sid)
    assert store.session_exists(new_sid)
    assert store.replay(old_sid) == []          # messages cascaded
    assert len(store.replay(new_sid)) == 1


def test_prune_old_sessions_zero_is_noop(tmp_path):
    from kluris.pack.history import SessionStore

    store = SessionStore(tmp_path / "sessions.db")
    sid = store.new_session()
    store.append_message(sid, "user", "keep me")
    assert store.prune_old_sessions(0) == 0
    assert store.session_exists(sid)


def test_sessions_created_at_index_exists(tmp_path):
    """The retention sweep and the list ordering both scan by created_at —
    the index keeps them O(log n) as history accumulates."""
    import sqlite3 as _sqlite3

    from kluris.pack.history import SessionStore

    SessionStore(tmp_path / "sessions.db")
    con = _sqlite3.connect(tmp_path / "sessions.db")
    names = {r[0] for r in con.execute(
        "SELECT name FROM sqlite_master WHERE type = 'index'"
    )}
    con.close()
    assert "idx_sessions_created" in names


def test_store_enables_wal_and_busy_timeout(tmp_path):
    """WAL (readers don't block the writer) + a 5s busy_timeout (wait instead
    of erroring 'database is locked') — both matter now that store calls run
    on worker threads off the event loop."""
    store = _store(tmp_path)
    try:
        mode = store._conn.execute("PRAGMA journal_mode").fetchone()[0]
        assert mode.lower() == "wal"
        assert store._conn.execute("PRAGMA busy_timeout").fetchone()[0] == 5000
    finally:
        store.close()


def test_store_concurrent_writes_are_safe(tmp_path):
    """The internal lock makes the single shared connection safe under
    concurrent threads — mirroring the asyncio.to_thread offload the chat
    routes use. Without it, concurrent cursor use raises or corrupts."""
    import threading

    store = _store(tmp_path)
    try:
        store.new_session(session_id="s1")
        errors: list[Exception] = []

        def worker(n: int) -> None:
            try:
                for i in range(25):
                    store.append_message("s1", "user", f"m{n}-{i}")
            except Exception as exc:  # pragma: no cover (failure path)
                errors.append(exc)

        threads = [threading.Thread(target=worker, args=(n,)) for n in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert errors == []
        assert len(store.replay("s1")) == 8 * 25
    finally:
        store.close()
