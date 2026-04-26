"""Lexical brain search.

Walks every neuron + glossary entry + brain.md body, scores literal-
substring matches against title/tag/path/body fields, and returns
ranked results. The single source of truth for read-only search.
"""

from __future__ import annotations

import re
from pathlib import Path

from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_index import YAML_NEURON_SUFFIXES, neuron_files


def extract_neuron_title(path: Path, content: str) -> str:
    """Return the neuron's display title.

    Prefer the first ``# Heading`` line from the body. Fall back to the
    filename stem with hyphens replaced by spaces and title-cased.
    """
    for line in content.splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem.replace("-", " ").title()


_GLOSSARY_TABLE_ROW = re.compile(r"^\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*$")
_GLOSSARY_BOLD_DASH = re.compile(r"^\*\*(.+?)\*\*\s*[-–—]+\s*(.+?)\s*$")
_GLOSSARY_HEADER_TERMS = {
    "term", "meaning", "definition", ":------", ":---", "---", "------",
}


def parse_glossary_entries(body: str) -> list[tuple[str, str]]:
    """Parse glossary.md body into ``[(term, definition)]`` tuples.

    Header rows, separator rows, and lines that match neither format
    are silently skipped.
    """
    entries: list[tuple[str, str]] = []
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        m = _GLOSSARY_BOLD_DASH.match(line)
        if m:
            term, definition = m.group(1).strip(), m.group(2).strip()
            if term and definition and term.lower() not in _GLOSSARY_HEADER_TERMS:
                entries.append((term, definition))
            continue

        m = _GLOSSARY_TABLE_ROW.match(line)
        if m:
            term, definition = m.group(1).strip(), m.group(2).strip()
            term_lower = term.lower()
            if (
                term_lower in _GLOSSARY_HEADER_TERMS
                or definition.lower() in _GLOSSARY_HEADER_TERMS
                or set(term) <= {"-", ":", " "}
            ):
                continue
            if term and definition:
                entries.append((term, definition))
    return entries


_FIELD_WEIGHTS = (
    ("title", 10),
    ("tag", 5),
    ("path", 3),
    ("body", 1),
)


def _count_in_fields(item: dict, query_lower: str) -> dict[str, int]:
    title_count = item["title"].lower().count(query_lower)
    body_count = item["body"].lower().count(query_lower)
    path_count = item["file"].lower().count(query_lower)
    tag_count = sum(
        1 for tag in item.get("tags", []) if query_lower in str(tag).lower()
    )
    return {
        "title": title_count,
        "tag": tag_count,
        "path": path_count,
        "body": body_count,
    }


def score_hit(item: dict, query_lower: str) -> int:
    """Score one searchable item against the query.

    Formula: ``title*10 + tag*5 + path*3 + body*1`` where each multiplier
    is the number of times the query appears in that field.
    """
    counts = _count_in_fields(item, query_lower)
    return sum(counts[field] * weight for field, weight in _FIELD_WEIGHTS)


def matched_fields(item: dict, query_lower: str) -> list[str]:
    """Return field names where the query matched at least once."""
    counts = _count_in_fields(item, query_lower)
    return [field for field, _ in _FIELD_WEIGHTS if counts[field] > 0]


def extract_snippet(text: str, query_lower: str, *, width: int = 200) -> str:
    """Return up to ``width`` chars of body text centered on the first match."""
    if not text or not query_lower:
        return ""

    text_lower = text.lower()
    idx = text_lower.find(query_lower)
    if idx < 0:
        return ""

    half = max(0, (width - len(query_lower)) // 2)
    start = max(0, idx - half)
    end = min(len(text), idx + len(query_lower) + half)

    snippet = text[start:end]
    if start > 0:
        snippet = "..." + snippet
    if end < len(text):
        snippet = snippet + "..."
    return snippet


def _rel(brain_path: Path, target: Path) -> str:
    """Brain-relative POSIX path string for stable JSON output."""
    return str(target.relative_to(brain_path)).replace("\\", "/")


def collect_searchable(brain_path: Path) -> list[dict]:
    """Walk the brain and return one dict per searchable item.

    Each dict has the same shape regardless of source kind:

    - ``kind``: ``"neuron"`` | ``"glossary"`` | ``"brain_md"``
    - ``file``: brain-relative POSIX path
    - ``title``: display title (H1 from body for neurons/brain.md, term for glossary)
    - ``tags``: list of frontmatter tags
    - ``body``: markdown body text (definition for glossary)
    - ``is_deprecated``: True iff the neuron has ``status: deprecated``
    """
    items: list[dict] = []

    for neuron in neuron_files(brain_path):
        try:
            meta, body = read_frontmatter(neuron)
        except Exception:
            continue
        is_yaml = neuron.suffix.lower() in YAML_NEURON_SUFFIXES
        file_type = "yaml" if is_yaml else "markdown"
        if is_yaml:
            fm_title = meta.get("title")
            if isinstance(fm_title, str) and fm_title.strip():
                title = fm_title.strip()
            else:
                title = neuron.stem.replace("-", " ").title()
        else:
            title = extract_neuron_title(neuron, body)
        tags = meta.get("tags", [])
        if not isinstance(tags, list):
            tags = []
        is_deprecated = str(meta.get("status", "active")).lower() == "deprecated"
        items.append({
            "kind": "neuron",
            "file": _rel(brain_path, neuron),
            "file_type": file_type,
            "title": title,
            "tags": tags,
            "body": body,
            "is_deprecated": is_deprecated,
        })

    glossary_path = brain_path / "glossary.md"
    if glossary_path.exists():
        try:
            _meta, glossary_body = read_frontmatter(glossary_path)
        except Exception:
            glossary_body = ""
        if isinstance(glossary_body, str):
            for term, definition in parse_glossary_entries(glossary_body):
                items.append({
                    "kind": "glossary",
                    "file": "glossary.md",
                    "file_type": "markdown",
                    "title": term,
                    "tags": [],
                    "body": definition,
                    "is_deprecated": False,
                })

    brain_md_path = brain_path / "brain.md"
    if brain_md_path.exists():
        try:
            _meta, brain_md_body = read_frontmatter(brain_md_path)
        except Exception:
            brain_md_body = ""
        if isinstance(brain_md_body, str):
            title = extract_neuron_title(brain_md_path, brain_md_body)
            items.append({
                "kind": "brain_md",
                "file": "brain.md",
                "file_type": "markdown",
                "title": title,
                "tags": [],
                "body": brain_md_body,
                "is_deprecated": False,
            })

    return items


def _passes_filters(
    item: dict,
    *,
    lobe_filter: str | None,
    tag_filter: str | None,
) -> bool:
    """Apply optional lobe and tag filters. AND semantics."""
    if lobe_filter is not None:
        prefix = lobe_filter.rstrip("/") + "/"
        if not item["file"].startswith(prefix):
            return False
    if tag_filter is not None:
        if tag_filter not in item.get("tags", []):
            return False
    return True


def search_brain(
    brain_path: Path,
    query: str,
    *,
    limit: int = 10,
    lobe_filter: str | None = None,
    tag_filter: str | None = None,
) -> list[dict]:
    """Search a brain for ``query``, returning ranked results.

    Returns a list of result dicts ordered by descending score, ties
    broken by file path. Items with score 0 are filtered out. Capped
    at ``limit`` results.
    """
    if limit <= 0:
        return []

    query_lower = query.lower()
    items = collect_searchable(brain_path)

    scored: list[tuple[int, str, dict]] = []
    for item in items:
        if not _passes_filters(item, lobe_filter=lobe_filter, tag_filter=tag_filter):
            continue
        score = score_hit(item, query_lower)
        if score == 0:
            continue
        fields = matched_fields(item, query_lower)
        snippet = (
            extract_snippet(item["body"], query_lower)
            if "body" in fields
            else ""
        )
        scored.append((score, item["file"], {
            "file": item["file"],
            "file_type": item.get("file_type", "markdown"),
            "title": item["title"],
            "matched_fields": fields,
            "snippet": snippet,
            "score": score,
            "deprecated": item["is_deprecated"],
        }))

    scored.sort(key=lambda triple: (-triple[0], triple[1]))
    return [result for _score, _file, result in scored[:limit]]
