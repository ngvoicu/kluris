"""Synapse validation, bidirectional checks, orphan detection, frontmatter checks."""

from __future__ import annotations

import os
import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter, update_frontmatter

# Directories to skip when walking the brain for any reason. This must be a
# superset of anything used by wake-up, mri, status, dream, etc. Keeping it
# centralized here prevents the "each command walks the brain differently"
# bug that caused status to count markdown under .github/workflows.
SKIP_DIRS = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
# kluris.yml is the brain's local config at the root. It must NEVER be indexed
# as a neuron — defense in depth alongside the opt-in `#---` block gate for
# yaml files (see `_has_yaml_opt_in_block`).
SKIP_FILES = {"brain.md", "index.md", "glossary.md", "README.md", ".gitignore", "kluris.yml"}
VALIDATE_SKIP_FILES = {"README.md"}  # Skip link validation in these (contain example links)
LINK_PATTERN = re.compile(r"\[([^\]]*)\]\(([^)]+)\)")
YAML_NEURON_SUFFIXES = {".yml", ".yaml"}


def _has_yaml_opt_in_block(path: Path) -> bool:
    """Return True if a yaml file has a complete `#---` / `#---` block at top.

    Opt-in invariant: yaml files are only indexed as kluris neurons when they
    declare themselves via a hash-style frontmatter block. This protects
    arbitrary yaml files (CI configs, k8s manifests) from being picked up,
    and makes the agent's authoring intent explicit.

    The gate verifies BOTH the opening sentinel AND a matching closing
    sentinel, AND that every line in between is a comment (starts with `#`).
    A file with only an opening `#---` and no closing is rejected — it would
    be a parse mismatch with `_read_yaml_neuron`, which needs both sentinels
    to extract metadata.

    Reads up to the first 4 KB so a reasonable frontmatter block (dozens of
    lines) is always covered. The sentinel must be at the top of the file
    (only whitespace-only lines allowed before the opening `#---`). Handles
    a leading UTF-8 BOM.
    """
    try:
        with path.open("rb") as f:
            head = f.read(4096)
    except OSError:
        return False
    try:
        text = head.decode("utf-8-sig", errors="replace")
    except Exception:
        return False
    lines = text.splitlines()
    idx = 0
    # Skip leading blank lines
    while idx < len(lines) and lines[idx].strip() == "":
        idx += 1
    # First non-blank line must be the opening sentinel
    if idx >= len(lines) or lines[idx].rstrip() != "#---":
        return False
    idx += 1
    # Walk to the closing sentinel; every line in between must be a comment
    while idx < len(lines):
        line = lines[idx]
        if line.rstrip() == "#---":
            return True
        if not line.lstrip().startswith("#"):
            return False
        idx += 1
    # Ran off the end before finding the closing sentinel — malformed
    return False


def _all_neuron_files(brain_path: Path) -> list[Path]:
    """Collect all neuron source files in the brain (markdown + opted-in yaml).

    Walks the brain once per suffix, filtering out tooling / hidden dirs
    and the SKIP_FILES set. For yaml files, an additional opt-in gate
    (`_has_yaml_opt_in_block`) ensures only files that declare themselves
    via a `#---` block are included.

    Skips any directory whose name starts with ``.`` -- covers ad-hoc
    editor state dirs we haven't explicitly enumerated.
    """
    files: list[Path] = []
    # Markdown
    for item in brain_path.rglob("*.md"):
        if any(part in SKIP_DIRS for part in item.parts):
            continue
        if any(part.startswith(".") for part in item.parts[:-1]):
            continue
        files.append(item)
    # Yaml (opt-in only)
    for suffix in ("*.yml", "*.yaml"):
        for item in brain_path.rglob(suffix):
            if any(part in SKIP_DIRS for part in item.parts):
                continue
            if any(part.startswith(".") for part in item.parts[:-1]):
                continue
            # Belt: explicit filename skip (kluris.yml et al.) happens here
            # too so even an adversarial kluris.yml with a `#---` block is
            # still rejected.
            if item.name in SKIP_FILES:
                continue
            if not _has_yaml_opt_in_block(item):
                continue
            files.append(item)
    return files


# Backward-compat alias: legacy callers (including tests) may still import
# `_all_md_files`. The name is misleading now that yaml is supported, but
# renaming would break every external consumer.
_all_md_files = _all_neuron_files


def _neuron_files(brain_path: Path) -> list[Path]:
    """Collect neuron files (markdown + opted-in yaml), excluding auto-generated
    and skip-listed files like map.md, brain.md, index.md, glossary.md,
    kluris.yml, etc.
    """
    neurons = []
    for f in _all_neuron_files(brain_path):
        if f.name in SKIP_FILES or f.name == "map.md":
            continue
        neurons.append(f)
    return neurons


def _is_within_brain(resolved: Path, brain_root: Path) -> bool:
    """Return True if ``resolved`` is inside ``brain_root`` (including equal).

    Uses the parents chain rather than is_relative_to so it behaves
    consistently across Python 3.10+ and on symlink-heavy filesystems.
    """
    try:
        brain_resolved = brain_root.resolve()
    except OSError:
        return False
    return resolved == brain_resolved or brain_resolved in resolved.parents


def parse_markdown_links(content: str) -> list[str]:
    """Extract all relative markdown link targets from content."""
    targets = []
    for match in LINK_PATTERN.finditer(content):
        target = match.group(2)
        if not target.startswith("http://") and not target.startswith("https://"):
            targets.append(target)
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
        if not target.exists() or not _is_within_brain(target, brain_path):
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
