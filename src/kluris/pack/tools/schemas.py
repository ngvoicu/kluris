"""Tool schemas in Anthropic + OpenAI tool-calling formats.

Provider classes pick the right shape per request via
:func:`anthropic_schemas` / :func:`openai_schemas`. The names match the
:data:`kluris.pack.tools.brain.TOOLS` dispatch table exactly.

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
            "limit": {"type": "integer", "minimum": 1, "maximum": 50},
            "lobe": {"type": "string"},
            "tag": {"type": "string"},
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
        "properties": {"lobe": {"type": "string", "minLength": 1}},
        "required": ["lobe"],
        "additionalProperties": False,
    }


_DESCRIPTIONS = {
    "wake_up": "Compact snapshot of the brain: lobes, recent neurons, glossary count, deprecation diagnostics. Call FIRST in a session.",
    "search": "Lexical search across neurons + glossary + brain.md. Returns ranked results with source paths.",
    "read_neuron": "Read one neuron's frontmatter and body by brain-relative path.",
    "multi_read": "Read multiple neurons in one call (saves round trips when composing across sources).",
    "related": "Outbound + inbound related neurons for a given neuron path.",
    "recent": "List recently-updated neurons by frontmatter `updated:` desc.",
    "glossary": "Look up a glossary term, or list every entry when called with no `term`.",
    "lobe_overview": "Lobe map.md body + per-neuron title/excerpt/tags + tag union. Use to triage a lobe before deep reading.",
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


def anthropic_schemas(max_multi_read: int) -> list[dict[str, Any]]:
    """Tool list in Anthropic Messages API format."""
    out: list[dict[str, Any]] = []
    for name, schema in _schemas_for(max_multi_read).items():
        out.append({
            "name": name,
            "description": _DESCRIPTIONS[name],
            "input_schema": schema,
        })
    return out


def openai_schemas(max_multi_read: int) -> list[dict[str, Any]]:
    """Tool list in OpenAI Chat Completions function-calling format."""
    out: list[dict[str, Any]] = []
    for name, schema in _schemas_for(max_multi_read).items():
        out.append({
            "type": "function",
            "function": {
                "name": name,
                "description": _DESCRIPTIONS[name],
                "parameters": schema,
            },
        })
    return out
