"""Search over a kluris brain (read-only).

Implementation lives in :mod:`kluris_runtime.search`. This module
re-exports both the public ``search_brain`` and the private helpers
that pre-existing tests import directly so the runtime stays the
single source of truth.

Designed to back the ``kluris search <query>`` CLI command.
"""

from __future__ import annotations

from kluris_runtime.search import (  # noqa: F401  (re-exports)
    _FIELD_WEIGHTS,
    _GLOSSARY_BOLD_DASH,
    _GLOSSARY_HEADER_TERMS,
    _GLOSSARY_TABLE_ROW,
    _count_in_fields,
    _passes_filters,
    _rel,
    collect_searchable as _collect_searchable,
    extract_neuron_title,
    extract_snippet as _extract_snippet,
    matched_fields as _matched_fields,
    parse_glossary_entries as _parse_glossary_entries,
    score_hit as _score_hit,
    search_brain,
)
