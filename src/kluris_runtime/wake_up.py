"""Read-only wake-up payload builder.

Produces a compact snapshot of a brain's live state — brain.md, lobes,
recent neurons, glossary, deprecation diagnostics — discovered from
the on-disk structure. The runtime version intentionally omits scaffold
metadata (``type`` / ``type_structure``); callers should use the live
``lobes[]`` payload to understand the current brain structure.
"""

from __future__ import annotations

import os
from pathlib import Path

from kluris_runtime.deprecation import detect_deprecation_issues
from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_index import (
    SKIP_DIRS,
    YAML_NEURON_SUFFIXES,
    has_yaml_opt_in_block,
)

# Matches the wake-up legacy contract: index everything except auto-
# generated maps and the brain's local config files.
_WAKE_UP_SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", "kluris.yml"}

_WAKE_UP_BRAIN_MD_MAX_BYTES = 4000


def _iter_neurons(root: Path):
    """Yield neuron files (markdown + opted-in yaml) under ``root``.

    Walks with os.walk and prunes ``SKIP_DIRS`` in place so we never descend
    into ``.git/`` — rglob would scandir ``.git/objects/*`` and race with
    git's background gc deleting loose-object dirs mid-walk (FileNotFoundError).
    Order is preserved: all ``*.md``, then ``*.yml``, then ``*.yaml``.
    """
    buckets: dict[str, list[Path]] = {".md": [], ".yml": [], ".yaml": []}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        base = Path(dirpath)
        for name in filenames:
            item = base / name
            ext = item.suffix.lower()
            if ext not in buckets:
                continue
            if name in _WAKE_UP_SKIP_FILES:
                continue
            if ext in YAML_NEURON_SUFFIXES and not has_yaml_opt_in_block(item):
                continue
            buckets[ext].append(item)
    for ext in (".md", ".yml", ".yaml"):
        yield from buckets[ext]


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


def _collect_lobes(brain_path: Path) -> list[dict]:
    """Return top-level lobes with neuron counts and descriptions."""
    lobes = []
    for child in sorted(brain_path.iterdir()):
        if not child.is_dir():
            continue
        if child.name in SKIP_DIRS or child.name.startswith("."):
            continue
        total = 0
        yaml_count = 0
        for item in _iter_neurons(child):
            total += 1
            if item.suffix.lower() in YAML_NEURON_SUFFIXES:
                yaml_count += 1
        lobes.append({
            "name": child.name,
            "description": _lobe_description(child),
            "neurons": total,
            "yaml_count": yaml_count,
        })
    return lobes


def _collect_recent(brain_path: Path, limit: int = 5) -> list[dict]:
    """Return up to ``limit`` most-recently-updated neurons, newest first."""
    candidates = []
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
    candidates.sort(key=lambda item: item["updated"], reverse=True)
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
) -> dict:
    """Build a discovered wake-up snapshot for ``brain_path``.

    Returns a dict with: ``ok``, ``name``, ``path``, ``description``,
    ``brain_md``, ``lobes``, ``total_neurons``, ``total_yaml_neurons``,
    ``recent``, ``glossary``, ``deprecation_count``, ``deprecation``.

    Does NOT include scaffold metadata (``type``, ``type_structure``).
    Callers should use the live ``lobes[]`` payload to understand the
    current brain structure.

    The brain path must exist; ``FileNotFoundError`` is the caller's
    problem (the CLI wraps it in a JSON error envelope).
    """
    if not brain_path.exists():
        raise FileNotFoundError(f"brain path does not exist: {brain_path}")

    resolved_name = name or brain_path.name

    lobes = _collect_lobes(brain_path)
    recent = _collect_recent(brain_path)
    total_neurons = sum(lobe["neurons"] for lobe in lobes)
    total_yaml_neurons = sum(lobe.get("yaml_count", 0) for lobe in lobes)
    brain_md_body = _collect_brain_md(brain_path)
    glossary_entries = _collect_glossary(brain_path)
    try:
        deprecation_issues = detect_deprecation_issues(brain_path)
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
    }
