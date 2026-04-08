"""YAML frontmatter read/write operations using python-frontmatter."""

from __future__ import annotations

from pathlib import Path

import frontmatter


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


def read_frontmatter(path: Path) -> tuple[dict, str]:
    """Read a markdown file and return (metadata_dict, content_string)."""
    post = frontmatter.load(str(path))
    return _normalize_metadata(dict(post.metadata)), post.content


def write_frontmatter(path: Path, metadata: dict, content: str) -> None:
    """Write a markdown file with YAML frontmatter and content."""
    post = frontmatter.Post(content, **metadata)
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")


def update_frontmatter(
    path: Path,
    updates: dict,
    *,
    preloaded: tuple[dict, str] | None = None,
) -> None:
    """Update specific frontmatter fields without changing the content.

    By default, reads the file via ``frontmatter.load()`` to get the current
    metadata + body, applies the updates, and writes the result back.

    When ``preloaded=(meta, body)`` is supplied, the function does NOT call
    ``frontmatter.load()`` — it uses the supplied tuple directly. This
    eliminates the hidden 2x read cost when callers like ``_sync_brain_state``
    have already read the file's frontmatter and just need to update one or
    two fields.

    The caller's metadata dict is NOT mutated (defensive copy).
    """
    if preloaded is None:
        post = frontmatter.load(str(path))
        for key, value in updates.items():
            post[key] = value
        path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
        return

    meta, body = preloaded
    new_meta = dict(meta)  # defensive copy so the caller's dict is unchanged
    new_meta.update(updates)
    post = frontmatter.Post(body, **new_meta)
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
