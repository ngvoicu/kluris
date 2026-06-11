"""One-walk boot snapshot of an immutable brain.

Inside the pack container the brain never changes, yet (pre-snapshot) five
of the eight tools re-walked the whole tree and re-parsed every neuron's
frontmatter on every call — seconds per call at 10k-50k neurons. This module
walks the brain ONCE, reads each neuron's frontmatter + body exactly once,
and derives everything the read-only tools need:

- ``rows``     — searchable items for the FTS engine, byte-identical to
  :func:`kluris_runtime.search.collect_searchable` (shared shaping helpers).
- ``entries``  — per-neuron metadata (title, label, tags, updated, status,
  excerpt, mtime, outbound related links) sorted by brain-relative path, for
  ``recent`` / ``files`` / ``lobe_overview`` / ``related``.
- ``inbound``  — reverse related-link map (resolved-target → linking neurons)
  so ``related``'s inbound scan becomes a dict lookup instead of an O(N)
  re-read of the brain.
- ``preparsed`` — ``(path, meta)`` pairs for
  :func:`kluris_runtime.deprecation.detect_deprecation_issues` so wake-up's
  diagnostics reuse this walk instead of doing their own.
- ``lobe_tags`` — per-top-level-lobe most-frequent tags so wake-up can give
  the agent routing hints without per-lobe probing.

Like the FTS walk cache, the registry is opt-in: populated once at app boot,
read-only thereafter, plain data only. Standalone callers and unit tests that
never boot an app simply find no snapshot and fall back to the per-call walk.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path

from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_excerpt import extract as extract_excerpt
from kluris_runtime.neuron_index import (
    YAML_NEURON_SUFFIXES,
    is_within_brain,
    neuron_files,
)
from kluris_runtime.search import (
    neuron_searchable_item,
    non_neuron_searchable_items,
)

# Most-frequent tags reported per lobe in wake-up. Small and fixed so the
# wake_up payload stays bounded regardless of brain size.
TOP_TAGS_PER_LOBE = 8

# Cap on dropped-neuron paths sampled into the snapshot's parse-error report.
# The COUNT is always exact; only the sample list is bounded so the payload
# stays small on a brain with many malformed files.
MAX_PARSE_ERROR_SAMPLE = 20


def _rel(brain_path: Path, target: Path) -> str:
    return str(target.relative_to(brain_path)).replace("\\", "/")


def build_snapshot(brain_path: Path) -> dict:
    """Walk ``brain_path`` once and build the full snapshot dict.

    Pure read; raises nothing for individual bad neurons (they are skipped,
    matching ``collect_searchable``). Does NOT register the result — call
    :func:`register_snapshot` (or use the registry helpers) for that.
    """
    brain_root = brain_path.resolve()

    parsed: list[tuple[Path, Path, str, dict, str]] = []
    parse_error_count = 0
    parse_error_sample: list[str] = []
    for neuron in neuron_files(brain_root):
        try:
            meta, body = read_frontmatter(neuron)
        except Exception:
            # Malformed frontmatter (bad YAML, non-UTF8, etc.). The neuron is
            # dropped from EVERY surface — search, listings, lobe counts — so
            # tally it: wake-up surfaces the count so the loss is observable
            # rather than a silent under-count of the brain.
            parse_error_count += 1
            if len(parse_error_sample) < MAX_PARSE_ERROR_SAMPLE:
                parse_error_sample.append(_rel(brain_root, neuron))
            continue
        try:
            resolved = neuron.resolve()
        except (OSError, RuntimeError):
            # RuntimeError: Path.resolve() raises it (not OSError) on a symlink
            # loop. Skip the one pathological neuron rather than let it abort
            # the whole snapshot build (which would disable every cached tool).
            continue
        parsed.append((neuron, resolved, _rel(brain_root, neuron), meta, body))

    rows: list[dict] = []
    entries: list[dict] = []
    preparsed: list[tuple[Path, dict]] = []
    inbound: dict[str, list[str]] = {}
    lobe_tag_counts: dict[str, Counter] = {}

    for neuron, resolved, rel, meta, body in parsed:
        rows.append(neuron_searchable_item(brain_root, neuron, meta, body))
        preparsed.append((neuron, meta))

        is_yaml = neuron.suffix.lower() in YAML_NEURON_SUFFIXES
        tags = meta.get("tags", [])
        if not isinstance(tags, list):
            tags = []
        tags = [str(t) for t in tags]
        status = str(meta.get("status", "active")).lower()

        # Sidebar label: stem-derived Title-Case, with the YAML frontmatter-
        # title fallback — exactly files_tool's historical semantics.
        label = neuron.stem.replace("-", " ").title()
        if is_yaml:
            fm_title = meta.get("title")
            if isinstance(fm_title, str) and fm_title.strip():
                label = fm_title.strip()

        # Display title + excerpt: extract_excerpt for markdown (H1 or stem
        # fallback + first content line), YAML matches lobe_overview today
        # (frontmatter title or stem, empty excerpt).
        if is_yaml:
            title, excerpt = label, ""
        else:
            title, excerpt = extract_excerpt(neuron, body)

        updated = meta.get("updated")
        try:
            mtime = neuron.stat().st_mtime
        except OSError:
            mtime = 0.0

        # Outbound related links, resolved with related_tool's exact
        # semantics: string entries only, resolve against the neuron's dir,
        # must stay in-brain and exist, first-seen dedup.
        related_rels: list[str] = []
        seen_targets: set[Path] = set()
        raw_related = meta.get("related", [])
        if isinstance(raw_related, list):
            for raw in raw_related:
                if not isinstance(raw, str):
                    continue
                try:
                    target = (neuron.parent / raw).resolve()
                except (OSError, RuntimeError):
                    continue
                if not is_within_brain(target, brain_root):
                    continue
                if not target.exists() or target in seen_targets:
                    continue
                seen_targets.add(target)
                related_rels.append(_rel(brain_root, target))
                if target != resolved:
                    # related_tool's inbound scan never reports a neuron as
                    # inbound to itself; keep the map to the same contract.
                    inbound.setdefault(str(target), []).append(rel)

        entries.append({
            "path": rel,
            "resolved": str(resolved),
            "file_type": "yaml" if is_yaml else "markdown",
            "title": title,
            "label": label,
            "excerpt": excerpt,
            "tags": tags,
            # None ⇔ no ``updated:`` frontmatter at all — consumers that skip
            # un-dated neurons (wake-up recent) must distinguish missing from
            # an explicit empty value, which sorts but never floats up.
            "updated": str(updated) if updated is not None else None,
            "status": status,
            "deprecated": status == "deprecated",
            "mtime": mtime,
            "filename": neuron.name,
            "related": related_rels,
        })

        lobe = rel.split("/", 1)[0] if "/" in rel else ""
        if lobe:
            counter = lobe_tag_counts.setdefault(lobe, Counter())
            counter.update(tags)

    rows.extend(non_neuron_searchable_items(brain_root))
    # Path-component order, matching ``sorted(neuron_files(...))`` so
    # snapshot-served listings order identically to the walk fallbacks.
    entries.sort(key=lambda e: tuple(e["path"].split("/")))

    # Deterministic top tags: by descending count, then alphabetically.
    lobe_tags = {
        lobe: [
            tag for tag, _count in sorted(
                counter.items(), key=lambda kv: (-kv[1], kv[0])
            )[:TOP_TAGS_PER_LOBE]
        ]
        for lobe, counter in lobe_tag_counts.items()
    }

    return {
        "brain_path": brain_root,
        "rows": rows,
        "entries": entries,
        "by_resolved": {e["resolved"]: e for e in entries},
        "inbound": inbound,
        "preparsed": preparsed,
        "lobe_tags": lobe_tags,
        "parse_errors": parse_error_count,
        "parse_error_sample": parse_error_sample,
    }


# Opt-in registry, mirroring search_fts._INDEX_REGISTRY: written once at app
# boot before requests are served, read-only thereafter, plain data only.
_SNAPSHOT_REGISTRY: dict[Path, dict] = {}


def register_snapshot(brain_path: Path, snapshot: dict) -> None:
    """Register a built snapshot for ``brain_path`` (boot only)."""
    _SNAPSHOT_REGISTRY[brain_path.resolve()] = snapshot


def get_snapshot(brain_path: Path) -> dict | None:
    """Return the registered snapshot for ``brain_path``, or ``None``."""
    return _SNAPSHOT_REGISTRY.get(brain_path.resolve())


def drop_snapshot(brain_path: Path) -> None:
    """Forget a registered snapshot (explicit invalidation / test teardown)."""
    _SNAPSHOT_REGISTRY.pop(brain_path.resolve(), None)


def _clear_snapshot_registry() -> None:
    """Reset the registry (tests only)."""
    _SNAPSHOT_REGISTRY.clear()
