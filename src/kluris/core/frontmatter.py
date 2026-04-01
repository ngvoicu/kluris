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


def update_frontmatter(path: Path, updates: dict) -> None:
    """Update specific frontmatter fields without changing the content."""
    post = frontmatter.load(str(path))
    for key, value in updates.items():
        post[key] = value
    path.write_text(frontmatter.dumps(post) + "\n", encoding="utf-8")
