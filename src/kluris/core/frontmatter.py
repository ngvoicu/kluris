"""YAML frontmatter read/write operations using python-frontmatter.

Markdown files use the standard `---` / `---` YAML block. Yaml neurons use
a hash-style `#---` / `#---` block where every line inside is a YAML
comment (prefix `# `), so the file stays valid yaml regardless of whether
a tool knows about the block.

Read APIs (``read_frontmatter``, ``_read_yaml_neuron``, ``YAML_SUFFIXES``,
``_normalize_metadata``) are re-exported from
:mod:`kluris_runtime.frontmatter` — the read-only runtime is the single
source of truth. Write helpers (``write_frontmatter``,
``update_frontmatter``) live here because writing is a CLI concern.
"""

from __future__ import annotations

from pathlib import Path

import frontmatter
import yaml

# Read APIs are sourced from the runtime so behavior never forks.
from kluris_runtime.frontmatter import (  # noqa: F401  (re-exports)
    YAML_SUFFIXES,
    _normalize_metadata,
    _read_yaml_neuron,
    read_frontmatter,
)


def _write_yaml_neuron(path: Path, metadata: dict, body: str) -> None:
    """Write a yaml neuron with a hash-style ``#---`` frontmatter block.

    The body is written verbatim — no yaml round-trip, no comment loss,
    no key reordering. The block is constructed by dumping the metadata
    dict via ``yaml.safe_dump``, then prefixing every line with ``# ``.

    Uses ``write_bytes`` (not ``write_text``) so the body's line endings
    are preserved exactly: Python's text-mode write translates ``\\n``
    to ``os.linesep`` on Windows, which would corrupt a CRLF body.
    Binary write is byte-for-byte.
    """
    dumped = yaml.safe_dump(metadata, sort_keys=True, default_flow_style=False)
    prefixed_lines = []
    for line in dumped.rstrip("\n").splitlines():
        prefixed_lines.append(f"# {line}" if line else "#")
    block = "#---\n" + "\n".join(prefixed_lines) + "\n#---\n"
    path.write_bytes((block + body).encode("utf-8"))


def write_frontmatter(path: Path, metadata: dict, content: str) -> None:
    """Write a markdown or yaml neuron with frontmatter + content.

    Yaml files get a ``#---`` hash block via :func:`_write_yaml_neuron`,
    which preserves the body byte-for-byte. Markdown files use the
    standard python-frontmatter ``---`` block.
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

    By default, reads the file via :func:`read_frontmatter` to get the
    current metadata + body, applies the updates, and writes the result
    back. When ``preloaded=(meta, body)`` is supplied, the function does
    NOT read the file — it uses the supplied tuple directly.
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
    new_meta = dict(meta)
    new_meta.update(updates)
    if is_yaml:
        _write_yaml_neuron(path, new_meta, body)
        return
    post = frontmatter.Post(body, **new_meta)
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
