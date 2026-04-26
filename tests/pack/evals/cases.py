"""Eval question fixtures + scripted provider responses.

Each case wires:
- the question
- the scripted provider's per-turn event lists
- the expected tool-trace prefix
- the expected answer text (string the scripted provider concatenates
  via ``token`` events; assertions read the full text after streaming)
- assertion arguments for the answer-shape helpers

Cases are deliberately simple — no LLM judge, only deterministic
substring checks. The point is to catch retrieval orchestration and
answer-shape regressions, not to grade prose.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable


@dataclass
class EvalCase:
    name: str
    question: str
    scripts: list[list[dict[str, Any]]]
    expected_trace_prefix: list[str]
    cited_paths: tuple[str, ...] = ()
    expects_sources_block: bool = False
    expects_no_answer: bool = False
    expects_conflict: bool = False
    expects_replacement: str = ""
    forbidden_inventions: tuple[str, ...] = ()


def _tool(name: str, args: dict, *, idx: int = 1) -> dict:
    return {"kind": "tool_use", "name": name, "id": f"tu{idx}", "args": args}


def _final(text: str, *, input_tokens: int = 5, output_tokens: int = 5) -> list[dict]:
    return [
        {"kind": "token", "text": text},
        {"kind": "usage", "input": input_tokens, "output": output_tokens},
        {"kind": "end"},
    ]


_NARROW_FACTUAL = EvalCase(
    name="narrow_factual",
    question="how does BTB authenticate users?",
    scripts=[
        [_tool("wake_up", {}), {"kind": "end"}],
        [_tool("search", {"query": "auth"}), {"kind": "end"}],
        [_tool("read_neuron", {"path": "projects/btb/auth.md"}), {"kind": "end"}],
        _final(
            "BTB authenticates users via JWT issued by Keycloak, per "
            "`projects/btb/auth.md`."
        ),
    ],
    expected_trace_prefix=["wake_up", "search", "read_neuron"],
    cited_paths=("projects/btb/auth.md",),
)


_BROAD_SYNTHESIS = EvalCase(
    name="broad_synthesis",
    question="how does authentication work across the platform?",
    scripts=[
        [_tool("wake_up", {}), {"kind": "end"}],
        [_tool("search", {"query": "auth"}), {"kind": "end"}],
        [_tool("multi_read", {"paths": [
            "projects/btb/auth.md", "knowledge/jwt.md",
        ]}), {"kind": "end"}],
        _final(
            "BTB uses JWT issued by Keycloak (`projects/btb/auth.md`); "
            "the underlying token format is documented in `knowledge/jwt.md`.\n\n"
            "Sources:\n- projects/btb/auth.md\n- knowledge/jwt.md\n"
        ),
    ],
    expected_trace_prefix=["wake_up", "search", "multi_read"],
    cited_paths=("projects/btb/auth.md", "knowledge/jwt.md"),
    expects_sources_block=True,
)


_DEFINITION = EvalCase(
    name="definition",
    question="what does JWT mean?",
    scripts=[
        [_tool("glossary", {"term": "JWT"}), {"kind": "end"}],
        _final(
            "JWT is JSON Web Token, used for stateless auth (per the brain "
            "glossary)."
        ),
    ],
    expected_trace_prefix=["glossary"],
    cited_paths=(),
)


_RECENCY = EvalCase(
    name="recency",
    question="what's changed recently in the knowledge lobe?",
    scripts=[
        [_tool("recent", {"lobe": "knowledge"}), {"kind": "end"}],
        [_tool("read_neuron", {"path": "knowledge/raw-sql-modern.md"}), {"kind": "end"}],
        _final(
            "Most recent knowledge update: `knowledge/raw-sql-modern.md` (2026-04-15) "
            "— current guidance is to prefer raw SQL over JPA."
        ),
    ],
    expected_trace_prefix=["recent", "read_neuron"],
    cited_paths=("knowledge/raw-sql-modern.md",),
)


_CROSS_LOBE = EvalCase(
    name="cross_lobe_comparison",
    question="which projects use JWT auth?",
    scripts=[
        [_tool("search", {"query": "jwt"}), {"kind": "end"}],
        [_tool("lobe_overview", {"lobe": "projects"}), {"kind": "end"}],
        _final(
            "Project BTB uses JWT (`projects/btb/auth.md`); no other project lobe "
            "currently mentions JWT.\n\nSources:\n- projects/btb/auth.md\n- knowledge/jwt.md\n"
        ),
    ],
    expected_trace_prefix=["search", "lobe_overview"],
    cited_paths=("projects/btb/auth.md", "knowledge/jwt.md"),
    expects_sources_block=True,
)


_NO_ANSWER = EvalCase(
    name="no_answer",
    question="what's our policy on quantum computing?",
    scripts=[
        [_tool("search", {"query": "quantum"}), {"kind": "end"}],
        _final("Nothing in the brain about quantum computing."),
    ],
    expected_trace_prefix=["search"],
    expects_no_answer=True,
    forbidden_inventions=("Shor's algorithm", "quantum supremacy"),
)


_CONFLICT = EvalCase(
    name="conflict",
    question="should we use raw SQL or JPA?",
    scripts=[
        [_tool("search", {"query": "sql"}), {"kind": "end"}],
        [_tool("multi_read", {"paths": [
            "knowledge/raw-sql-modern.md", "knowledge/raw-sql-old.md",
        ]}), {"kind": "end"}],
        _final(
            "There's a conflict between current and old guidance: "
            "`knowledge/raw-sql-modern.md` recommends raw SQL while "
            "`knowledge/raw-sql-old.md` (deprecated) preserves the historic stance.\n\n"
            "Sources:\n- knowledge/raw-sql-modern.md\n- knowledge/raw-sql-old.md\n"
        ),
    ],
    expected_trace_prefix=["search", "multi_read"],
    cited_paths=("knowledge/raw-sql-modern.md", "knowledge/raw-sql-old.md"),
    expects_sources_block=True,
    expects_conflict=True,
)


_DEPRECATED_REPLACEMENT = EvalCase(
    name="deprecated_replacement",
    question="what's the guidance on raw SQL?",
    scripts=[
        [_tool("read_neuron", {"path": "knowledge/raw-sql-old.md"}), {"kind": "end"}],
        [_tool("read_neuron", {"path": "knowledge/raw-sql-modern.md"}), {"kind": "end"}],
        _final(
            "The old guidance in `knowledge/raw-sql-old.md` was superseded by "
            "`knowledge/raw-sql-modern.md` — use the modern guidance.\n\n"
            "Sources:\n- knowledge/raw-sql-modern.md\n- knowledge/raw-sql-old.md\n"
        ),
    ],
    expected_trace_prefix=["read_neuron", "read_neuron"],
    cited_paths=("knowledge/raw-sql-old.md", "knowledge/raw-sql-modern.md"),
    expects_sources_block=True,
    expects_replacement="knowledge/raw-sql-modern.md",
)


_LOBE_OVERVIEW = EvalCase(
    name="lobe_overview",
    question="give me the gist of the infrastructure lobe",
    scripts=[
        [_tool("lobe_overview", {"lobe": "infrastructure"}), {"kind": "end"}],
        _final(
            "Infrastructure lobe: hosting + deploys notes, plus an OpenAPI "
            "spec at `infrastructure/openapi.yml`."
        ),
    ],
    expected_trace_prefix=["lobe_overview"],
    cited_paths=("infrastructure/openapi.yml",),
)


CASES: list[EvalCase] = [
    _NARROW_FACTUAL,
    _BROAD_SYNTHESIS,
    _DEFINITION,
    _RECENCY,
    _CROSS_LOBE,
    _NO_ANSWER,
    _CONFLICT,
    _DEPRECATED_REPLACEMENT,
    _LOBE_OVERVIEW,
]
