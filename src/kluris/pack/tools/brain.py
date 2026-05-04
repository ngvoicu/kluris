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
import os
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
    search_brain,
)
from kluris_runtime.wake_up import build_payload


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
    return str(target.relative_to(brain_path)).replace("\\", "/")


# --- 1. wake_up --------------------------------------------------------------


def wake_up_tool(brain_path: Path) -> dict[str, Any]:
    """Live snapshot of the brain.

    Wraps :func:`kluris_runtime.wake_up.build_payload`. Does NOT
    include scaffold metadata (``type`` / ``type_structure``).
    """
    return build_payload(brain_path)


# --- 2. search ---------------------------------------------------------------


def search_tool(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe: str | None = None,
    tag: str | None = None,
) -> dict[str, Any]:
    """Lexical search across neurons + glossary + brain.md."""
    if not isinstance(query, str) or not query.strip():
        return {"ok": False, "error": "query must be a non-empty string"}
    results = search_brain(
        brain_path,
        query,
        limit=int(limit) if limit else 10,
        lobe_filter=lobe,
        tag_filter=tag,
    )
    return {
        "ok": True,
        "query": query,
        "total": len(results),
        "results": results,
    }


# --- 3. read_neuron ----------------------------------------------------------


def read_neuron_tool(brain_path: Path, path: str) -> dict[str, Any]:
    """Read one neuron's frontmatter + body."""
    target = resolve_in_brain(brain_path, path)
    meta, body = read_frontmatter(target)
    deprecated = str(meta.get("status", "active")).lower() == "deprecated"
    return {
        "ok": True,
        "path": _rel(brain_path, target),
        "frontmatter": meta,
        "body": body,
        "deprecated": deprecated,
    }


# --- 4. multi_read -----------------------------------------------------------


def multi_read_tool(
    brain_path: Path,
    paths: list[str],
    *,
    max_paths: int,
) -> dict[str, Any]:
    """Read up to ``max_paths`` neurons in one call.

    Each path is sandboxed independently — a bad path produces an
    ``{path, error}`` entry without aborting the rest of the batch.
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
            results.append({
                "path": _rel(brain_path, target),
                "frontmatter": meta,
                "body": body,
                "deprecated": deprecated,
            })
        except SandboxError as exc:
            results.append({"path": str(raw), "error": f"sandbox: {exc}"})
        except NotFoundError as exc:
            results.append({"path": str(raw), "error": f"not_found: {exc}"})
        except Exception as exc:  # pragma: no cover (defensive)
            results.append({"path": str(raw), "error": f"read_error: {exc}"})
    return {"ok": True, "results": results}


# --- 5. related --------------------------------------------------------------


def related_tool(brain_path: Path, path: str) -> dict[str, Any]:
    """Outbound + inbound related neurons.

    Outbound: ``related:`` frontmatter on the source neuron.
    Inbound: any neuron in the brain whose ``related:`` includes the
    source path (reverse scan).

    Cycles are naturally bounded — we walk a finite set of neuron
    files exactly once per call.
    """
    target = resolve_in_brain(brain_path, path)
    target_resolved = target.resolve()
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
    """
    items: list[dict[str, Any]] = []
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

    items.sort(
        key=lambda d: (d["updated"] or "", d["_mtime"], d["_filename"]),
        reverse=True,
    )
    trimmed = [{k: v for k, v in item.items() if not k.startswith("_")}
               for item in items[: max(0, int(limit))]]
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
    """
    files: list[dict[str, Any]] = []
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
) -> dict[str, Any]:
    """Lobe map.md body + per-neuron title/excerpt/tags + tag union.

    Truncates the response so ``len(json.dumps(response).encode("utf-8"))``
    is at most ``budget`` UTF-8 bytes. Drops trailing neurons one at a
    time and re-encodes after each drop. ``map_body`` is never
    truncated mid-string — if it alone exceeds the budget, neurons are
    dropped to ``[]`` and a ``note`` directs the agent to other tools.
    """
    if not isinstance(lobe, str) or not lobe:
        raise NotFoundError("lobe must be a non-empty string")

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
    tags_seen: list[str] = []
    tag_set: set[str] = set()
    for neuron in sorted(neuron_files(brain_path)):
        rel = _rel(brain_path, neuron)
        if not rel.startswith(prefix):
            continue
        try:
            meta, body = read_frontmatter(neuron)
        except Exception:
            continue
        is_yaml = neuron.suffix.lower() in YAML_NEURON_SUFFIXES
        if is_yaml:
            fm_title = meta.get("title")
            title = (
                fm_title.strip() if isinstance(fm_title, str) and fm_title.strip()
                else neuron.stem.replace("-", " ").title()
            )
            excerpt = ""
        else:
            title, excerpt = extract_excerpt(neuron, body)
        tags = meta.get("tags", []) or []
        if not isinstance(tags, list):
            tags = []
        for t in tags:
            t_str = str(t)
            if t_str not in tag_set:
                tag_set.add(t_str)
                tags_seen.append(t_str)
        neurons.append({
            "path": rel,
            "title": title,
            "excerpt": excerpt,
            "tags": list(tags),
            "deprecated": str(meta.get("status", "active")).lower() == "deprecated",
        })

    response: dict[str, Any] = {
        "ok": True,
        "lobe": lobe_rel,
        "map_body": map_body,
        "neurons": neurons,
        "tag_union": tags_seen,
    }
    return _trim_to_budget(response, budget)


def _encoded_size(payload: dict[str, Any]) -> int:
    return len(json.dumps(payload, ensure_ascii=False).encode("utf-8"))


def _trim_to_budget(response: dict[str, Any], budget: int) -> dict[str, Any]:
    """Drop trailing neurons until ``response`` JSON fits the budget.

    Re-encodes after each drop so the assertion is exact, not an
    estimate. If even an empty ``neurons`` list overruns, falls back
    to the ``map_body``-only response with a ``note`` telling the
    agent to use ``search`` / ``recent``.
    """
    omitted = 0
    while _encoded_size(response) > budget and response["neurons"]:
        response["neurons"].pop()
        omitted += 1
        # Recompute tag_union from the remaining neurons so we don't
        # advertise tags that came only from dropped entries.
        seen: list[str] = []
        seen_set: set[str] = set()
        for n in response["neurons"]:
            for t in n.get("tags", []):
                ts = str(t)
                if ts not in seen_set:
                    seen_set.add(ts)
                    seen.append(ts)
        response["tag_union"] = seen
    if omitted:
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
