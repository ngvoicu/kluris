"""Boot-time builder for the persistent FTS5 search index.

The runtime search engine (:mod:`kluris_runtime.search_fts`) is part of the
WRITE-FREE runtime — it may read an on-disk index but never create one. This
pack-side module owns the single write: at boot it materializes the searchable
rows into an FTS5 database under the pack's writable ``data_dir``, then
registers the file with the runtime engine. From that point every unfiltered
search runs against the prebuilt index over a fresh per-query read-only
connection instead of rebuilding an in-memory table per call — the dominant
search cost on large brains.

The build is strictly best-effort: any failure leaves the engine on its
in-memory path, so search can get slower but never broken.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

from kluris_runtime.search_fts import (
    _BM25_WEIGHTS,
    fts5_available,
    register_db,
)

# Filename under data_dir/cache/. Rebuilt on every boot: the data volume
# outlives the image, so a stale index from a previous brain version must
# never be served.
SEARCH_DB_NAME = "search-index.sqlite"

# Sanity: the on-disk schema must keep the indexed columns (and their bm25
# weight order) identical to the in-memory table, or ranking would drift.
assert len(_BM25_WEIGHTS) == 4


def build_search_db(brain_path: Path, rows: list[dict], db_path: Path) -> bool:
    """Build the persistent index at ``db_path`` from searchable ``rows``.

    Returns ``True`` and registers the file with the runtime engine on
    success; returns ``False`` (leaving the in-memory path active) on any
    failure or when this interpreter's sqlite3 lacks FTS5.
    """
    if not fts5_available():
        return False
    db_path = Path(db_path)
    try:
        db_path.parent.mkdir(parents=True, exist_ok=True)
        for suffix in ("", "-wal", "-shm", "-journal"):
            Path(str(db_path) + suffix).unlink(missing_ok=True)
        con = sqlite3.connect(db_path)
        try:
            con.execute(
                "CREATE VIRTUAL TABLE docs USING fts5("
                "title, tags, path, body, "
                "file_type UNINDEXED, deprecated UNINDEXED, "
                "tokenize='unicode61')"
            )
            con.executemany(
                "INSERT INTO docs(title, tags, path, body, file_type, deprecated) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                [
                    (
                        it["title"],
                        " ".join(str(t) for t in it.get("tags", [])),
                        it["file"],
                        it["body"],
                        it.get("file_type", "markdown"),
                        1 if it["is_deprecated"] else 0,
                    )
                    for it in rows
                ],
            )
            con.commit()
        finally:
            con.close()
    except (sqlite3.Error, OSError):
        return False
    register_db(brain_path, db_path)
    return True
