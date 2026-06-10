"""SQLite-backed conversation history.

Stores sessions and messages at ``/data/sessions.db`` (file mode 0600
where the platform supports it). One session per browser cookie; new
conversations rotate the cookie + create a fresh session row.

Schema (idempotent ``CREATE IF NOT EXISTS``):

```sql
CREATE TABLE sessions (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_calls_json TEXT,
    tool_use_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_created ON messages(session_id, created_at);
```
"""

from __future__ import annotations

import os
import sqlite3
import time
import uuid
from contextlib import contextmanager
from pathlib import Path
from typing import Any


_SCHEMA = """
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_calls_json TEXT,
    tool_use_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session_created
    ON messages(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_sessions_created
    ON sessions(created_at);
"""


class SessionStore:
    """Thin wrapper over a single :class:`sqlite3.Connection`."""

    def __init__(self, db_path: Path) -> None:
        self.db_path = Path(db_path)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        # Ensure the DB file exists with 0600 perms before sqlite opens
        # it (sqlite respects umask, so we can't rely on default mode).
        if not self.db_path.exists():
            fd = os.open(str(self.db_path), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(fd)
        else:
            try:
                os.chmod(self.db_path, 0o600)
            except OSError:
                pass
        self._conn = sqlite3.connect(str(self.db_path), isolation_level=None,
                                     check_same_thread=False)
        self._conn.execute("PRAGMA foreign_keys = ON")
        self._conn.executescript(_SCHEMA)

    @contextmanager
    def cursor(self):
        cur = self._conn.cursor()
        try:
            yield cur
        finally:
            cur.close()

    def close(self) -> None:
        self._conn.close()

    # --- Sessions ----------------------------------------------------

    def new_session(self, *, session_id: str | None = None) -> str:
        sid = session_id or uuid.uuid4().hex
        now = int(time.time())
        with self.cursor() as cur:
            cur.execute(
                "INSERT OR REPLACE INTO sessions(id, created_at) VALUES (?, ?)",
                (sid, now),
            )
        return sid

    def session_exists(self, sid: str) -> bool:
        with self.cursor() as cur:
            cur.execute("SELECT 1 FROM sessions WHERE id = ?", (sid,))
            return cur.fetchone() is not None

    def list_sessions(self, *, limit: int = 100) -> list[dict[str, Any]]:
        """Return recent NON-EMPTY sessions with message counts + first-user
        preview.

        Ordered by ``created_at`` descending, capped at ``limit``. Sessions
        with zero messages (e.g. a page load that opened a fresh conversation
        but never sent a message) are excluded — the "Past conversations"
        picker only lists conversations that actually have content. The
        ``EXISTS`` filter runs before ``LIMIT``, so the cap applies to
        non-empty sessions. The preview is the first user message truncated
        at 200 chars, a header line for otherwise-opaque hex session IDs.
        """
        if limit <= 0:
            return []
        with self.cursor() as cur:
            cur.execute(
                """
                SELECT
                    s.id,
                    s.created_at,
                    (SELECT COUNT(*) FROM messages m
                        WHERE m.session_id = s.id) AS msg_count,
                    (SELECT content FROM messages m
                        WHERE m.session_id = s.id AND m.role = 'user'
                        ORDER BY m.id ASC LIMIT 1) AS first_user
                FROM sessions s
                WHERE EXISTS (
                    SELECT 1 FROM messages m WHERE m.session_id = s.id
                )
                ORDER BY s.created_at DESC
                LIMIT ?
                """,
                (limit,),
            )
            rows = cur.fetchall()
        out: list[dict[str, Any]] = []
        for r in rows:
            preview = (r[3] or "").strip()
            if len(preview) > 200:
                preview = preview[:200].rstrip() + "…"
            out.append({
                "id": r[0],
                "created_at": r[1] or 0,
                "message_count": r[2] or 0,
                "preview": preview,
            })
        return out

    def delete_session(self, sid: str) -> None:
        """Cascade-delete a session and all its messages."""
        with self.cursor() as cur:
            # Foreign keys are ON, but be explicit so the delete still
            # works if a future schema migration relaxes the cascade.
            cur.execute("DELETE FROM messages WHERE session_id = ?", (sid,))
            cur.execute("DELETE FROM sessions WHERE id = ?", (sid,))

    def prune_old_sessions(self, retention_days: int) -> int:
        """Delete sessions (and their messages) older than ``retention_days``.

        Returns the number of sessions removed. ``retention_days <= 0`` is a
        no-op — retention is strictly opt-in, deleting a deployer's history
        must never be a surprise default. Called at boot; a long-running
        container otherwise accumulates a session row per page load and a
        full transcript per turn, forever.
        """
        if retention_days <= 0:
            return 0
        cutoff = int(time.time()) - retention_days * 86400
        with self.cursor() as cur:
            cur.execute(
                "DELETE FROM messages WHERE session_id IN "
                "(SELECT id FROM sessions WHERE created_at < ?)",
                (cutoff,),
            )
            cur.execute(
                "DELETE FROM sessions WHERE created_at < ?", (cutoff,),
            )
            return cur.rowcount or 0

    # --- Messages ----------------------------------------------------

    def append_message(
        self,
        session_id: str,
        role: str,
        content: str,
        *,
        tool_calls_json: str | None = None,
        tool_use_id: str | None = None,
    ) -> int:
        now = int(time.time() * 1000)
        with self.cursor() as cur:
            cur.execute(
                "INSERT INTO messages(session_id, role, content, "
                "tool_calls_json, tool_use_id, created_at) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                (session_id, role, content, tool_calls_json, tool_use_id, now),
            )
            return cur.lastrowid or 0

    def replay(self, session_id: str) -> list[dict[str, Any]]:
        with self.cursor() as cur:
            cur.execute(
                "SELECT id, role, content, tool_calls_json, tool_use_id, created_at "
                "FROM messages WHERE session_id = ? ORDER BY id ASC",
                (session_id,),
            )
            rows = cur.fetchall()
        return [
            {
                "id": r[0],
                "role": r[1],
                "content": r[2],
                "tool_calls_json": r[3],
                "tool_use_id": r[4],
                "created_at": r[5],
            }
            for r in rows
        ]
