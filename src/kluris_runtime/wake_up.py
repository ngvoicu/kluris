"""Read-only wake-up payload builder.

Produces a compact snapshot of a brain's live state — brain.md, lobes,
recent neurons, glossary, deprecation diagnostics — discovered from
the on-disk structure. The runtime version intentionally omits scaffold
metadata (``type`` / ``type_structure``); callers should use the live
``lobes[]`` payload to understand the current brain structure.
"""

from __future__ import annotations

import datetime
from pathlib import Path

from kluris_runtime.deprecation import detect_deprecation_issues
from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_index import (
    SKIP_DIRS,
    YAML_NEURON_SUFFIXES,
    neuron_files,
)

_WAKE_UP_BRAIN_MD_MAX_BYTES = 4000


def _iter_neurons(root: Path):
    """Yield neuron files (markdown + opted-in yaml) under ``root``.

    Delegates to :func:`kluris_runtime.neuron_index.neuron_files` so there is
    exactly ONE definition of "what counts as a neuron". The walkers had
    drifted: this one did not prune nested dot-directories, so a neuron under
    ``lobe/.archive/`` was counted by wake-up yet invisible to search / files
    / related — an inconsistency that grows with real-world brains.
    """
    yield from neuron_files(root)


def _lobe_description(lobe_path: Path) -> str:
    """Read a lobe's description from its map.md.

    Checks frontmatter ``description`` first. Falls back to the first
    non-heading, non-navigation body line so legacy map.md files still
    surface a description in wake-up.
    """
    map_file = lobe_path / "map.md"
    if not map_file.exists():
        return ""
    try:
        meta, content = read_frontmatter(map_file)
        desc = meta.get("description", "")
        if isinstance(desc, str) and desc.strip():
            return desc.strip()
    except Exception:
        try:
            content = map_file.read_text(encoding="utf-8")
        except OSError:
            return ""
    title_seen = False
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("# "):
            title_seen = True
            continue
        if not title_seen:
            continue
        if line.startswith(("up ", "sideways ", "## ", "- [")):
            continue
        return line
    return ""


def _collect_lobes(brain_path: Path, snapshot: dict | None = None) -> list[dict]:
    """Return top-level lobes with neuron counts and descriptions.

    With a boot ``snapshot`` (see :mod:`kluris_runtime.snapshot`) the counts
    come from its single walk instead of re-walking each lobe subtree, and
    each lobe gains ``top_tags`` — its most frequent neuron tags — so the
    agent can route a query to the right lobe without probing them one by
    one. Lobes that exist on disk but hold no neurons still appear (count 0).
    """
    md_counts: dict[str, int] = {}
    yaml_counts: dict[str, int] = {}
    if snapshot is not None:
        for entry in snapshot["entries"]:
            rel = entry["path"]
            if "/" not in rel:
                continue
            lobe = rel.split("/", 1)[0]
            if entry["file_type"] == "yaml":
                yaml_counts[lobe] = yaml_counts.get(lobe, 0) + 1
            else:
                md_counts[lobe] = md_counts.get(lobe, 0) + 1

    lobes = []
    for child in sorted(brain_path.iterdir()):
        if not child.is_dir():
            continue
        if child.name in SKIP_DIRS or child.name.startswith("."):
            continue
        if snapshot is not None:
            yaml_count = yaml_counts.get(child.name, 0)
            total = md_counts.get(child.name, 0) + yaml_count
        else:
            total = 0
            yaml_count = 0
            for item in _iter_neurons(child):
                total += 1
                if item.suffix.lower() in YAML_NEURON_SUFFIXES:
                    yaml_count += 1
        lobe: dict = {
            "name": child.name,
            "description": _lobe_description(child),
            "neurons": total,
            "yaml_count": yaml_count,
        }
        if snapshot is not None:
            lobe["top_tags"] = snapshot["lobe_tags"].get(child.name, [])
        lobes.append(lobe)
    return lobes


def _recency_key(updated: str) -> tuple[int, str]:
    """Sort key for the 'recent' list.

    ISO dates/datetimes are already lexicographically chronological, so valid
    values keep their raw string (preserving intra-day precision) and sort
    identically to a plain string sort. Values whose date prefix is NOT ISO
    sort BELOW all valid ones instead of interleaving unpredictably.
    """
    try:
        datetime.date.fromisoformat(updated[:10])
        return (1, updated)
    except ValueError:
        return (0, updated)


def _collect_recent(
    brain_path: Path, limit: int = 5, snapshot: dict | None = None
) -> list[dict]:
    """Return up to ``limit`` most-recently-updated neurons, newest first.

    With a boot ``snapshot`` the candidates come from its single walk (no
    per-call frontmatter re-reads); the shape and ordering are identical.
    """
    candidates = []
    if snapshot is not None:
        candidates = [
            {
                "path": entry["path"],
                "updated": entry["updated"],
                "file_type": entry["file_type"],
            }
            for entry in snapshot["entries"]
            if entry["updated"] is not None
        ]
    else:
        for item in _iter_neurons(brain_path):
            try:
                meta, _ = read_frontmatter(item)
            except Exception:
                continue
            updated = meta.get("updated")
            if updated is None:
                continue
            file_type = "yaml" if item.suffix.lower() in YAML_NEURON_SUFFIXES else "markdown"
            candidates.append({
                "path": str(item.relative_to(brain_path)).replace("\\", "/"),
                "updated": str(updated),
                "file_type": file_type,
            })
    # Break ties on `updated` by path so the order is deterministic and
    # identical across the snapshot path (entries already path-sorted) and the
    # walk fallback (raw os.walk order). Path descending keeps the stable
    # recency-desc intent under reverse=True; recent_tool uses mtime+filename,
    # but wake_up has neither here, so path is the portable tie-break.
    candidates.sort(
        key=lambda item: (_recency_key(item["updated"]), item["path"]),
        reverse=True,
    )
    return candidates[:limit]


def _collect_brain_md(brain_path: Path) -> str:
    """Return the body of brain.md (frontmatter stripped), capped to bound payload."""
    brain_md = brain_path / "brain.md"
    if not brain_md.exists():
        return ""
    try:
        _meta, body = read_frontmatter(brain_md)
    except Exception:
        return ""
    if not isinstance(body, str):
        return ""
    body = body.strip()
    if len(body.encode("utf-8")) > _WAKE_UP_BRAIN_MD_MAX_BYTES:
        truncated = body.encode("utf-8")[:_WAKE_UP_BRAIN_MD_MAX_BYTES]
        body = truncated.decode("utf-8", errors="ignore") + "\n\n[... truncated]"
    return body


def _collect_glossary(brain_path: Path) -> list[dict]:
    """Parse glossary.md and return ``[{term, definition}]`` entries."""
    from kluris_runtime.search import parse_glossary_entries

    glossary = brain_path / "glossary.md"
    if not glossary.exists():
        return []
    try:
        _meta, body = read_frontmatter(glossary)
    except Exception:
        return []
    if not isinstance(body, str):
        return []
    return [{"term": term, "definition": definition}
            for term, definition in parse_glossary_entries(body)]


def build_payload(
    brain_path: Path,
    *,
    name: str | None = None,
    description: str = "",
    snapshot: dict | None = None,
) -> dict:
    """Build a discovered wake-up snapshot for ``brain_path``.

    Returns a dict with: ``ok``, ``name``, ``path``, ``description``,
    ``brain_md``, ``lobes``, ``total_neurons``, ``total_yaml_neurons``,
    ``recent``, ``glossary``, ``deprecation_count``, ``deprecation``,
    ``parse_errors``.

    Does NOT include scaffold metadata (``type``, ``type_structure``).
    Callers should use the live ``lobes[]`` payload to understand the
    current brain structure.

    ``snapshot`` (a :func:`kluris_runtime.snapshot.build_snapshot` result)
    makes the whole payload come from ONE brain walk instead of three —
    recency, deprecation diagnostics, and lobe counts all reuse its parsed
    frontmatter — and enriches each lobe with ``top_tags`` routing hints.

    The brain path must exist; ``FileNotFoundError`` is the caller's
    problem (the CLI wraps it in a JSON error envelope).
    """
    if not brain_path.exists():
        raise FileNotFoundError(f"brain path does not exist: {brain_path}")

    resolved_name = name or brain_path.name

    lobes = _collect_lobes(brain_path, snapshot)
    recent = _collect_recent(brain_path, snapshot=snapshot)
    total_neurons = sum(lobe["neurons"] for lobe in lobes)
    total_yaml_neurons = sum(lobe.get("yaml_count", 0) for lobe in lobes)
    brain_md_body = _collect_brain_md(brain_path)
    glossary_entries = _collect_glossary(brain_path)
    try:
        deprecation_issues = detect_deprecation_issues(
            brain_path,
            preparsed=snapshot["preparsed"] if snapshot is not None else None,
        )
    except Exception:
        deprecation_issues = []

    return {
        "ok": True,
        "name": resolved_name,
        "path": str(brain_path),
        "description": description or "",
        "brain_md": brain_md_body,
        "lobes": lobes,
        "total_neurons": total_neurons,
        "total_yaml_neurons": total_yaml_neurons,
        "recent": recent,
        "glossary": glossary_entries,
        "deprecation_count": len(deprecation_issues),
        "deprecation": deprecation_issues,
        # Neurons dropped at the boot walk because their frontmatter wouldn't
        # parse — they're invisible to search/listings, so report the count so
        # an operator can tell "the brain has N files" from "N are searchable".
        # 0 when no snapshot is available (the per-call walk doesn't tally).
        "parse_errors": (
            snapshot.get("parse_errors", 0) if snapshot is not None else 0
        ),
    }
