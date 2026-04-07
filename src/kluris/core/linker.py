"""Synapse validation, bidirectional checks, orphan detection, frontmatter checks."""

from __future__ import annotations

import os
import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter, update_frontmatter

SKIP_DIRS = {".git"}
SKIP_FILES = {"brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}
VALIDATE_SKIP_FILES = {"README.md"}  # Skip link validation in these (contain example links)
LINK_PATTERN = re.compile(r"\[([^\]]*)\]\(([^)]+)\)")


def _all_md_files(brain_path: Path) -> list[Path]:
    """Collect all .md files in the brain, excluding .git."""
    files = []
    for item in brain_path.rglob("*.md"):
        if any(part in SKIP_DIRS for part in item.parts):
            continue
        files.append(item)
    return files


def _neuron_files(brain_path: Path) -> list[Path]:
    """Collect neuron .md files (not map.md, brain.md, index.md, etc.)."""
    neurons = []
    for f in _all_md_files(brain_path):
        if f.name in SKIP_FILES or f.name == "map.md":
            continue
        neurons.append(f)
    return neurons


def parse_markdown_links(content: str) -> list[str]:
    """Extract all relative markdown link targets from content."""
    targets = []
    for match in LINK_PATTERN.finditer(content):
        target = match.group(2)
        if not target.startswith("http://") and not target.startswith("https://"):
            targets.append(target)
    return targets


def validate_synapses(brain_path: Path) -> list[dict]:
    """Find broken markdown links and broken related synapses."""
    broken = []
    neurons = {neuron.resolve() for neuron in _neuron_files(brain_path)}
    for md_file in _all_md_files(brain_path):
        if md_file.name in VALIDATE_SKIP_FILES:
            continue
        meta, content = read_frontmatter(md_file)
        links = parse_markdown_links(content)
        for target in links:
            resolved = (md_file.parent / target).resolve()
            if not resolved.exists():
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
            if not resolved.exists():
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
    """Check neurons for missing required frontmatter fields."""
    issues = []
    for neuron in _neuron_files(brain_path):
        meta, _ = read_frontmatter(neuron)
        rel = str(neuron.relative_to(brain_path))
        if "parent" not in meta:
            issues.append({"file": rel, "field": "parent"})
        if "created" not in meta:
            issues.append({"file": rel, "field": "created"})
        if "updated" not in meta:
            issues.append({"file": rel, "field": "updated"})
    return issues


def detect_deprecation_issues(brain_path: Path) -> list[dict]:
    """Find deprecation-frontmatter inconsistencies.

    Four issue kinds are reported:

    - `active_links_to_deprecated`: an active neuron has `related:` pointing
      at a deprecated neuron. The active neuron probably needs updating to
      point at the replacement.
    - `deprecated_without_replacement`: a neuron is marked deprecated but has
      no `replaced_by`, so readers have no migration path.
    - `replaced_by_missing`: a `replaced_by` path doesn't resolve to an
      existing file in the brain.
    - `replaced_by_not_active`: a `replaced_by` path resolves to something
      that is not an active neuron — either another deprecated neuron
      (dead migration chain) or a non-neuron file like `map.md`.

    Neurons without a `status` field are treated as active.
    References from `map.md` files are ignored — maps are auto-generated
    indexes, not editorial endorsements.
    """
    issues: list[dict] = []
    neurons = _neuron_files(brain_path)

    # Build a status map: {resolved_path: "active"|"deprecated"}
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

    # Per-neuron checks: deprecated_without_replacement, replaced_by_missing,
    # replaced_by_not_active
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
        if not target.exists():
            issues.append({
                "kind": "replaced_by_missing",
                "file": rel,
                "target": replaced_by,
            })
            continue

        # Target exists but is not an active neuron: either it's a non-neuron
        # file (map.md, brain.md, README.md, glossary.md) or it's another
        # deprecated neuron (dead migration chain).
        if target not in neuron_paths or status_by_path.get(target) != "active":
            issues.append({
                "kind": "replaced_by_not_active",
                "file": rel,
                "target": replaced_by,
            })

    # Cross-neuron check: active neurons linking to deprecated ones via
    # `related:` frontmatter.
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
