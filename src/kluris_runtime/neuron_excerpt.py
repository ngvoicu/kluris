"""Read-only neuron title + first-line excerpt extraction.

Shared by the MRI viewer and the packed chat server's ``lobe_overview``
tool. Returns the H1 (or filename-derived fallback) and the first non-
trivial body line — enough to triage a neuron without reading it.
"""

from __future__ import annotations

import re
from pathlib import Path

# HTML anchor tags like ``<a id="x"></a>`` are sometimes inlined into
# headings or body lines for cross-referencing. Strip them so they
# don't bleed into the excerpt.
_EMPTY_HTML_ANCHOR = re.compile(r"<a\s+[^>]*></a>")


def _strip_empty_html_anchors(text: str) -> str:
    return _EMPTY_HTML_ANCHOR.sub("", text)


def extract(path: Path, body: str) -> tuple[str, str]:
    """Return ``(title, excerpt)`` for the neuron at ``path``.

    - ``title``: the first ``# Heading`` line in ``body`` if present,
      otherwise the filename stem with hyphens replaced by spaces and
      title-cased.
    - ``excerpt``: the first non-trivial body line after the H1,
      skipping subheadings (``##``), list markers (``-``, ``*``), code
      fences, frontmatter rulers (``---``), and navigation lines
      (``up ``, ``sideways ``). Capped at 220 characters.

    The function is a pure read — no I/O. Callers pass ``body`` from
    :func:`kluris_runtime.frontmatter.read_frontmatter`.
    """
    title = ""
    excerpt = ""
    title_seen = False

    for raw_line in body.splitlines():
        line = _strip_empty_html_anchors(raw_line).strip()
        if not line:
            continue
        if line.startswith("# ") and not title:
            title = line[2:].strip()
            title_seen = True
            continue
        # Skip non-content lines whether or not the H1 has appeared.
        if line.startswith(("## ", "- ", "* ", "```", "---", "up ", "sideways ")):
            continue
        excerpt = line
        break

    if not title:
        title = path.stem.replace("-", " ").title()
    return title, excerpt[:220]
