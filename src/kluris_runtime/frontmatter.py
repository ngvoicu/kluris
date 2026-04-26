"""Read-only YAML frontmatter parsing.

Markdown files use the standard ``---`` / ``---`` YAML block. Yaml neurons
use a hash-style ``#---`` / ``#---`` block where every line inside is a
YAML comment (prefix ``# ``), so the file stays valid yaml regardless of
whether a tool knows about the block.

This module is the single source of truth for ``read_frontmatter``. The
write helpers (``write_frontmatter`` / ``update_frontmatter``) live in
``kluris.core.frontmatter`` because writing is a CLI concern, not a
runtime concern.
"""

from __future__ import annotations

import datetime
from pathlib import Path

import frontmatter
import yaml

YAML_SUFFIXES = {".yml", ".yaml"}


def _normalize_metadata(metadata: dict) -> dict:
    """Convert date objects to ISO strings for consistent handling."""
    result = {}
    for key, value in metadata.items():
        if isinstance(value, (datetime.date, datetime.datetime)):
            result[key] = value.isoformat()
        else:
            result[key] = value
    return result


def _read_yaml_neuron(path: Path) -> tuple[dict, str]:
    """Read a yaml neuron and return ``(metadata_dict, body_string)``.

    Looks for a leading ``#---`` line followed by hash-prefixed YAML
    comment lines, terminated by a closing ``#---``. The block is
    stripped from the body, parsed as yaml, and returned as the metadata
    dict. Files without the opt-in block return ``({}, full_content)``.
    """
    try:
        raw = path.read_bytes()
    except OSError:
        return {}, ""
    content = raw.decode("utf-8-sig", errors="replace")
    lines = content.splitlines(keepends=True)

    first_idx = 0
    while first_idx < len(lines) and lines[first_idx].strip() == "":
        first_idx += 1
    if first_idx >= len(lines) or lines[first_idx].rstrip("\r\n").rstrip() != "#---":
        return {}, content

    block_lines: list[str] = []
    end_idx = first_idx + 1
    found_close = False
    while end_idx < len(lines):
        line = lines[end_idx]
        stripped_line = line.rstrip("\r\n")
        if stripped_line.rstrip() == "#---":
            found_close = True
            break
        if not stripped_line.lstrip().startswith("#"):
            return {}, content
        block_lines.append(line)
        end_idx += 1

    if not found_close:
        return {}, content

    stripped_block = []
    for line in block_lines:
        if line.startswith("# "):
            stripped_block.append(line[2:])
        elif line.startswith("#"):
            stripped_block.append(line[1:])
        else:
            return {}, content
    block_text = "".join(stripped_block)

    try:
        meta = yaml.safe_load(block_text) or {}
        if not isinstance(meta, dict):
            meta = {}
    except yaml.YAMLError:
        return {}, content

    body = "".join(lines[end_idx + 1:])
    return _normalize_metadata(meta), body


def read_frontmatter(path: Path) -> tuple[dict, str]:
    """Read a markdown or yaml neuron and return ``(metadata, body)``.

    Markdown files are parsed via python-frontmatter's standard ``---``
    block. Yaml files (``.yml`` / ``.yaml``) dispatch to
    :func:`_read_yaml_neuron`, which handles the hash-style ``#---``
    opt-in block.
    """
    if path.suffix.lower() in YAML_SUFFIXES:
        return _read_yaml_neuron(path)
    post = frontmatter.load(str(path))
    return _normalize_metadata(dict(post.metadata)), post.content
