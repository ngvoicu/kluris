"""Synapse validation, bidirectional checks, orphan detection, frontmatter checks.

Read-only primitives (``_has_yaml_opt_in_block``, ``_all_neuron_files``,
``_neuron_files``, ``_is_within_brain``, ``detect_deprecation_issues``,
``SKIP_DIRS``, ``SKIP_FILES``, ``YAML_NEURON_SUFFIXES``) are re-exported
from :mod:`kluris_runtime.neuron_index` and :mod:`kluris_runtime.deprecation`.
The runtime is the single source of truth — this module only owns
write/fix flows.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter, update_frontmatter
from kluris_runtime.deprecation import (  # noqa: F401  (re-export)
    detect_deprecation_issues,
)
from kluris_runtime.neuron_index import (  # noqa: F401  (re-exports)
    SKIP_DIRS,
    SKIP_FILES,
    YAML_NEURON_SUFFIXES,
    all_neuron_files as _all_neuron_files,
    has_yaml_opt_in_block as _has_yaml_opt_in_block,
    is_within_brain as _is_within_brain,
    neuron_files as _neuron_files,
)

VALIDATE_SKIP_FILES = {"README.md"}  # Skip link validation in these (contain example links)
LINK_PATTERN = re.compile(r"\[([^\]]*)\]\(([^)]+)\)")


# Backward-compat alias: legacy callers (including tests) may still import
# `_all_md_files`. The name is misleading now that yaml is supported, but
# renaming would break every external consumer.
_all_md_files = _all_neuron_files


def parse_markdown_links(content: str) -> list[str]:
    """Extract all relative markdown link targets from content.

    Strips ``#anchor`` and ``?query`` so callers that resolve the target as
    a filesystem path (e.g. broken-link detection) don't mistake a real
    file with an anchor like ``glossary.md#jwt`` for a missing file.
    """
    targets = []
    for match in LINK_PATTERN.finditer(content):
        target = match.group(2)
        if target.startswith("http://") or target.startswith("https://"):
            continue
        target_path = target.split("#", 1)[0].split("?", 1)[0]
        if target_path:
            targets.append(target_path)
    return targets


def validate_synapses(brain_path: Path) -> list[dict]:
    """Find broken markdown links and broken related synapses.

    A link is "broken" if its resolved target does not exist OR if it resolves
    outside ``brain_path``. The latter keeps brains self-contained — a relative
    link like ``../../outside/file.md`` that happens to exist on disk is still
    invalid because it reaches outside the brain's git repo.
    """
    broken = []
    neurons = {neuron.resolve() for neuron in _neuron_files(brain_path)}
    for md_file in _all_md_files(brain_path):
        if md_file.name in VALIDATE_SKIP_FILES:
            continue
        meta, content = read_frontmatter(md_file)
        links = parse_markdown_links(content)
        for target in links:
            resolved = (md_file.parent / target).resolve()
            if not resolved.exists() or not _is_within_brain(resolved, brain_path):
                broken.append({
                    "file": str(md_file.relative_to(brain_path)),
                    "target": target,
                })

        if md_file.resolve() not in neurons:
            continue

        related = meta.get("related", [])
        if not isinstance(related, list):
            continue

        for target in related:
            if not isinstance(target, str):
                continue
            resolved = (md_file.parent / target).resolve()
            if not resolved.exists() or not _is_within_brain(resolved, brain_path):
                broken.append({
                    "file": str(md_file.relative_to(brain_path)),
                    "target": target,
                })
    return broken


def validate_bidirectional(brain_path: Path) -> list[dict]:
    """Find one-way synapses: A has B in related but B doesn't have A."""
    one_way = []
    neurons = _neuron_files(brain_path)

    # Build a map of file -> related paths (resolved)
    related_map: dict[Path, list[Path]] = {}
    for neuron in neurons:
        meta, _ = read_frontmatter(neuron)
        related = meta.get("related", [])
        if isinstance(related, list):
            resolved = []
            for rel in related:
                resolved.append((neuron.parent / rel).resolve())
            related_map[neuron.resolve()] = resolved

    # Check bidirectionality
    for source, targets in related_map.items():
        for target in targets:
            if target in related_map:
                if source not in related_map[target]:
                    one_way.append({
                        "source": str(source.relative_to(brain_path)),
                        "target": str(target.relative_to(brain_path)),
                    })
            # If target isn't a neuron with related:, that's also one-way
            elif target.exists() and target.suffix == ".md":
                target_meta, _ = read_frontmatter(target)
                target_related = target_meta.get("related", [])
                if isinstance(target_related, list):
                    resolved_back = [
                        (target.parent / r).resolve() for r in target_related
                    ]
                    if source not in resolved_back:
                        one_way.append({
                            "source": str(source.relative_to(brain_path)),
                            "target": str(target.relative_to(brain_path)),
                        })

    return one_way


def fix_bidirectional_synapses(brain_path: Path) -> int:
    """Add missing reverse related links for existing neuron targets."""
    fixed = 0
    neurons = {neuron.resolve(): neuron for neuron in _neuron_files(brain_path)}

    for source in neurons.values():
        meta, _ = read_frontmatter(source)
        related = meta.get("related", [])
        if not isinstance(related, list):
            continue

        for rel in related:
            if not isinstance(rel, str):
                continue

            target = neurons.get((source.parent / rel).resolve())
            if target is None:
                continue

            target_meta, _ = read_frontmatter(target)
            target_related = target_meta.get("related", [])
            if not isinstance(target_related, list):
                target_related = []

            resolved_back = [
                (target.parent / existing).resolve()
                for existing in target_related
                if isinstance(existing, str)
            ]
            if source.resolve() in resolved_back:
                continue

            reverse_link = Path(os.path.relpath(source, start=target.parent)).as_posix()
            update_frontmatter(target, {"related": [*target_related, reverse_link]})
            fixed += 1

    return fixed


def detect_orphans(brain_path: Path) -> list[Path]:
    """Find neurons not referenced from any map.md."""
    neurons = _neuron_files(brain_path)

    # Collect all references from map.md files
    referenced = set()
    for md_file in _all_md_files(brain_path):
        if md_file.name == "map.md" or md_file.name == "brain.md":
            _, content = read_frontmatter(md_file)
            links = parse_markdown_links(content)
            for target in links:
                resolved = (md_file.parent / target).resolve()
                referenced.add(resolved)

    orphans = []
    for neuron in neurons:
        if neuron.resolve() not in referenced:
            orphans.append(neuron.relative_to(brain_path))

    return orphans


def check_frontmatter(brain_path: Path) -> list[dict]:
    """Check neurons for missing required frontmatter fields and type errors.

    Two kinds of issues are reported, both in the same flat list:

    - ``{file, field}`` — a required field is missing.
    - ``{file, field, kind: "type"}`` — a field exists but has the wrong
      type (e.g. ``related:`` is a string instead of a list, ``replaced_by:``
      is a list instead of a string, ``tags:`` is a string instead of a list).

    Previously the other validators silently skipped malformed values, which
    meant ``dream`` could report ``healthy`` while the underlying frontmatter
    was quietly broken. This surfaces those errors as actionable warnings.

    Contract is file-type aware:
      - Markdown neurons: require ``parent``, ``created``, ``updated``.
      - Yaml neurons: require only ``updated`` (``parent`` is inferred from
        the containing lobe, ``created`` from git log at dream time).
    """
    issues = []
    for neuron in _neuron_files(brain_path):
        meta, _ = read_frontmatter(neuron)
        rel = str(neuron.relative_to(brain_path))
        is_yaml = neuron.suffix.lower() in YAML_NEURON_SUFFIXES
        if not is_yaml:
            if "parent" not in meta:
                issues.append({"file": rel, "field": "parent"})
            if "created" not in meta:
                issues.append({"file": rel, "field": "created"})
        if "updated" not in meta:
            issues.append({"file": rel, "field": "updated"})

        # Type checks for optional fields that downstream code assumes
        # have specific shapes. If these are wrong, the other validators
        # silently skip the neuron — which masks the bug.
        if "related" in meta and not isinstance(meta["related"], list):
            issues.append({"file": rel, "field": "related", "kind": "type"})
        if "tags" in meta and not isinstance(meta["tags"], list):
            issues.append({"file": rel, "field": "tags", "kind": "type"})
        if "replaced_by" in meta and meta["replaced_by"] is not None \
                and not isinstance(meta["replaced_by"], str):
            issues.append({"file": rel, "field": "replaced_by", "kind": "type"})
    return issues


def fix_missing_frontmatter(brain_path: Path) -> int:
    """Infer missing parent frontmatter for neurons from their directory."""
    fixed = 0
    for neuron in _neuron_files(brain_path):
        if neuron.parent == brain_path:
            continue

        meta, _ = read_frontmatter(neuron)
        if "parent" in meta:
            continue

        update_frontmatter(neuron, {"parent": "./map.md"})
        fixed += 1

    return fixed
