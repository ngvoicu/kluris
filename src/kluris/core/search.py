"""Search over a kluris brain (read-only).

Walks every neuron + glossary entry + brain.md body, scores literal-substring
matches against title/tag/path/body fields, and returns ranked results.

Designed to back the ``kluris search <query>`` CLI command. Pure Python
stdlib + python-frontmatter (already a kluris dep). No regex search, no
external full-text index, no caching layer.
"""

from __future__ import annotations

import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter
from kluris.core.linker import _neuron_files


# --- Title extraction ---


def extract_neuron_title(path: Path, content: str) -> str:
    """Return the neuron's display title.

    Prefer the first ``# Heading`` line from the body. Fall back to the
    filename stem with hyphens replaced by spaces and title-cased — same
    convention used by ``core/maps.py:61-87`` for map.md rendering.
    """
    for line in content.splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem.replace("-", " ").title()


# --- Glossary parsing ---
#
# Mirrors ``cli._wake_up_collect_glossary`` so search and wake-up agree on
# what counts as a glossary entry. Both formats kluris ships and recommends
# are supported:
#
#   - Markdown table row:  | Term | Definition |     (the scaffolded format)
#   - Bold-dash one-liner: **Term** -- Definition    (the SKILL.md format)


_GLOSSARY_TABLE_ROW = re.compile(r"^\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*$")
_GLOSSARY_BOLD_DASH = re.compile(r"^\*\*(.+?)\*\*\s*[-–—]+\s*(.+?)\s*$")
_GLOSSARY_HEADER_TERMS = {
    "term", "meaning", "definition", ":------", ":---", "---", "------",
}


def _parse_glossary_entries(body: str) -> list[tuple[str, str]]:
    """Parse glossary.md body into ``[(term, definition)]`` tuples.

    Header rows, separator rows, and lines that match neither format are
    silently skipped.
    """
    entries: list[tuple[str, str]] = []
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        # Bold-dash format first (more specific).
        m = _GLOSSARY_BOLD_DASH.match(line)
        if m:
            term, definition = m.group(1).strip(), m.group(2).strip()
            if term and definition and term.lower() not in _GLOSSARY_HEADER_TERMS:
                entries.append((term, definition))
            continue

        # Markdown table row.
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


# --- Scoring ---


_FIELD_WEIGHTS = (
    ("title", 10),
    ("tag", 5),
    ("path", 3),
    ("body", 1),
)


def _count_in_fields(item: dict, query_lower: str) -> dict[str, int]:
    """Return ``{field: occurrence_count}`` for each scoreable field.

    Tag matching counts the number of tag values that contain the query
    as a substring (each matching tag = 1, regardless of in-tag count).
    All other fields use ``str.count`` for total occurrences.
    """
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


def _score_hit(item: dict, query_lower: str) -> int:
    """Score one searchable item against the query.

    Formula: ``title*10 + tag*5 + path*3 + body*1`` where each multiplier
    is the number of times the query appears in that field. Returns 0
    if the query never appears in any scoreable field.

    The query is treated as a literal substring — no regex interpretation.
    Both query and text are expected to be already lowercase-folded by
    the caller (the public ``search_brain`` does this once and reuses).
    """
    counts = _count_in_fields(item, query_lower)
    return sum(counts[field] * weight for field, weight in _FIELD_WEIGHTS)


def _matched_fields(item: dict, query_lower: str) -> list[str]:
    """Return field names where the query matched at least once.

    Order is canonical: ``title``, ``tag``, ``path``, ``body``. Empty
    list if no field matched.
    """
    counts = _count_in_fields(item, query_lower)
    return [field for field, _ in _FIELD_WEIGHTS if counts[field] > 0]


# --- Snippet extraction ---


def _extract_snippet(text: str, query_lower: str, *, width: int = 200) -> str:
    """Return up to ``width`` characters of body text centered on the first
    match of ``query_lower``.

    The query is treated as a literal substring, lowercase-folded for
    matching but the original case is preserved in the returned slice.
    Multi-byte characters are preserved (Python ``str`` slicing operates
    on code points, not bytes).

    Returns:
        - Empty string if the query is not found in ``text``.
        - The full body if it's shorter than ``width``.
        - A windowed slice with ``"..."`` ellipsis markers when one or
          both ends are truncated.
    """
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


# --- Searchable collection ---


def _rel(brain_path: Path, target: Path) -> str:
    """Brain-relative POSIX path string for stable JSON output."""
    return str(target.relative_to(brain_path)).replace("\\", "/")


def _collect_searchable(brain_path: Path) -> list[dict]:
    """Walk the brain and return one dict per searchable item.

    Each dict has the same shape regardless of source kind:

    - ``kind``: ``"neuron"`` | ``"glossary"`` | ``"brain_md"``
    - ``file``: brain-relative POSIX path (e.g. ``"projects/btb/auth.md"``)
    - ``title``: display title (H1 from body for neurons/brain.md, term for glossary)
    - ``tags``: list of frontmatter tags (empty for glossary/brain.md)
    - ``body``: markdown body text (definition for glossary)
    - ``is_deprecated``: True iff the neuron's frontmatter has ``status: deprecated``
    """
    items: list[dict] = []

    # 1. Neurons
    for neuron in _neuron_files(brain_path):
        try:
            meta, body = read_frontmatter(neuron)
        except Exception:
            continue
        title = extract_neuron_title(neuron, body)
        tags = meta.get("tags", [])
        if not isinstance(tags, list):
            tags = []
        is_deprecated = str(meta.get("status", "active")).lower() == "deprecated"
        items.append({
            "kind": "neuron",
            "file": _rel(brain_path, neuron),
            "title": title,
            "tags": tags,
            "body": body,
            "is_deprecated": is_deprecated,
        })

    # 2. Glossary entries
    glossary_path = brain_path / "glossary.md"
    if glossary_path.exists():
        try:
            _meta, glossary_body = read_frontmatter(glossary_path)
        except Exception:
            glossary_body = ""
        if isinstance(glossary_body, str):
            for term, definition in _parse_glossary_entries(glossary_body):
                items.append({
                    "kind": "glossary",
                    "file": "glossary.md",
                    "title": term,
                    "tags": [],
                    "body": definition,
                    "is_deprecated": False,
                })

    # 3. brain.md body
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
                "title": title,
                "tags": [],
                "body": brain_md_body,
                "is_deprecated": False,
            })

    return items


# --- Public search API ---


def _passes_filters(
    item: dict,
    *,
    lobe_filter: str | None,
    tag_filter: str | None,
) -> bool:
    """Apply optional lobe and tag filters. AND semantics."""
    if lobe_filter is not None:
        # Lobe filter matches neurons whose `file` starts with `<lobe>/`.
        # Glossary (file: "glossary.md") and brain.md (file: "brain.md")
        # naturally fail this check because they live at the brain root.
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

    The query is treated as a literal substring (no regex). Both query and
    item text are lowercase-folded for matching, but the original casing
    is preserved in the returned ``title`` / ``snippet`` fields.

    Returns a list of result dicts ordered by descending score, ties broken
    by file path (alphabetical, stable). Items with score 0 are filtered out.
    Capped at ``limit`` results.

    Optional filters:
        - ``lobe_filter``: keep only items whose ``file`` starts with
          ``<lobe>/``. Glossary and brain.md items live at the brain root,
          so a lobe filter naturally excludes them.
        - ``tag_filter``: keep only items whose frontmatter ``tags:`` list
          contains the exact tag string. Glossary and brain.md items have
          no tags and are excluded.

    Each result dict has:
        - ``file``: brain-relative POSIX path
        - ``title``: display title (H1 from body, term for glossary)
        - ``matched_fields``: list of field names where the query matched
        - ``snippet``: up to 200 chars of body context (empty if no body match)
        - ``score``: integer total score
    """
    if limit <= 0:
        return []

    query_lower = query.lower()
    items = _collect_searchable(brain_path)

    scored: list[tuple[int, str, dict]] = []
    for item in items:
        if not _passes_filters(item, lobe_filter=lobe_filter, tag_filter=tag_filter):
            continue
        score = _score_hit(item, query_lower)
        if score == 0:
            continue
        fields = _matched_fields(item, query_lower)
        snippet = (
            _extract_snippet(item["body"], query_lower)
            if "body" in fields
            else ""
        )
        scored.append((score, item["file"], {
            "file": item["file"],
            "title": item["title"],
            "matched_fields": fields,
            "snippet": snippet,
            "score": score,
            # `deprecated` comes straight from `_collect_searchable`'s
            # `is_deprecated` flag, which is set by reading frontmatter
            # `status: deprecated` directly during collection. Do NOT
            # source this from `linker.detect_deprecation_issues` —
            # that function reports issue states (broken chains, etc.),
            # NOT the full set of deprecated neurons. A clean deprecated
            # neuron with valid replaced_by would be missed by that path.
            "deprecated": item["is_deprecated"],
        }))

    # Sort: descending score first, ties broken by file path (ascending)
    scored.sort(key=lambda triple: (-triple[0], triple[1]))
    return [result for _score, _file, result in scored[:limit]]
