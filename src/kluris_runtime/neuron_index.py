"""Read-only neuron indexing.

Walks a brain directory and yields neuron files (markdown + opted-in
yaml) under the standard kluris contract. Owns the path-sandbox check
used by every read-only retrieval tool.
"""

from __future__ import annotations

from pathlib import Path

# Directories to skip when walking the brain for any reason. Centralized
# so wake-up, search, mri, status, etc. agree on what counts.
SKIP_DIRS = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
# kluris.yml is the brain's local config; never indexed as a neuron.
SKIP_FILES = {"brain.md", "index.md", "glossary.md", "README.md", ".gitignore", "kluris.yml"}
YAML_NEURON_SUFFIXES = {".yml", ".yaml"}


def has_yaml_opt_in_block(path: Path) -> bool:
    """Return True if a yaml file has a complete ``#---`` block at top.

    The gate verifies BOTH the opening sentinel AND a matching closing
    sentinel, AND that every line in between is a comment. A file with
    only an opening ``#---`` and no closing is rejected.
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
    while idx < len(lines) and lines[idx].strip() == "":
        idx += 1
    if idx >= len(lines) or lines[idx].rstrip() != "#---":
        return False
    idx += 1
    while idx < len(lines):
        line = lines[idx]
        if line.rstrip() == "#---":
            return True
        if not line.lstrip().startswith("#"):
            return False
        idx += 1
    return False


def all_neuron_files(brain_path: Path) -> list[Path]:
    """Collect all neuron source files (markdown + opted-in yaml).

    Walks the brain once per suffix, filtering out tooling / hidden
    dirs and the ``SKIP_FILES`` set. Yaml files must declare themselves
    via a ``#---`` block.
    """
    files: list[Path] = []
    for item in brain_path.rglob("*.md"):
        if any(part in SKIP_DIRS for part in item.parts):
            continue
        if any(part.startswith(".") for part in item.parts[:-1]):
            continue
        files.append(item)
    for suffix in ("*.yml", "*.yaml"):
        for item in brain_path.rglob(suffix):
            if any(part in SKIP_DIRS for part in item.parts):
                continue
            if any(part.startswith(".") for part in item.parts[:-1]):
                continue
            if item.name in SKIP_FILES:
                continue
            if not has_yaml_opt_in_block(item):
                continue
            files.append(item)
    return files


def neuron_files(brain_path: Path) -> list[Path]:
    """Collect neuron files, excluding auto-generated indexes.

    Drops ``map.md`` and any name in :data:`SKIP_FILES` (e.g.
    ``brain.md``, ``glossary.md``, ``kluris.yml``).
    """
    return [
        f
        for f in all_neuron_files(brain_path)
        if f.name not in SKIP_FILES and f.name != "map.md"
    ]


def is_within_brain(resolved: Path, brain_root: Path) -> bool:
    """Return True if ``resolved`` is inside ``brain_root`` (inclusive).

    Uses the parents chain rather than ``Path.is_relative_to`` so it
    behaves consistently across Python 3.10+ and on symlink-heavy
    filesystems.
    """
    try:
        brain_resolved = brain_root.resolve()
    except OSError:
        return False
    return resolved == brain_resolved or brain_resolved in resolved.parents
