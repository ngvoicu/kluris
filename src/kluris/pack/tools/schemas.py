"""Tool schemas in OpenAI function-calling format.

LiteLLM takes OpenAI-format tool schemas for EVERY provider (translating to
each native shape internally), so :func:`openai_schemas` is the single
emitter. The names match the :data:`kluris.pack.tools.brain.TOOLS` dispatch
table exactly.

``multi_read.paths.maxItems`` is read at app boot from the runtime
``KLURIS_MAX_MULTI_READ_PATHS`` value so the schema and the runtime
validation never drift.
"""

from __future__ import annotations

from typing import Any


def _wake_up_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {},
        "additionalProperties": False,
    }


def _search_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "query": {"type": "string", "minLength": 1},
            "limit": {
                "type": "integer", "minimum": 1, "maximum": 50,
                "description": "Hits per page (per lobe when group_by_lobe).",
            },
            "lobe": {"type": "string"},
            "tag": {"type": "string"},
            "offset": {
                "type": "integer", "minimum": 0,
                "description": (
                    "Skip into the ranked results. `total` in the response is "
                    "the full match count — page with offset instead of "
                    "re-searching with rephrased queries."
                ),
            },
            "snippet_chars": {
                "type": "integer", "minimum": 50, "maximum": 2000,
                "description": "Widen each hit's body snippet (default 200 chars).",
            },
            "full_bodies": {
                "type": "integer", "minimum": 0, "maximum": 5,
                "description": (
                    "Attach the (clamped) full body of the top N hits so a "
                    "question can be answered from one search without "
                    "per-hit read_neuron calls."
                ),
            },
            "group_by_lobe": {
                "type": "boolean",
                "description": (
                    "Return the top `limit` hits PER lobe instead of a flat "
                    "list — the one-call way to answer 'X across every "
                    "lobe/country'. Use a small limit (3-5)."
                ),
            },
        },
        "required": ["query"],
        "additionalProperties": False,
    }


def _read_neuron_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "path": {"type": "string", "minLength": 1},
        },
        "required": ["path"],
        "additionalProperties": False,
    }


def _multi_read_schema(max_paths: int) -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "paths": {
                "type": "array",
                "items": {"type": "string", "minLength": 1},
                "minItems": 1,
                "maxItems": max_paths,
            },
        },
        "required": ["paths"],
        "additionalProperties": False,
    }


def _related_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {"path": {"type": "string", "minLength": 1}},
        "required": ["path"],
        "additionalProperties": False,
    }


def _recent_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "limit": {"type": "integer", "minimum": 1, "maximum": 100},
            "lobe": {"type": "string"},
            "include_deprecated": {"type": "boolean"},
        },
        "additionalProperties": False,
    }


def _glossary_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {"term": {"type": "string"}},
        "additionalProperties": False,
    }


def _lobe_overview_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "lobe": {"type": "string", "minLength": 1},
            "offset": {
                "type": "integer", "minimum": 0,
                "description": (
                    "Skip into the lobe's neuron list. The response carries "
                    "total_count and next_offset — page large lobes instead "
                    "of accepting the truncated head."
                ),
            },
        },
        "required": ["lobe"],
        "additionalProperties": False,
    }


_DESCRIPTIONS = {
    "wake_up": "Compact snapshot of the brain: lobes (with their top tags), recent neurons, glossary terms, deprecation diagnostics. Call FIRST in a session.",
    "search": "Ranked search across neurons + glossary + brain.md. `total` is the full match count; page with `offset`. For broad questions use `full_bodies` (answer from one call) or `group_by_lobe` (top hits per lobe) instead of re-searching with rephrasings.",
    "read_neuron": "Read one neuron's frontmatter and body by brain-relative path.",
    "multi_read": "Read multiple neurons in one call (saves round trips when composing across sources).",
    "related": "Outbound + inbound related neurons for a given neuron path.",
    "recent": "List recently-updated neurons by frontmatter `updated:` desc.",
    "glossary": "Look up a glossary term, or list every entry when called with no `term`.",
    "lobe_overview": "Lobe map.md body + per-neuron title/excerpt/tags + tag union. Use to triage a lobe before deep reading; page large lobes with `offset`.",
}


def _schemas_for(max_multi_read: int) -> dict[str, dict[str, Any]]:
    return {
        "wake_up": _wake_up_schema(),
        "search": _search_schema(),
        "read_neuron": _read_neuron_schema(),
        "multi_read": _multi_read_schema(max_multi_read),
        "related": _related_schema(),
        "recent": _recent_schema(),
        "glossary": _glossary_schema(),
        "lobe_overview": _lobe_overview_schema(),
    }


def openai_schemas(max_multi_read: int) -> list[dict[str, Any]]:
    """Tool list in OpenAI function-calling format.

    ``strict: false`` is emitted on every function. The Responses API (and
    LiteLLM's bridge) passes ``strict`` through; omitting it makes OpenAI
    normalize functions to STRICT mode (every property ``required`` +
    ``additionalProperties:false``), which the optional-param tools (``recent``,
    ``glossary``, ``search``) violate → 400. ``strict:false`` is the best-effort
    calling the pack wants.
    """
    out: list[dict[str, Any]] = []
    for name, schema in _schemas_for(max_multi_read).items():
        out.append({
            "type": "function",
            "function": {
                "name": name,
                "description": _DESCRIPTIONS[name],
                "parameters": schema,
                "strict": False,
            },
        })
    return out
