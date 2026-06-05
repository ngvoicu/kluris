"""BM25 brain search via SQLite FTS5.

A drop-in alternative to :func:`kluris_runtime.search.search_brain` with the
SAME signature and result-dict shape, but ranked by FTS5's ``bm25()`` instead
of literal-substring counts. Two wins over substring scoring:

- **Tokenization** — a multi-word query matches its terms independently, so
  ``"auth flow"`` finds a neuron mentioning *auth* and *flow* far apart (which
  substring search, requiring the contiguous phrase, scores 0).
- **TF-IDF ranking + length normalization** — rare/discriminative terms
  outrank common ones, and a short glossary line competes fairly with a long
  neuron body.

Each token is matched as a PREFIX (``"auth"*`` reaches *authentication*), so
recall is at least as good as the substring engine for the common case while
fixing its multi-word blind spot.

FTS5 ships compiled into the stdlib ``sqlite3`` on the pack's
``python:3.12-slim`` base image. :func:`fts5_available` guards the path so the
same code degrades to the substring engine anywhere FTS5 is missing; any query
error falls back the same way. Callers can therefore use this in place of
``search_brain`` unconditionally.

The dominant per-query cost is the ``collect_searchable()`` brain file-walk, not
the sub-millisecond FTS5 table build. So the pack opts the configured brain into
a BUILD-ONCE cache of that walk at boot (see :func:`build_index`): the brain is
immutable inside the container, so the cached walk is valid for the whole
process lifetime and per-request search skips the file I/O. Each query still
builds its OWN fresh in-memory FTS5 table from the cached rows on the CALLING
thread, so there is no shared SQLite connection to cross a thread boundary — the
engine stays stateless and thread-safe regardless of how requests are dispatched
(important: the route handler runs on a different thread than ``create_app``).
The opt-in is keyed by resolved ``brain_path`` so the agent and route callers
both reuse it via ``Config.brain_dir`` with no signature changes; standalone
callers and unit tests that never boot an app fall back to a fresh per-query
walk — identical behavior. Filtering stays BEFORE indexing so BM25 IDF is scoped
to the eligible rows, exactly as today.
"""

from __future__ import annotations

import re
import sqlite3
from pathlib import Path

from kluris_runtime.search import collect_searchable, extract_snippet

# bm25() column weights, mirroring the substring engine's field weights
# (title 10, tag 5, path 3, body 1) so ranking intent is preserved. These are
# constants (never user input), so they are safe to inline into the SQL.
_BM25_WEIGHTS = (10.0, 5.0, 3.0, 1.0)

# Field names in FTS5 column order, also the order matched_fields is reported
# in (matching the substring engine's _FIELD_WEIGHTS ordering).
_FIELDS = ("title", "tag", "path", "body")

_fts5_supported: bool | None = None


def fts5_available() -> bool:
    """Return True iff this interpreter's ``sqlite3`` has the FTS5 extension.

    Probed once and cached. The pack's base image has it; this guard lets the
    same code degrade gracefully where it doesn't.
    """
    global _fts5_supported
    if _fts5_supported is None:
        try:
            con = sqlite3.connect(":memory:")
            try:
                con.execute("CREATE VIRTUAL TABLE _probe USING fts5(x)")
                _fts5_supported = True
            finally:
                con.close()
        except sqlite3.Error:
            _fts5_supported = False
    return _fts5_supported


def _match_expr(query: str) -> tuple[str | None, list[str]]:
    """Turn free text into a safe FTS5 MATCH expression + the raw tokens.

    Tokens are reduced to word characters and each is quoted as an FTS5 string
    literal (neutralizing FTS5 operators), suffixed with ``*`` for prefix
    matching, then OR-joined for recall — ``bm25()`` ranks documents matching
    more/rarer tokens higher. Returns ``(None, [])`` for a query with no usable
    tokens (caller falls back to the substring engine).
    """
    tokens = re.findall(r"\w+", query.lower())
    if not tokens:
        return None, []
    expr = " OR ".join(f'"{t}"*' for t in tokens)
    return expr, tokens


def _passes(item: dict, lobe_filter: str | None, tag_filter: str | None) -> bool:
    """Same lobe (path-prefix) + tag (exact membership) filtering as
    ``search_brain``, applied before indexing so the budget/limit is spent on
    eligible items only."""
    if lobe_filter is not None:
        if not item["file"].startswith(lobe_filter.rstrip("/") + "/"):
            return False
    if tag_filter is not None and tag_filter not in item.get("tags", []):
        return False
    return True


def _matched_fields(item: dict, tokens: list[str]) -> list[str]:
    """Fields where ANY query token appears (token-aware substring test),
    reported in ``_FIELDS`` order to match the substring engine's output."""
    texts = {
        "title": item["title"].lower(),
        "tag": " ".join(str(t).lower() for t in item.get("tags", [])),
        "path": item["file"].lower(),
        "body": item["body"].lower(),
    }
    return [f for f in _FIELDS if any(tok in texts[f] for tok in tokens)]


def _snippet(body: str, tokens: list[str]) -> str:
    """Body snippet centered on the first query token found in the body."""
    lowered = body.lower()
    for tok in tokens:
        if tok in lowered:
            return extract_snippet(body, tok)
    return ""


def _build_fts_table(items: list[dict]) -> sqlite3.Connection:
    """Build a fresh in-memory FTS5 table over ``items``.

    FTS5 assigns rowid 1..N in INSERT order, so the caller maps a result
    rowid back via ``items[rowid - 1]``. Insert order therefore also decides
    bm25 tie-breaks — keep ``items`` in its original walk order.
    """
    con = sqlite3.connect(":memory:")
    con.execute(
        "CREATE VIRTUAL TABLE docs USING fts5("
        "title, tags, path, body, tokenize='unicode61')"
    )
    con.executemany(
        "INSERT INTO docs(title, tags, path, body) VALUES (?, ?, ?, ?)",
        [
            (
                it["title"],
                " ".join(str(t) for t in it.get("tags", [])),
                it["file"],
                it["body"],
            )
            for it in items
        ],
    )
    return con


def _query_table(con: sqlite3.Connection, expr: str, limit: int) -> list:
    """Run the BM25 MATCH query, returning ``(rowid, raw_score)`` rows."""
    weights = ", ".join(str(w) for w in _BM25_WEIGHTS)
    return con.execute(
        f"SELECT rowid, bm25(docs, {weights}) AS score "
        "FROM docs WHERE docs MATCH ? ORDER BY score LIMIT ?",
        (expr, limit),
    ).fetchall()


def _rows_to_results(items: list[dict], rows: list, tokens: list[str]) -> list[dict]:
    """Map ``(rowid, raw_score)`` rows back to result dicts."""
    results: list[dict] = []
    for rowid, raw_score in rows:
        item = items[rowid - 1]
        fields = _matched_fields(item, tokens)
        results.append({
            "file": item["file"],
            "file_type": item.get("file_type", "markdown"),
            "title": item["title"],
            "matched_fields": fields,
            "snippet": _snippet(item["body"], tokens) if "body" in fields else "",
            # bm25() returns more-negative = more-relevant; expose a positive
            # score so "higher is better" matches the substring engine's
            # convention for any consumer that compares scores.
            "score": round(-raw_score, 4),
            "deprecated": item["is_deprecated"],
        })
    return results


# Opt-in cache of the collect_searchable() walk, populated only by
# build_index() at app boot, keyed by resolved brain_path. Mirrors the
# module-global _fts5_supported guard: written once before requests are served
# and read-only thereafter. Stores only plain data (no SQLite connection), so
# there is nothing thread-bound to leak; each query builds its own table.
_INDEX_REGISTRY: dict[Path, list[dict]] = {}


def build_index(brain_path: Path) -> None:
    """Cache the ``collect_searchable()`` walk for ``brain_path`` (opt-in).

    Called once at app boot. The brain is immutable inside the container, so
    the cached walk is valid for the process lifetime and lets every later
    query skip the brain file I/O (the dominant cost). Deliberately does NOT
    cache a built FTS5 table/connection: each query builds its own table on
    its own thread, keeping the engine stateless and thread-safe.
    """
    _INDEX_REGISTRY[brain_path.resolve()] = collect_searchable(brain_path)


def drop_index(brain_path: Path) -> None:
    """Forget a cached walk (explicit invalidation / test teardown)."""
    _INDEX_REGISTRY.pop(brain_path.resolve(), None)


def _clear_index_registry() -> None:
    """Reset the walk cache (tests only)."""
    _INDEX_REGISTRY.clear()


def search_brain_fts(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe_filter: str | None = None,
    tag_filter: str | None = None,
) -> list[dict]:
    """BM25/FTS5 ranked search. Same shape as ``search.search_brain``.

    Reuses a boot-cached brain walk (see :func:`build_index`) when one is
    registered for ``brain_path``, skipping the file I/O; otherwise walks the
    brain fresh. Either way it builds its own per-query in-memory FTS5 table on
    the calling thread. Falls back to the substring engine when FTS5 is
    unavailable, the query has no usable tokens, or anything errors — so it is
    always safe to call in place of ``search_brain``.
    """
    from kluris_runtime.search import search_brain  # local import: fallback only

    if limit <= 0:
        return []

    expr, tokens = _match_expr(query)
    if expr is None or not fts5_available():
        return search_brain(
            brain_path, query, limit=limit,
            lobe_filter=lobe_filter, tag_filter=tag_filter,
        )

    # Reuse the cached walk when available, else walk fresh (today's path).
    cached = _INDEX_REGISTRY.get(brain_path.resolve())
    source = cached if cached is not None else collect_searchable(brain_path)

    try:
        # Filter BEFORE indexing so BM25 IDF is scoped to the eligible rows,
        # exactly as today. The fresh per-query connection lives only on this
        # thread, so the engine is thread-safe however requests are dispatched.
        items = [it for it in source if _passes(it, lobe_filter, tag_filter)]
        if not items:
            return []
        con = _build_fts_table(items)
        try:
            rows = _query_table(con, expr, limit)
        finally:
            con.close()
        return _rows_to_results(items, rows, tokens)
    except sqlite3.Error:
        return search_brain(
            brain_path, query, limit=limit,
            lobe_filter=lobe_filter, tag_filter=tag_filter,
        )
