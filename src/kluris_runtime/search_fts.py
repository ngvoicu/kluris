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

Two boot-time opt-ins make search scale to very large brains (both built by
:func:`build_index`; the brain is immutable inside the container, so both are
valid for the whole process lifetime):

- a cache of the ``collect_searchable()`` walk, so per-request search never
  re-reads the brain from disk;
- a PERSISTENT on-disk FTS5 index, so unfiltered queries skip the per-query
  table rebuild entirely (linear in corpus size — the dominant search cost at
  10k+ neurons) and instead open a fresh per-query READ-ONLY connection on
  the calling thread. No shared connection ever crosses a thread boundary, so
  the engine stays stateless and thread-safe regardless of how requests are
  dispatched.

Lobe/tag-FILTERED queries still build a fresh in-memory table from the
eligible subset of the cached rows: filtering BEFORE indexing keeps BM25 IDF
scoped to the eligible rows, so filtered ranking is unchanged. Standalone
callers and unit tests that never boot an app fall back to a fresh per-query
walk + build — identical behavior.
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


# Cap on phrase-pre-pass hits promoted ahead of the OR window. A CONTIGUOUS
# phrase is far rarer than its OR token-set, so this is generous headroom.
_PHRASE_PROMOTE_CAP = 50


def _phrase_expr(query: str) -> str | None:
    """FTS5 CONTIGUOUS-phrase expression for ``query``, or ``None`` for a
    single token.

    The OR expression (:func:`_match_expr`) tokenizes a multi-word query into
    independent prefix terms. On a homogeneous corpus where each term is
    near-zero-IDF (e.g. ``acquirer``/``transaction``/``fee`` each occur in most
    neurons), bm25 collapses to a near-flat score and the exact-named neuron
    loses to term-dense overview pages — even with the 10x title weight, which
    multiplies a near-zero IDF. A phrase MATCH requires the tokens CONTIGUOUS,
    which is rare enough to rank an exact/near-exact match decisively. Phrase
    hits are a SUBSET of the OR hits, so promoting them ahead of the OR results
    only re-orders the same match set — ``total`` is unchanged. Single-token
    queries need no phrase pass (phrase == OR), so return ``None`` to skip it.
    """
    tokens = re.findall(r"\w+", query.lower())
    if len(tokens) < 2:
        return None
    return '"' + " ".join(tokens) + '"'


def _phrase_promote(
    phrase_results: list[dict], or_results: list[dict], limit: int, offset: int
) -> list[dict]:
    """Merge ``phrase_results`` ahead of ``or_results``, dedup, and page.

    Dedup key is ``(file, title)`` — NOT ``file`` — because every glossary
    entry shares ``file == "glossary.md"`` but is a distinct result. Phrase
    hits lead (precision); OR hits backfill (recall). Returns the
    ``[offset:offset+limit]`` slice of the merged order.
    """
    seen: set[tuple[str, str]] = set()
    merged: list[dict] = []
    for r in phrase_results + or_results:
        key = (r["file"], r.get("title", ""))
        if key in seen:
            continue
        seen.add(key)
        merged.append(r)
    return merged[offset:offset + limit]


def _bucket_promote(
    lobes: dict[str, list[dict]], phrase_results: list[dict], per_lobe: int
) -> dict[str, list[dict]]:
    """Promote contiguous-phrase hits to the front of each lobe's bucket,
    re-capping at ``per_lobe``. Dedup key ``(file, title)``. Mutates and
    returns ``lobes``."""
    by_lobe: dict[str, list[dict]] = {}
    for r in phrase_results:
        by_lobe.setdefault(_lobe_of(r["file"]), []).append(r)
    for lobe, hits in by_lobe.items():
        seen: set[tuple[str, str]] = set()
        merged: list[dict] = []
        for r in hits + lobes.get(lobe, []):
            key = (r["file"], r.get("title", ""))
            if key in seen:
                continue
            seen.add(key)
            merged.append(r)
        lobes[lobe] = merged[:per_lobe]
    return lobes


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


def _snippet(body: str, tokens: list[str], width: int = 200) -> str:
    """Body snippet centered on the first query token found in the body."""
    lowered = body.lower()
    for tok in tokens:
        if tok in lowered:
            return extract_snippet(body, tok, width=width)
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


def _query_table(
    con: sqlite3.Connection, expr: str, limit: int, offset: int = 0
) -> list:
    """Run the BM25 MATCH query, returning ``(rowid, raw_score)`` rows."""
    weights = ", ".join(str(w) for w in _BM25_WEIGHTS)
    return con.execute(
        f"SELECT rowid, bm25(docs, {weights}) AS score "
        "FROM docs WHERE docs MATCH ? ORDER BY score, path LIMIT ? OFFSET ?",
        (expr, limit, offset),
    ).fetchall()


def _match_total(con: sqlite3.Connection, expr: str) -> int:
    """Full match count for ``expr`` (the page window's denominator)."""
    return int(
        con.execute(
            "SELECT count(*) FROM docs WHERE docs MATCH ?", (expr,)
        ).fetchone()[0]
    )


def _rows_to_results(
    items: list[dict],
    rows: list,
    tokens: list[str],
    *,
    snippet_chars: int = 200,
    include_bodies: int = 0,
) -> list[dict]:
    """Map ``(rowid, raw_score)`` rows back to result dicts."""
    results: list[dict] = []
    for rowid, raw_score in rows:
        item = items[rowid - 1]
        fields = _matched_fields(item, tokens)
        result = {
            "file": item["file"],
            "file_type": item.get("file_type", "markdown"),
            "title": item["title"],
            "matched_fields": fields,
            "snippet": (
                _snippet(item["body"], tokens, snippet_chars)
                if "body" in fields else ""
            ),
            # bm25() returns more-negative = more-relevant; expose a positive
            # score so "higher is better" matches the substring engine's
            # convention for any consumer that compares scores.
            "score": round(-raw_score, 4),
            "deprecated": item["is_deprecated"],
        }
        if len(results) < include_bodies:
            result["body"] = item["body"]
        results.append(result)
    return results


# Opt-in cache of the collect_searchable() walk, populated only by
# build_index() at app boot, keyed by resolved brain_path. Mirrors the
# module-global _fts5_supported guard: written once before requests are served
# and read-only thereafter. Stores only plain data (no SQLite connection), so
# there is nothing thread-bound to leak; each query builds its own table.
_INDEX_REGISTRY: dict[Path, list[dict]] = {}

# Opt-in persistent FTS5 index, built once at boot when build_index() is
# given a db_path. Unfiltered queries open a fresh per-query READ-ONLY
# connection to it on the calling thread — no shared connection ever crosses
# a thread boundary, and the per-query table REBUILD (linear in corpus size,
# the dominant search cost at 10k+ neurons) disappears. Filtered queries
# still rebuild from the eligible subset so BM25 IDF stays scoped to it.
_DB_REGISTRY: dict[Path, Path] = {}

# On-disk schema: same four indexed columns (and bm25 weights) as the
# in-memory table, plus two UNINDEXED payload columns the result mapper reads
# back. Weights for unindexed columns are required positionally and ignored.
_DB_WEIGHTS = (*_BM25_WEIGHTS, 0.0, 0.0)


def build_index(
    brain_path: Path,
    *,
    rows: list[dict] | None = None,
) -> None:
    """Cache the searchable walk for ``brain_path`` (opt-in).

    Called once at app boot. The brain is immutable inside the container, so
    the cache is valid for the process lifetime. ``rows`` lets the boot
    snapshot's single walk feed this too (must be ``collect_searchable``
    shaped).

    This module is part of the WRITE-FREE runtime, so the persistent on-disk
    index is built by the pack layer (``kluris.pack.search_index``) and
    announced here via :func:`register_db`.
    """
    resolved = brain_path.resolve()
    source = rows if rows is not None else collect_searchable(brain_path)
    _INDEX_REGISTRY[resolved] = source


def register_db(brain_path: Path, db_path: Path) -> None:
    """Register a persistent FTS5 index file for ``brain_path``.

    The file is built by the pack layer (this runtime is write-free); from
    here on, unfiltered queries are served from it over per-query read-only
    connections.
    """
    _DB_REGISTRY[brain_path.resolve()] = Path(db_path)


def drop_index(brain_path: Path) -> None:
    """Forget a cached walk + persistent index (invalidation / teardown)."""
    resolved = brain_path.resolve()
    _INDEX_REGISTRY.pop(resolved, None)
    _DB_REGISTRY.pop(resolved, None)


def _clear_index_registry() -> None:
    """Reset the caches (tests only)."""
    _INDEX_REGISTRY.clear()
    _DB_REGISTRY.clear()


def _query_db(db_path: Path, expr: str, limit: int, offset: int) -> tuple[list, int]:
    """Query the persistent index over a fresh read-only connection.

    Returns ``(rows, total)`` where each row carries the score plus every
    field the result mapper needs — no in-RAM items list required.
    """
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        weights = ", ".join(str(w) for w in _DB_WEIGHTS)
        rows = con.execute(
            f"SELECT bm25(docs, {weights}) AS score, "
            "title, tags, path, body, file_type, deprecated "
            "FROM docs WHERE docs MATCH ? ORDER BY score, path LIMIT ? OFFSET ?",
            (expr, limit, offset),
        ).fetchall()
        return rows, _match_total(con, expr)
    finally:
        con.close()


def _db_rows_to_results(
    rows: list,
    tokens: list[str],
    *,
    snippet_chars: int = 200,
    include_bodies: int = 0,
) -> list[dict]:
    """Map persistent-index rows to result dicts (same shape as in-memory)."""
    results: list[dict] = []
    for raw_score, title, tags_str, file, body, file_type, deprecated in rows:
        # _matched_fields space-joins the tags list before matching, so a
        # single-element list holding the stored joined string matches
        # identically to the original list.
        item = {
            "title": title,
            "tags": [tags_str] if tags_str else [],
            "file": file,
            "body": body,
        }
        fields = _matched_fields(item, tokens)
        result = {
            "file": file,
            "file_type": file_type,
            "title": title,
            "matched_fields": fields,
            "snippet": (
                _snippet(body, tokens, snippet_chars)
                if "body" in fields else ""
            ),
            "score": round(-raw_score, 4),
            "deprecated": bool(deprecated),
        }
        if len(results) < include_bodies:
            result["body"] = body
        results.append(result)
    return results


def search_brain_fts_paged(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    offset: int = 0,
    lobe_filter: str | None = None,
    tag_filter: str | None = None,
    snippet_chars: int = 200,
    include_bodies: int = 0,
) -> dict:
    """BM25/FTS5 ranked search with deterministic pagination.

    Returns ``{"results": [...], "total": int}`` — ``total`` is the full match
    count so callers can page a broad result set instead of re-querying with
    permuted phrasings.

    Unfiltered queries are served from the boot-built persistent index when
    one is registered (see :func:`build_index`) over a fresh per-query
    read-only connection — no rebuild, no shared connection. Lobe/tag-filtered
    queries (and brains without a persistent index) keep the per-query table
    build over the eligible rows so BM25 IDF stays scoped to the subset.
    Falls back to the substring engine when FTS5 is unavailable, the query has
    no usable tokens, or anything errors — always safe to call.
    """
    from kluris_runtime.search import search_brain_paged  # fallback only

    if limit <= 0:
        return {"results": [], "total": 0}
    offset = max(0, int(offset))

    expr, tokens = _match_expr(query)
    if expr is None or not fts5_available():
        return search_brain_paged(
            brain_path, query, limit=limit, offset=offset,
            lobe_filter=lobe_filter, tag_filter=tag_filter,
            snippet_chars=snippet_chars, include_bodies=include_bodies,
        )

    resolved = brain_path.resolve()
    phrase = _phrase_expr(query)
    # When promoting, phrase hits (<= cap) prepended can push OR rows past the
    # requested page, so widen the OR fetch enough to refill it after the merge.
    or_limit = limit if phrase is None else offset + limit + _PHRASE_PROMOTE_CAP
    or_offset = offset if phrase is None else 0

    if lobe_filter is None and tag_filter is None:
        db_path = _DB_REGISTRY.get(resolved)
        if db_path is not None:
            try:
                or_rows, total = _query_db(db_path, expr, or_limit, or_offset)
                or_res = _db_rows_to_results(
                    or_rows, tokens, snippet_chars=snippet_chars,
                    include_bodies=include_bodies)
                if phrase is None:
                    return {"results": or_res, "total": total}
                ph_rows, _ = _query_db(db_path, phrase, _PHRASE_PROMOTE_CAP, 0)
                ph_res = _db_rows_to_results(
                    ph_rows, tokens, snippet_chars=snippet_chars,
                    include_bodies=include_bodies)
                return {"results": _phrase_promote(ph_res, or_res, limit, offset),
                        "total": total}
            except sqlite3.Error:
                pass  # degrade to the in-memory build below

    # Reuse the cached walk when available, else walk fresh (today's path).
    cached = _INDEX_REGISTRY.get(resolved)
    source = cached if cached is not None else collect_searchable(brain_path)

    try:
        # Filter BEFORE indexing so BM25 IDF is scoped to the eligible rows,
        # exactly as today. The fresh per-query connection lives only on this
        # thread, so the engine is thread-safe however requests are dispatched.
        # The OR and phrase queries run on the SAME table — built once.
        items = [it for it in source if _passes(it, lobe_filter, tag_filter)]
        if not items:
            return {"results": [], "total": 0}
        con = _build_fts_table(items)
        try:
            or_rows = _query_table(con, expr, or_limit, or_offset)
            total = _match_total(con, expr)
            or_res = _rows_to_results(
                items, or_rows, tokens,
                snippet_chars=snippet_chars, include_bodies=include_bodies)
            if phrase is None:
                ph_res = None
            else:
                ph_rows = _query_table(con, phrase, _PHRASE_PROMOTE_CAP, 0)
                ph_res = _rows_to_results(
                    items, ph_rows, tokens,
                    snippet_chars=snippet_chars, include_bodies=include_bodies)
        finally:
            con.close()
        if ph_res is None:
            return {"results": or_res, "total": total}
        return {"results": _phrase_promote(ph_res, or_res, limit, offset),
                "total": total}
    except sqlite3.Error:
        return search_brain_paged(
            brain_path, query, limit=limit, offset=offset,
            lobe_filter=lobe_filter, tag_filter=tag_filter,
            snippet_chars=snippet_chars, include_bodies=include_bodies,
        )


def search_brain_fts(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe_filter: str | None = None,
    tag_filter: str | None = None,
) -> list[dict]:
    """BM25/FTS5 ranked search. Same shape as ``search.search_brain``."""
    return search_brain_fts_paged(
        brain_path, query, limit=limit,
        lobe_filter=lobe_filter, tag_filter=tag_filter,
    )["results"]


def _lobe_of(file: str) -> str:
    return file.split("/", 1)[0] if "/" in file else "(root)"


def _query_db_grouped(
    db_path: Path, expr: str, per_lobe: int
) -> tuple[list, int]:
    """Exact per-lobe top-K over the persistent index, in ONE query.

    A flat top-N window cannot give per-lobe coverage — on a homogeneous
    corpus the highest-ranked N bunch into one lobe. The window function
    partitions by the path's first segment, so every lobe with a match
    surfaces its own best hits regardless of how other lobes rank.
    """
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        weights = ", ".join(str(w) for w in _DB_WEIGHTS)
        rows = con.execute(
            "SELECT score, title, tags, path, body, file_type, deprecated "
            "FROM ("
            "  SELECT t.*, ROW_NUMBER() OVER ("
            "    PARTITION BY lobe ORDER BY score, path) AS rn"
            "  FROM ("
            f"    SELECT bm25(docs, {weights}) AS score, title, tags, path, "
            "          body, file_type, deprecated, "
            "          CASE WHEN instr(path, '/') > 0 "
            "               THEN substr(path, 1, instr(path, '/') - 1) "
            "               ELSE '(root)' END AS lobe "
            "    FROM docs WHERE docs MATCH ?"
            "  ) t"
            ") WHERE rn <= ? ORDER BY lobe, rn",
            (expr, per_lobe),
        ).fetchall()
        return rows, _match_total(con, expr)
    finally:
        con.close()


def _query_table_grouped(
    con: sqlite3.Connection, expr: str, per_lobe: int
) -> list:
    """Per-lobe top-K over an in-memory ``docs`` table, returning
    ``(rowid, raw_score)`` rows ordered by (lobe, in-lobe rank).

    Same global-IDF + ``ROW_NUMBER() OVER (PARTITION BY lobe)`` shape as the
    on-disk grouped query, so the two paths rank identically. The in-memory
    table carries a ``path`` column, so the lobe is derived the same way.
    """
    weights = ", ".join(str(w) for w in _BM25_WEIGHTS)
    return con.execute(
        "SELECT rid, score FROM ("
        "  SELECT t.*, ROW_NUMBER() OVER ("
        "    PARTITION BY lobe ORDER BY score, path) AS rn"
        "  FROM ("
        f"    SELECT rowid AS rid, bm25(docs, {weights}) AS score, path, "
        "          CASE WHEN instr(path, '/') > 0 "
        "               THEN substr(path, 1, instr(path, '/') - 1) "
        "               ELSE '(root)' END AS lobe "
        "    FROM docs WHERE docs MATCH ?"
        "  ) t"
        ") WHERE rn <= ? ORDER BY lobe, rn",
        (expr, per_lobe),
    ).fetchall()


def search_brain_fts_grouped(
    brain_path: Path,
    query: str,
    *,
    per_lobe: int = 3,
    snippet_chars: int = 200,
) -> dict:
    """Top hits PER top-level lobe — the one-call answer to "X across every
    lobe". Returns ``{"lobes": {lobe: [results]}, "total": int}``.

    On the persistent index this is a single partitioned window query; the
    in-memory path builds one small table per lobe over the (cached) rows.
    Falls back to bucketing the substring engine's ranked list when FTS5 is
    unavailable — degraded but never erroring.
    """
    from kluris_runtime.search import search_brain_paged  # fallback only

    if per_lobe <= 0:
        return {"lobes": {}, "total": 0}

    expr, tokens = _match_expr(query)
    if expr is None or not fts5_available():
        paged = search_brain_paged(
            brain_path, query, limit=per_lobe * 64,
            snippet_chars=snippet_chars,
        )
        lobes: dict[str, list[dict]] = {}
        for hit in paged["results"]:
            bucket = lobes.setdefault(_lobe_of(hit["file"]), [])
            if len(bucket) < per_lobe:
                bucket.append(hit)
        return {"lobes": lobes, "total": paged["total"]}

    resolved = brain_path.resolve()

    db_path = _DB_REGISTRY.get(resolved)
    if db_path is not None:
        try:
            rows, total = _query_db_grouped(db_path, expr, per_lobe)
            lobes = {}
            for result in _db_rows_to_results(
                rows, tokens, snippet_chars=snippet_chars
            ):
                lobes.setdefault(_lobe_of(result["file"]), []).append(result)
            phrase = _phrase_expr(query)
            if phrase is not None:
                ph_rows, _ = _query_db(db_path, phrase, _PHRASE_PROMOTE_CAP, 0)
                lobes = _bucket_promote(
                    lobes,
                    _db_rows_to_results(ph_rows, tokens, snippet_chars=snippet_chars),
                    per_lobe)
            return {"lobes": lobes, "total": total}
        except sqlite3.Error:
            pass  # degrade to the in-memory path below

    # In-memory path: build ONE table over ALL rows and partition with the
    # same window query as the on-disk path, so BM25 IDF is GLOBAL on both —
    # within-lobe ranking is identical whether or not a persistent index is
    # registered (a per-lobe subset build would scope IDF differently and
    # diverge). Exact per-lobe top-K, no coverage window.
    cached = _INDEX_REGISTRY.get(resolved)
    source = cached if cached is not None else collect_searchable(brain_path)
    try:
        con = _build_fts_table(source)
        try:
            rows = _query_table_grouped(con, expr, per_lobe)
            total = _match_total(con, expr)
            lobes = {}
            for result in _rows_to_results(
                source, rows, tokens, snippet_chars=snippet_chars
            ):
                lobes.setdefault(_lobe_of(result["file"]), []).append(result)
            # Phrase pre-pass on the SAME table (no extra build): promote
            # contiguous-phrase hits to the front of each lobe.
            phrase = _phrase_expr(query)
            if phrase is not None:
                ph_rows = _query_table(con, phrase, _PHRASE_PROMOTE_CAP, 0)
                lobes = _bucket_promote(
                    lobes,
                    _rows_to_results(source, ph_rows, tokens,
                                     snippet_chars=snippet_chars),
                    per_lobe)
        finally:
            con.close()
        return {"lobes": lobes, "total": total}
    except sqlite3.Error:
        # FTS5 genuinely unavailable mid-flight: bucket the substring engine's
        # FULL ranked list (no window cap, so every matched lobe is covered).
        paged = search_brain_paged(
            brain_path, query, limit=len(source) + 1,
            snippet_chars=snippet_chars,
        )
        lobes = {}
        for hit in paged["results"]:
            bucket = lobes.setdefault(_lobe_of(hit["file"]), [])
            if len(bucket) < per_lobe:
                bucket.append(hit)
        return {"lobes": lobes, "total": paged["total"]}
