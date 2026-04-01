"""Synapse validation, bidirectional checks, orphan detection, frontmatter checks."""

from __future__ import annotations

import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter

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
    """Find broken markdown links (target file doesn't exist)."""
    broken = []
    for md_file in _all_md_files(brain_path):
        if md_file.name in VALIDATE_SKIP_FILES:
            continue
        _, content = read_frontmatter(md_file)
        links = parse_markdown_links(content)
        for target in links:
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
