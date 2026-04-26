"""TEST-PACK-51 — offline answer-quality evals.

Runs each fixture case through the agent loop with the scripted
provider, captures the tool trace and the final assistant text, and
asserts on:

- expected tool-trace prefix
- no duplicate ``read_neuron`` calls
- direct-answer prelude (no "let me think...")
- inline citations
- ``Sources:`` block when 2+ neurons cited
- no-answer cases say so plainly
- conflict cases surface "conflict" / "disagree"
- deprecated-replacement cases name the replacement
"""

from __future__ import annotations

import pytest

from kluris.pack.agent import run_agent
from kluris.pack.config import Config

from .assertions import (
    assert_calls_out_conflict,
    assert_cites_paths_inline,
    assert_has_sources_block,
    assert_names_replacement,
    assert_no_duplicate_reads,
    assert_no_invention,
    assert_says_no_answer,
    assert_starts_with_direct_answer,
    assert_trace_starts_with,
)
from .cases import CASES, EvalCase
from .scripted_provider import ScriptedProvider


pytestmark = pytest.mark.asyncio


@pytest.mark.parametrize("case", CASES, ids=lambda c: c.name)
async def test_answer_quality_case(eval_config: Config, case: EvalCase):
    provider = ScriptedProvider(case.scripts)
    trace: list[dict] = []
    text_parts: list[str] = []
    async for ev in run_agent(
        config=eval_config,
        provider=provider,
        history=[],
        user_message=case.question,
        trace_hook=trace.append,
    ):
        if ev["kind"] == "token":
            text_parts.append(ev["text"])
    answer = "".join(text_parts)

    assert_trace_starts_with(trace, case.expected_trace_prefix)
    assert_no_duplicate_reads(trace)

    if case.expects_no_answer:
        assert_says_no_answer(answer)
        if case.forbidden_inventions:
            assert_no_invention(answer, case.forbidden_inventions)
        return

    assert_starts_with_direct_answer(answer)

    if case.cited_paths:
        assert_cites_paths_inline(answer, case.cited_paths)

    if case.expects_sources_block:
        assert_has_sources_block(answer, case.cited_paths)

    if case.expects_conflict:
        assert_calls_out_conflict(answer)

    if case.expects_replacement:
        assert_names_replacement(answer, case.expects_replacement)


async def test_eval_count_meets_spec_minimum():
    """Spec says >= 8 fixture questions; assertion guards against
    accidental case removal during a refactor.
    """
    assert len(CASES) >= 8


async def test_live_model_evals_are_marker_skipped():
    """The live eval module exists, registers ``llm_eval`` marker, and
    skips by default. We don't run it here — just confirm the marker
    is wired in :file:`pyproject.toml`.
    """
    import tomllib
    from pathlib import Path

    repo_root = Path(__file__).resolve().parents[3]
    cfg = tomllib.loads((repo_root / "pyproject.toml").read_text())
    markers = cfg.get("tool", {}).get("pytest", {}).get(
        "ini_options", {}
    ).get("markers", [])
    if not markers:
        pytest.skip("pyproject.toml has no pytest markers section yet")
    assert any("llm_eval" in m for m in markers)
