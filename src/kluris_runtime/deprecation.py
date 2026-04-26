"""Read-only deprecation diagnostics.

Surfaces inconsistencies in deprecation frontmatter without modifying
anything. Used by wake-up to give the agent a count of issues so it can
decide whether to dig in before answering.
"""

from __future__ import annotations

from pathlib import Path

from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_index import is_within_brain, neuron_files


def detect_deprecation_issues(brain_path: Path) -> list[dict]:
    """Find deprecation-frontmatter inconsistencies.

    Four issue kinds are reported:

    - ``active_links_to_deprecated`` — an active neuron has ``related:``
      pointing at a deprecated neuron.
    - ``deprecated_without_replacement`` — a neuron is marked deprecated
      but has no ``replaced_by``.
    - ``replaced_by_missing`` — a ``replaced_by`` path does not resolve
      to an existing file in the brain.
    - ``replaced_by_not_active`` — a ``replaced_by`` path resolves to
      something that is not an active neuron (e.g. another deprecated
      neuron, or a ``map.md``).

    Neurons without a ``status`` field are treated as active.
    References from ``map.md`` files are ignored — maps are
    auto-generated indexes, not editorial endorsements.
    """
    issues: list[dict] = []
    neurons = neuron_files(brain_path)

    status_by_path: dict[Path, str] = {}
    meta_by_path: dict[Path, dict] = {}
    neuron_paths: set[Path] = set()
    for neuron in neurons:
        meta, _ = read_frontmatter(neuron)
        status = str(meta.get("status", "active")).lower()
        resolved = neuron.resolve()
        status_by_path[resolved] = status
        meta_by_path[resolved] = meta
        neuron_paths.add(resolved)

    for neuron in neurons:
        resolved = neuron.resolve()
        meta = meta_by_path[resolved]
        rel = str(neuron.relative_to(brain_path))

        if status_by_path[resolved] != "deprecated":
            continue

        replaced_by = meta.get("replaced_by")
        if replaced_by is None or replaced_by == "":
            issues.append({
                "kind": "deprecated_without_replacement",
                "file": rel,
            })
            continue

        if not isinstance(replaced_by, str):
            continue

        target = (neuron.parent / replaced_by).resolve()
        if not target.exists() or not is_within_brain(target, brain_path):
            issues.append({
                "kind": "replaced_by_missing",
                "file": rel,
                "target": replaced_by,
            })
            continue

        if target not in neuron_paths or status_by_path.get(target) != "active":
            issues.append({
                "kind": "replaced_by_not_active",
                "file": rel,
                "target": replaced_by,
            })

    for neuron in neurons:
        resolved = neuron.resolve()
        if status_by_path[resolved] == "deprecated":
            continue

        meta = meta_by_path[resolved]
        related = meta.get("related", [])
        if not isinstance(related, list):
            continue

        for rel_link in related:
            if not isinstance(rel_link, str):
                continue
            target = (neuron.parent / rel_link).resolve()
            if status_by_path.get(target) == "deprecated":
                issues.append({
                    "kind": "active_links_to_deprecated",
                    "source": str(neuron.relative_to(brain_path)),
                    "target": str(target.relative_to(brain_path)),
                })

    return issues
