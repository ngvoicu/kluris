"""YAML frontmatter read/write operations using python-frontmatter.

Markdown files use the standard `---` / `---` YAML block. Yaml neurons use
a hash-style `#---` / `#---` block where every line inside is a YAML
comment (prefix `# `), so the file stays valid yaml regardless of whether
a tool knows about the block. See `.specs/yaml-neurons/SPEC.md` for the
rationale.
"""

from __future__ import annotations

from pathlib import Path

import frontmatter
import yaml

YAML_SUFFIXES = {".yml", ".yaml"}


def _normalize_metadata(metadata: dict) -> dict:
    """Convert date objects to ISO strings for consistent handling."""
    import datetime
    result = {}
    for key, value in metadata.items():
        if isinstance(value, (datetime.date, datetime.datetime)):
            result[key] = value.isoformat()
        else:
            result[key] = value
    return result


def _read_yaml_neuron(path: Path) -> tuple[dict, str]:
    """Read a yaml neuron file and return (metadata_dict, body_string).

    Looks for a leading `#---` line followed by hash-prefixed YAML comment
    lines, terminated by a closing `#---`. The block is stripped from the
    body, parsed as yaml, and returned as the metadata dict. Files without
    the opt-in block return `({}, full_content)` — this is the opt-out path.

    Reads the file in binary mode and decodes with utf-8-sig to preserve
    line endings exactly (no universal-newlines `\\r\\n` → `\\n` normalization)
    and to transparently strip a leading UTF-8 BOM if present. The body
    string returned is byte-for-byte the file text after the closing
    sentinel, so callers (including the writer) can round-trip unchanged.
    """
    try:
        raw = path.read_bytes()
    except OSError:
        return {}, ""
    # utf-8-sig transparently strips a leading BOM if present.
    content = raw.decode("utf-8-sig", errors="replace")

    # Split keeping line endings so the body can be reassembled byte-for-byte.
    lines = content.splitlines(keepends=True)

    # Fast reject: first non-empty line must be `#---` (the opt-in sentinel).
    first_idx = 0
    while first_idx < len(lines) and lines[first_idx].strip() == "":
        first_idx += 1
    if first_idx >= len(lines) or lines[first_idx].rstrip("\r\n").rstrip() != "#---":
        return {}, content

    # Walk until the closing `#---`. Every line inside MUST be a hash
    # comment (`#`-prefixed) — non-comment content invalidates the block
    # per the opt-in contract.
    block_lines: list[str] = []
    end_idx = first_idx + 1
    found_close = False
    while end_idx < len(lines):
        line = lines[end_idx]
        stripped_line = line.rstrip("\r\n")
        if stripped_line.rstrip() == "#---":
            found_close = True
            break
        # Strict: in-block lines must be comments. Any other content voids
        # the block and makes this file opt-out.
        if not stripped_line.lstrip().startswith("#"):
            return {}, content
        block_lines.append(line)
        end_idx += 1

    if not found_close:
        # No closing sentinel — malformed, treat as opt-out.
        return {}, content

    # Strip the leading `# ` (or `#`) from each block line and parse as yaml.
    stripped_block = []
    for line in block_lines:
        if line.startswith("# "):
            stripped_block.append(line[2:])
        elif line.startswith("#"):
            stripped_block.append(line[1:])
        else:
            # Should be impossible given the strict check above, but
            # be defensive: any non-comment line inside the block voids it.
            return {}, content
    block_text = "".join(stripped_block)

    try:
        meta = yaml.safe_load(block_text) or {}
        if not isinstance(meta, dict):
            meta = {}
    except yaml.YAMLError:
        return {}, content

    # Body is everything after the closing `#---`, byte-preserving.
    body = "".join(lines[end_idx + 1:])
    return _normalize_metadata(meta), body


def read_frontmatter(path: Path) -> tuple[dict, str]:
    """Read a markdown or yaml neuron and return (metadata_dict, body_string).

    Markdown files are parsed via python-frontmatter's standard `---` block.
    Yaml files (`.yml` / `.yaml`) dispatch to `_read_yaml_neuron`, which
    handles the hash-style `#---` opt-in block.
    """
    if path.suffix.lower() in YAML_SUFFIXES:
        return _read_yaml_neuron(path)
    post = frontmatter.load(str(path))
    return _normalize_metadata(dict(post.metadata)), post.content


def _write_yaml_neuron(path: Path, metadata: dict, body: str) -> None:
    """Write a yaml neuron with a hash-style `#---` frontmatter block.

    The body is written verbatim — no yaml round-trip, no comment loss, no
    key reordering. The block is constructed by dumping the metadata dict
    via `yaml.safe_dump`, then prefixing every line with `# `.

    Uses `write_bytes` (not `write_text`) so the body's line endings are
    preserved exactly: Python's text-mode `write_text` translates `\\n`
    to `os.linesep` on Windows, which would corrupt a CRLF body
    (`\\r\\n` → `\\r\\r\\n`) or silently lose a LF body on a CRLF-native
    platform. Binary write is byte-for-byte.
    """
    # yaml.safe_dump returns a trailing newline; strip it before splitting.
    dumped = yaml.safe_dump(metadata, sort_keys=True, default_flow_style=False)
    prefixed_lines = []
    for line in dumped.rstrip("\n").splitlines():
        prefixed_lines.append(f"# {line}" if line else "#")
    block = "#---\n" + "\n".join(prefixed_lines) + "\n#---\n"
    path.write_bytes((block + body).encode("utf-8"))


def write_frontmatter(path: Path, metadata: dict, content: str) -> None:
    """Write a markdown or yaml neuron with frontmatter + content.

    Yaml files get a `#---` hash block via `_write_yaml_neuron`, which
    preserves the body byte-for-byte. Markdown files use the standard
    python-frontmatter `---` block.
    """
    if path.suffix.lower() in YAML_SUFFIXES:
        _write_yaml_neuron(path, metadata, content)
        return
    post = frontmatter.Post(content, **metadata)
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")


def update_frontmatter(
    path: Path,
    updates: dict,
    *,
    preloaded: tuple[dict, str] | None = None,
) -> None:
    """Update specific frontmatter fields without changing the content.

    By default, reads the file via ``read_frontmatter()`` to get the current
    metadata + body, applies the updates, and writes the result back.

    When ``preloaded=(meta, body)`` is supplied, the function does NOT read
    the file — it uses the supplied tuple directly. This eliminates the
    hidden 2x read cost when callers like ``_sync_brain_state`` have already
    read the file's frontmatter and just need to update one or two fields.

    The caller's metadata dict is NOT mutated (defensive copy).

    Dispatches on file suffix: `.yml` / `.yaml` go through the yaml-neuron
    path (`_write_yaml_neuron` with `read_frontmatter` for the initial load)
    so the hash-style `#---` block is used. Markdown files use python-
    frontmatter.
    """
    is_yaml = path.suffix.lower() in YAML_SUFFIXES

    if preloaded is None:
        if is_yaml:
            current_meta, body = _read_yaml_neuron(path)
            new_meta = dict(current_meta)
            new_meta.update(updates)
            _write_yaml_neuron(path, new_meta, body)
            return
        post = frontmatter.load(str(path))
        for key, value in updates.items():
            post[key] = value
        path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
        return

    meta, body = preloaded
    new_meta = dict(meta)  # defensive copy so the caller's dict is unchanged
    new_meta.update(updates)
    if is_yaml:
        _write_yaml_neuron(path, new_meta, body)
        return
    post = frontmatter.Post(body, **new_meta)
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
