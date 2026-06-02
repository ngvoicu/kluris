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

The index is built in a fresh in-memory database PER QUERY from the same
``collect_searchable()`` walk the substring engine uses — identical file I/O to
today, plus a sub-millisecond insert of a few hundred rows. The brain is
immutable inside the container, so a build-once-at-boot cache is a safe future
optimization; v1 keeps the per-request model to stay simple, stateless, and
thread-safe (a fresh connection per call).
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


def search_brain_fts(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe_filter: str | None = None,
    tag_filter: str | None = None,
) -> list[dict]:
    """BM25/FTS5 ranked search. Same shape as ``search.search_brain``.

    Falls back to the substring engine when FTS5 is unavailable, the query has
    no usable tokens, or anything errors — so it is always safe to call in
    place of ``search_brain``.
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

    try:
        items = [
            it for it in collect_searchable(brain_path)
            if _passes(it, lobe_filter, tag_filter)
        ]
        if not items:
            return []

        weights = ", ".join(str(w) for w in _BM25_WEIGHTS)
        con = sqlite3.connect(":memory:")
        try:
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
            rows = con.execute(
                f"SELECT rowid, bm25(docs, {weights}) AS score "
                "FROM docs WHERE docs MATCH ? ORDER BY score LIMIT ?",
                (expr, limit),
            ).fetchall()
        finally:
            con.close()
    except sqlite3.Error:
        return search_brain(
            brain_path, query, limit=limit,
            lobe_filter=lobe_filter, tag_filter=tag_filter,
        )

    # FTS5 auto-assigns rowid 1..N in insert order → items[rowid - 1].
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
