"""Eight read-only brain tool dispatchers.

Each function takes a ``brain_path`` plus the LLM-supplied arguments
and returns a JSON-serialisable dict. Path arguments are sandboxed via
:func:`kluris_runtime.neuron_index.is_within_brain` — anything that
would escape the brain root raises :class:`SandboxError`, surfaced to
the model as a structured tool error.

This module contains zero filesystem-write APIs by construction. A
greps-the-source test in ``tests.pack.test_readonly_enforcement``
fails CI if any are introduced here.
"""

from __future__ import annotations

import json
from difflib import get_close_matches
from pathlib import Path
from typing import Any

from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_excerpt import extract as extract_excerpt
from kluris_runtime.neuron_index import (
    YAML_NEURON_SUFFIXES,
    is_within_brain,
    neuron_files,
)
from kluris_runtime.search import (
    parse_glossary_entries,
    search_brain_paged,
)
from kluris_runtime.search_fts import (
    search_brain_fts_grouped,
    search_brain_fts_paged,
)
from kluris_runtime.snapshot import get_snapshot
from kluris_runtime.wake_up import _recency_key, build_payload


class NotFoundError(LookupError):
    """Path resolves inside the brain but no file exists at it."""


class SandboxError(ValueError):
    """Path escapes the brain root or otherwise violates the sandbox."""


def resolve_in_brain(brain_root: Path, raw: str) -> Path:
    """Resolve ``raw`` against ``brain_root`` enforcing the sandbox.

    Symlinks are resolved before the check, so a symlink inside the
    brain that points outside is rejected. Absolute paths and
    ``../`` traversals are likewise rejected.
    """
    if not isinstance(raw, str) or not raw:
        raise SandboxError("path argument must be a non-empty string")
    raw = raw.replace("\\", "/").lstrip("/")
    candidate = (brain_root / raw).resolve()
    if not is_within_brain(candidate, brain_root):
        raise SandboxError(f"path {raw!r} is outside the brain root")
    if not candidate.exists():
        raise NotFoundError(f"path {raw!r} not found")
    return candidate


def _rel(brain_path: Path, target: Path) -> str:
    # ``target`` is usually resolve_in_brain()-resolved while ``brain_path``
    # arrives as the caller passed it; when the brain path contains a symlink
    # segment (macOS /tmp, a symlinked deploy dir) the unresolved base is not
    # a prefix of the resolved target — fall back to the resolved base.
    try:
        rel = target.relative_to(brain_path)
    except ValueError:
        rel = target.relative_to(brain_path.resolve())
    return str(rel).replace("\\", "/")


# --- 1. wake_up --------------------------------------------------------------


# Opt-in cache of the wake_up payload, populated only by
# build_wake_up_cache() at app boot, keyed by resolved brain_path. The brain
# is immutable inside the container, so the snapshot is valid for the whole
# process lifetime. It holds plain JSON data (no connection/handle), so reads
# are safe from any request thread; callers treat the payload as read-only.
_WAKE_UP_CACHE: dict[Path, dict[str, Any]] = {}


def build_wake_up_cache(brain_path: Path, snapshot: dict | None = None) -> None:
    """Precompute and cache the wake_up snapshot for ``brain_path`` (opt-in).

    Called once at app boot so the agent's first-call-of-session wake_up and
    every ``/api/brain/tree`` UI load skip the brain re-walk —
    :func:`build_payload` otherwise reads every neuron's frontmatter twice per
    call (once for recency, once for deprecation diagnostics). With the boot
    ``snapshot`` it reuses that single walk instead and the payload gains the
    per-lobe ``top_tags`` routing hints.
    """
    _WAKE_UP_CACHE[brain_path.resolve()] = build_payload(
        brain_path, snapshot=snapshot
    )


def drop_wake_up_cache(brain_path: Path) -> None:
    """Forget a cached snapshot (explicit invalidation / test teardown)."""
    _WAKE_UP_CACHE.pop(brain_path.resolve(), None)


def _clear_wake_up_cache() -> None:
    """Reset the wake_up cache (tests only)."""
    _WAKE_UP_CACHE.clear()


def wake_up_tool(brain_path: Path) -> dict[str, Any]:
    """Live snapshot of the brain.

    Returns the boot-cached snapshot when one is registered for ``brain_path``
    (see :func:`build_wake_up_cache`), else computes it fresh — so standalone
    callers and unit tests that never boot an app are unchanged. Wraps
    :func:`kluris_runtime.wake_up.build_payload`. Does NOT include scaffold
    metadata (``type`` / ``type_structure``).
    """
    cached = _WAKE_UP_CACHE.get(brain_path.resolve())
    if cached is not None:
        return cached
    return build_payload(brain_path)


# --- 2. search ---------------------------------------------------------------


# Per-hit cap for ``full_bodies`` content. Deliberately much smaller than the
# read_neuron clamp: at the schema's max of 5 bodies per call this bounds the
# tool result to ~20KB regardless of neuron sizes.
_SEARCH_BODY_MAX_BYTES = 4096
_MAX_SNIPPET_CHARS = 2000
_MAX_FULL_BODIES = 5
# recent() page-size ceiling — mirrors the JSON schema's maximum. The runtime
# is the real boundary: a non-strict provider may ignore the schema maximum,
# and an unbounded limit would return the whole brain's recent list.
_MAX_RECENT_LIMIT = 100


def search_tool(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe: str | None = None,
    tag: str | None = None,
    offset: int = 0,
    snippet_chars: int | None = None,
    full_bodies: int = 0,
    group_by_lobe: bool = False,
) -> dict[str, Any]:
    """BM25 search across neurons + glossary + brain.md.

    Ranked by SQLite FTS5's ``bm25()`` (tokenized, prefix-matched,
    TF-IDF-weighted). Falls back to the literal-substring engine if FTS5 is
    unavailable or errors, so the tool can never regress to no results.

    Broad-question affordances (all opt-in, defaults preserve the classic
    shape): ``total`` always reports the FULL match count and ``offset`` pages
    deterministically through it; ``snippet_chars`` widens snippets;
    ``full_bodies`` attaches the clamped body of the top N hits so a question
    can be answered from one search; ``group_by_lobe`` returns the top hits
    PER lobe — the single-call answer to "X across every lobe/country".
    """
    if not isinstance(query, str) or not query.strip():
        return {"ok": False, "error": "query must be a non-empty string"}
    n = int(limit) if limit else 10
    off = max(0, int(offset) if offset else 0)
    chars = int(snippet_chars) if snippet_chars else 200
    chars = max(50, min(chars, _MAX_SNIPPET_CHARS))
    bodies = max(0, min(int(full_bodies) if full_bodies else 0, _MAX_FULL_BODIES))

    if group_by_lobe:
        # Grouping partitions by top-level lobe across the WHOLE brain;
        # lobe/tag filters don't compose with it and are ignored here.
        return _grouped_search(
            brain_path, query, per_lobe=n, snippet_chars=chars,
        )

    paged = _run_search(
        brain_path, query, limit=n, offset=off, lobe=lobe, tag=tag,
        snippet_chars=chars, include_bodies=bodies,
    )
    results = paged["results"]
    for r in results:
        if "body" in r:
            body, truncated = _clamp_body(r["body"], _SEARCH_BODY_MAX_BYTES)
            r["body"] = body
            if truncated:
                r["body_truncated"] = True
    return {
        "ok": True,
        "query": query,
        "total": paged["total"],
        "offset": off,
        "count": len(results),
        "results": results,
    }


def _run_search(
    brain_path: Path,
    query: str,
    *,
    limit: int,
    offset: int,
    lobe: str | None,
    tag: str | None,
    snippet_chars: int,
    include_bodies: int,
) -> dict[str, Any]:
    """FTS engine with substring-engine last resort (never regresses to zero
    results on an engine error)."""
    try:
        return search_brain_fts_paged(
            brain_path, query, limit=limit, offset=offset,
            lobe_filter=lobe, tag_filter=tag,
            snippet_chars=snippet_chars, include_bodies=include_bodies,
        )
    except Exception:
        return search_brain_paged(
            brain_path, query, limit=limit, offset=offset,
            lobe_filter=lobe, tag_filter=tag,
            snippet_chars=snippet_chars, include_bodies=include_bodies,
        )


# Cap on the explicitly-listed empty lobes so a query that misses on a brain
# with hundreds of lobes can't bloat the result; the count is always exact.
_MAX_NO_MATCH_LOBES_LISTED = 50


def _grouped_search(
    brain_path: Path,
    query: str,
    *,
    per_lobe: int,
    snippet_chars: int,
) -> dict[str, Any]:
    """Exact top hits per top-level lobe (partitioned ranking, not a flat
    window — a flat top-N bunches into the single most relevant lobe on a
    large corpus, missing every other lobe).

    Also reports the lobes that were swept and matched NOTHING
    (``no_match_lobes``). Without it the model can't tell "this lobe has no
    entry" from "this lobe wasn't covered", so on an "X across all lobes"
    question it re-searches each missing lobe one by one — the dominant
    tool-call fan-out. Naming the empty lobes makes absence definitive in a
    single call."""
    per_lobe = max(1, min(per_lobe, 10))
    search_ok = True
    try:
        grouped = search_brain_fts_grouped(
            brain_path, query, per_lobe=per_lobe, snippet_chars=snippet_chars,
        )
    except Exception:
        grouped = {"lobes": {}, "total": 0}
        search_ok = False
    result = {
        "ok": True,
        "query": query,
        "grouped_by_lobe": True,
        "total": grouped["total"],
        "per_lobe_limit": per_lobe,
        "lobes": grouped["lobes"],
    }
    # The full lobe set comes from the boot snapshot (its ``lobe_tags`` is keyed
    # by every top-level lobe that holds neurons; ``_lobe_of`` buckets root
    # files as "(root)", which is not a lobe, so it never appears here). No
    # snapshot ⇒ omit the hint rather than pay for a brain walk. The hint is
    # ONLY emitted when the grouped search actually ran: on a search error the
    # empty bucket set would otherwise be reported as "every lobe definitively
    # has no match", which the note tells the model to trust — a dangerous lie.
    snap = get_snapshot(brain_path) if search_ok else None
    if snap is not None:
        all_lobes = set(snap.get("lobe_tags", {}).keys())
        no_match = sorted(all_lobes - set(grouped["lobes"].keys()))
        result["lobes_searched"] = len(all_lobes)
        result["lobes_with_matches"] = len(all_lobes) - len(no_match)
        result["no_match_lobes"] = no_match[:_MAX_NO_MATCH_LOBES_LISTED]
        result["no_match_lobe_count"] = len(no_match)
        result["no_match_note"] = (
            "These lobes had no keyword match for this query. Don't repeat the "
            "same search per-lobe — but keyword search matches whole-word "
            "prefixes only, so if you expected content in one of them, try a "
            "differently-phrased query rather than treating it as definitively "
            "empty."
        )
    return result


# --- 3. read_neuron ----------------------------------------------------------


def _clamp_body(body: str, max_bytes: int | None) -> tuple[str, bool]:
    """Truncate ``body`` to at most ``max_bytes`` UTF-8 bytes for the agent
    path so one fat neuron can't dominate a turn's token budget.

    Returns ``(body, truncated)``. ``max_bytes`` falsy / ``<= 0`` → no clamp:
    the brain-explorer UI passes ``None`` so human readers always get the whole
    neuron; only the agent dispatch passes a cap.
    """
    if not max_bytes or max_bytes <= 0:
        return body, False
    encoded = body.encode("utf-8")
    if len(encoded) <= max_bytes:
        return body, False
    clipped = encoded[:max_bytes].decode("utf-8", errors="ignore")
    return (
        clipped
        + "\n\n[... neuron truncated to fit the agent's per-neuron budget; "
        "use search to pull the specific section you need ...]"
    ), True


def read_neuron_tool(
    brain_path: Path, path: str, *, max_bytes: int | None = None
) -> dict[str, Any]:
    """Read one neuron's frontmatter + body.

    ``max_bytes`` (agent path only) caps the body; the UI passes ``None``.
    """
    target = resolve_in_brain(brain_path, path)
    meta, body = read_frontmatter(target)
    deprecated = str(meta.get("status", "active")).lower() == "deprecated"
    body, truncated = _clamp_body(body, max_bytes)
    result: dict[str, Any] = {
        "ok": True,
        "path": _rel(brain_path, target),
        "frontmatter": meta,
        "body": body,
        "deprecated": deprecated,
    }
    if truncated:
        result["truncated"] = True
    return result


# --- 4. multi_read -----------------------------------------------------------


def multi_read_tool(
    brain_path: Path,
    paths: list[str],
    *,
    max_paths: int,
    max_bytes: int | None = None,
) -> dict[str, Any]:
    """Read up to ``max_paths`` neurons in one call.

    Each path is sandboxed independently — a bad path produces an
    ``{path, error}`` entry without aborting the rest of the batch.
    ``max_bytes`` (agent path only) caps each neuron body; the UI passes
    ``None``. A batch read of many full files is the usual turn-budget bomb,
    so the agent always passes a per-neuron cap.
    """
    if not isinstance(paths, list):
        return {"ok": False, "error": "paths must be a list of strings"}
    if len(paths) > max_paths:
        return {
            "ok": False,
            "error": (
                f"too many paths: got {len(paths)}, max is {max_paths}"
            ),
        }

    results: list[dict[str, Any]] = []
    for raw in paths:
        try:
            target = resolve_in_brain(brain_path, raw)
            meta, body = read_frontmatter(target)
            deprecated = str(meta.get("status", "active")).lower() == "deprecated"
            body, truncated = _clamp_body(body, max_bytes)
            entry: dict[str, Any] = {
                "path": _rel(brain_path, target),
                "frontmatter": meta,
                "body": body,
                "deprecated": deprecated,
            }
            if truncated:
                entry["truncated"] = True
            results.append(entry)
        except SandboxError as exc:
            results.append({"path": str(raw), "error": f"sandbox: {exc}"})
        except NotFoundError as exc:
            results.append({"path": str(raw), "error": f"not_found: {exc}"})
        except Exception as exc:  # pragma: no cover (defensive)
            results.append({"path": str(raw), "error": f"read_error: {exc}"})
    return {"ok": True, "results": results}


# --- 5. related --------------------------------------------------------------


def _outbound_links(brain_path: Path, target: Path) -> list[str]:
    """Resolved in-brain ``related:`` targets of one neuron (single read)."""
    target_meta, _ = read_frontmatter(target)
    outbound: list[str] = []
    seen: set[Path] = set()
    raw_related = target_meta.get("related", [])
    if isinstance(raw_related, list):
        for raw in raw_related:
            if not isinstance(raw, str):
                continue
            try:
                resolved = (target.parent / raw).resolve()
            except OSError:
                continue
            if not is_within_brain(resolved, brain_path):
                continue
            if not resolved.exists() or resolved in seen:
                continue
            seen.add(resolved)
            outbound.append(_rel(brain_path, resolved))
    return outbound


def related_tool(brain_path: Path, path: str) -> dict[str, Any]:
    """Outbound + inbound related neurons.

    Outbound: ``related:`` frontmatter on the source neuron.
    Inbound: any neuron in the brain whose ``related:`` includes the
    source path.

    With a boot snapshot both directions are dict lookups against its
    precomputed link maps — no brain re-walk. Without one (standalone
    callers, unit tests) the inbound side is the classic reverse scan.
    """
    target = resolve_in_brain(brain_path, path)
    target_resolved = target.resolve()

    snap = get_snapshot(brain_path)
    if snap is not None:
        entry = snap["by_resolved"].get(str(target_resolved))
        outbound = (
            list(entry["related"]) if entry is not None
            else _outbound_links(brain_path, target)
        )
        return {
            "ok": True,
            "path": _rel(brain_path, target),
            "outbound": outbound,
            "inbound": list(snap["inbound"].get(str(target_resolved), [])),
        }

    outbound = _outbound_links(brain_path, target)

    inbound: list[str] = []
    for neuron in neuron_files(brain_path):
        if neuron.resolve() == target_resolved:
            continue
        try:
            meta, _ = read_frontmatter(neuron)
        except Exception:
            continue
        rel = meta.get("related", [])
        if not isinstance(rel, list):
            continue
        for entry in rel:
            if not isinstance(entry, str):
                continue
            try:
                pointed = (neuron.parent / entry).resolve()
            except OSError:
                continue
            if pointed == target_resolved:
                inbound.append(_rel(brain_path, neuron))
                break

    return {
        "ok": True,
        "path": _rel(brain_path, target),
        "outbound": outbound,
        "inbound": inbound,
    }


# --- 6. recent ---------------------------------------------------------------


def recent_tool(
    brain_path: Path,
    *,
    limit: int = 10,
    lobe: str | None = None,
    include_deprecated: bool = False,
) -> dict[str, Any]:
    """Recently-updated neurons.

    Sorts by frontmatter ``updated:`` descending; falls back to file
    mtime when ``updated`` is absent. Filename is the final tie-break
    so the output is deterministic across platforms.

    Served from the boot snapshot when one is registered — no brain
    re-walk, no per-call frontmatter reads.
    """
    items: list[dict[str, Any]] = []
    snap = get_snapshot(brain_path)
    if snap is not None:
        for entry in snap["entries"]:
            rel = entry["path"]
            if lobe and not rel.startswith(lobe.rstrip("/") + "/"):
                continue
            if entry["deprecated"] and not include_deprecated:
                continue
            items.append({
                "path": rel,
                "updated": entry["updated"] or "",
                "_mtime": entry["mtime"],
                "_filename": entry["filename"],
                "deprecated": entry["deprecated"],
            })
    else:
        for neuron in neuron_files(brain_path):
            rel = _rel(brain_path, neuron)
            if lobe and not rel.startswith(lobe.rstrip("/") + "/"):
                continue
            try:
                meta, _ = read_frontmatter(neuron)
            except Exception:
                continue
            is_dep = str(meta.get("status", "active")).lower() == "deprecated"
            if is_dep and not include_deprecated:
                continue
            updated = meta.get("updated")
            try:
                mtime = neuron.stat().st_mtime
            except OSError:
                mtime = 0.0
            items.append({
                "path": rel,
                "updated": str(updated) if updated else "",
                "_mtime": mtime,
                "_filename": neuron.name,
                "deprecated": is_dep,
            })

    # Use the same recency key as wake_up's recent[] so the two "most recent"
    # surfaces agree — a non-ISO `updated:` sorts below valid dates rather than
    # floating to the top. mtime + filename remain the deterministic tie-break.
    items.sort(
        key=lambda d: (_recency_key(d["updated"] or ""), d["_mtime"], d["_filename"]),
        reverse=True,
    )
    n = max(0, min(int(limit), _MAX_RECENT_LIMIT))
    trimmed = [{k: v for k, v in item.items() if not k.startswith("_")}
               for item in items[:n]]
    return {"ok": True, "results": trimmed}


# --- 7. glossary -------------------------------------------------------------


def glossary_tool(
    brain_path: Path,
    term: str | None = None,
) -> dict[str, Any]:
    """Look up a glossary term, or list all entries."""
    glossary_path = brain_path / "glossary.md"
    if not glossary_path.exists():
        return {"ok": True, "entries": [], "term": term, "match": None,
                "alternates": []}
    try:
        _meta, body = read_frontmatter(glossary_path)
    except Exception:
        return {"ok": True, "entries": [], "term": term, "match": None,
                "alternates": []}
    pairs = parse_glossary_entries(body or "")

    if term is None:
        return {
            "ok": True,
            "entries": [{"term": t, "definition": d} for t, d in pairs],
        }

    term_norm = term.strip()
    term_low = term_norm.lower()
    match = None
    for t, d in pairs:
        if t.lower() == term_low:
            match = {"term": t, "definition": d}
            break
    candidates = [t for t, _ in pairs if t.lower() != term_low]
    alternates = get_close_matches(term_norm, candidates, n=3, cutoff=0.6)
    return {
        "ok": True,
        "term": term_norm,
        "match": match,
        "alternates": [
            {"term": t, "definition": next(d for tt, d in pairs if tt == t)}
            for t in alternates
        ],
    }


def files_tool(brain_path: Path) -> dict[str, Any]:
    """Flat listing of every neuron + glossary.md in the brain.

    Backs the chat UI's left-sidebar file tree (the same structure
    the MRI's left panel renders). Each entry has the brain-relative
    POSIX ``path``, a stem-derived ``title``, and a ``deprecated``
    flag so the UI can apply strikethrough styling without a second
    roundtrip.

    Title source matches the MRI: the filename stem with hyphens
    → spaces and Title-Case (``post-imports-users.md`` → ``Post
    Imports Users``). YAML neurons fall back to frontmatter ``title``
    if set, because YAML stems like ``openapi`` title-case poorly.
    The H1 is intentionally NOT used — compact labels read more
    consistently across a sidebar than authored H1s do.

    The frontend builds a nested folder tree from the path strings —
    no ordering / nesting work happens here.

    Served from the boot snapshot when one is registered (its ``label``
    field carries exactly these title semantics) — no brain re-walk.
    """
    files: list[dict[str, Any]] = []
    snap = get_snapshot(brain_path)
    if snap is not None:
        files = [
            {
                "path": e["path"],
                "title": e["label"],
                "deprecated": e["deprecated"],
            }
            for e in snap["entries"]
        ]
    else:
        for neuron in sorted(neuron_files(brain_path)):
            rel = _rel(brain_path, neuron)
            title = neuron.stem.replace("-", " ").title()
            deprecated = False
            try:
                meta, _body = read_frontmatter(neuron)
                deprecated = str(meta.get("status", "active")).lower() == "deprecated"
                if neuron.suffix.lower() in YAML_NEURON_SUFFIXES:
                    fm_title = meta.get("title")
                    if isinstance(fm_title, str) and fm_title.strip():
                        title = fm_title.strip()
            except Exception:
                pass
            files.append({"path": rel, "title": title, "deprecated": deprecated})

    glossary_path = brain_path / "glossary.md"
    glossary_entry = (
        {"path": "glossary.md", "title": "Glossary"}
        if glossary_path.exists() else None
    )
    return {"ok": True, "files": files, "glossary": glossary_entry}


# --- 8. lobe_overview --------------------------------------------------------


def lobe_overview_tool(
    brain_path: Path,
    lobe: str,
    *,
    budget: int,
    offset: int = 0,
) -> dict[str, Any]:
    """Lobe map.md body + per-neuron title/excerpt/tags + tag union.

    Truncates the response so ``len(json.dumps(response).encode("utf-8"))``
    is at most ``budget`` UTF-8 bytes. ``map_body`` is never truncated
    mid-string — if it alone exceeds the budget, neurons are dropped to
    ``[]`` and a ``note`` directs the agent to other tools.

    Large lobes are PAGEABLE instead of silently lossy: ``total_count``
    reports the lobe's full size, ``offset`` skips into the (path-ordered)
    neuron list, and ``next_offset`` is present whenever more neurons exist
    beyond what this response carries.

    Served from the boot snapshot when one is registered; the no-snapshot
    fallback walks ONLY the lobe subtree (never the whole brain).
    """
    lobe_dir = resolve_in_brain(brain_path, lobe)
    if not lobe_dir.is_dir():
        raise NotFoundError(f"lobe {lobe!r} not found")

    map_md = lobe_dir / "map.md"
    map_body = ""
    if map_md.exists():
        try:
            _meta, map_body = read_frontmatter(map_md)
        except Exception:
            map_body = ""

    lobe_rel = _rel(brain_path, lobe_dir).rstrip("/")
    prefix = lobe_rel + "/"
    neurons: list[dict[str, Any]] = []

    snap = get_snapshot(brain_path)
    if snap is not None:
        neurons = [
            {
                "path": e["path"],
                "title": e["title"],
                "excerpt": e["excerpt"],
                "tags": list(e["tags"]),
                "deprecated": e["deprecated"],
            }
            for e in snap["entries"]
            if e["path"].startswith(prefix)
        ]
    else:
        for neuron in sorted(neuron_files(lobe_dir)):
            rel = _rel(brain_path, neuron)
            try:
                meta, body = read_frontmatter(neuron)
            except Exception:
                continue
            is_yaml = neuron.suffix.lower() in YAML_NEURON_SUFFIXES
            if is_yaml:
                fm_title = meta.get("title")
                title = (
                    fm_title.strip()
                    if isinstance(fm_title, str) and fm_title.strip()
                    else neuron.stem.replace("-", " ").title()
                )
                excerpt = ""
            else:
                title, excerpt = extract_excerpt(neuron, body)
            tags = meta.get("tags", []) or []
            if not isinstance(tags, list):
                tags = []
            neurons.append({
                "path": rel,
                "title": title,
                "excerpt": excerpt,
                "tags": [str(t) for t in tags],
                "deprecated": str(meta.get("status", "active")).lower() == "deprecated",
            })

    total_count = len(neurons)
    offset = max(0, int(offset) if offset else 0)

    response: dict[str, Any] = {
        "ok": True,
        "lobe": lobe_rel,
        "map_body": map_body,
        "neurons": neurons[offset:],
        "tag_union": [],
        "total_count": total_count,
        "offset": offset,
    }
    out = _trim_to_budget(response, budget)
    if offset + len(out["neurons"]) < total_count:
        out["next_offset"] = offset + len(out["neurons"])
    return out


def _encoded_size(payload: dict[str, Any]) -> int:
    return len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))


def _entry_cost(neuron: dict[str, Any]) -> int:
    """Conservative encoded cost of one neuron entry: its own JSON plus its
    tags' worst-case contribution to ``tag_union`` (every tag new), plus list
    separators. Over-estimating under-fills — never overflows."""
    cost = len(json.dumps(neuron, ensure_ascii=False).encode("utf-8")) + 2
    for t in neuron.get("tags", []):
        cost += len(json.dumps(str(t), ensure_ascii=False).encode("utf-8")) + 2
    return cost


def _tag_union(neurons: list[dict[str, Any]]) -> list[str]:
    seen: list[str] = []
    seen_set: set[str] = set()
    for n in neurons:
        for t in n.get("tags", []):
            ts = str(t)
            if ts not in seen_set:
                seen_set.add(ts)
                seen.append(ts)
    return seen


def _trim_to_budget(response: dict[str, Any], budget: int) -> dict[str, Any]:
    """Keep the longest neuron prefix whose response JSON fits the budget.

    Single accumulation pass over per-entry cost estimates (the estimates are
    conservative, so the exact re-encode backstop below almost never has to
    drop more) — NOT the old pop-one-and-re-encode-everything loop, which was
    quadratic in lobe size. If even an empty ``neurons`` list overruns
    (``map_body`` alone too big), falls back to the map-body-only response
    with a ``note`` telling the agent to use ``search`` / ``recent``.
    """
    all_neurons = response["neurons"]
    base = {**response, "neurons": [], "tag_union": []}
    # Slack for the truncated/omitted_count/next_offset keys added later.
    remaining = budget - _encoded_size(base) - 64

    kept: list[dict[str, Any]] = []
    for neuron in all_neurons:
        cost = _entry_cost(neuron)
        if remaining - cost < 0:
            break
        remaining -= cost
        kept.append(neuron)

    omitted = len(all_neurons) - len(kept)
    response["neurons"] = kept
    response["tag_union"] = _tag_union(kept)
    if omitted:
        response["truncated"] = True
        response["omitted_count"] = omitted

    # Exact backstop: the estimate is conservative, so this loop is expected
    # to run zero times; it guarantees the byte contract regardless.
    while _encoded_size(response) > budget and response["neurons"]:
        response["neurons"].pop()
        omitted += 1
        response["tag_union"] = _tag_union(response["neurons"])
        response["truncated"] = True
        response["omitted_count"] = omitted

    if _encoded_size(response) > budget:
        # map_body alone exceeds the budget — keep map_body verbatim,
        # drop neurons completely, add a note pointing to other tools.
        original_omitted = omitted + len(response["neurons"])
        response["neurons"] = []
        response["tag_union"] = []
        response["truncated"] = True
        response["omitted_count"] = original_omitted
        response["note"] = "map_body exceeds budget; use search/recent for neurons"

    return response


# --- Tool dispatch table -----------------------------------------------------


TOOLS: dict[str, Any] = {
    "wake_up": wake_up_tool,
    "search": search_tool,
    "read_neuron": read_neuron_tool,
    "multi_read": multi_read_tool,
    "related": related_tool,
    "recent": recent_tool,
    "glossary": glossary_tool,
    "lobe_overview": lobe_overview_tool,
}
