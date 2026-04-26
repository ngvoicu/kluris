"""TEST-PACK-49 — scripted-provider sanity + agent trace hook."""

from __future__ import annotations

import pytest

from kluris.pack.agent import run_agent
from kluris.pack.config import Config

from .scripted_provider import ScriptedProvider


pytestmark = pytest.mark.asyncio


async def test_scripted_provider_replays_script(eval_config: Config):
    provider = ScriptedProvider([
        [
            {"kind": "tool_use", "name": "search", "id": "tu1",
             "args": {"query": "auth"}},
            {"kind": "end"},
        ],
        [
            {"kind": "token", "text": "ans"},
            {"kind": "end"},
        ],
    ])
    trace: list[dict] = []
    events = []
    async for ev in run_agent(
        config=eval_config,
        provider=provider,
        history=[],
        user_message="x",
        trace_hook=trace.append,
    ):
        events.append(ev)
    assert provider.calls == 2
    kinds = [e["kind"] for e in events]
    assert "tool" in kinds
    assert "tool_result" in kinds
    assert events[-1]["kind"] == "end"


async def test_trace_hook_excludes_secrets(eval_config: Config):
    provider = ScriptedProvider([
        [
            {"kind": "tool_use", "name": "read_neuron", "id": "tu",
             "args": {"path": "knowledge/jwt.md"}},
            {"kind": "end"},
        ],
        [{"kind": "token", "text": "ok"}, {"kind": "end"}],
    ])
    trace: list[dict] = []
    async for _ in run_agent(
        config=eval_config,
        provider=provider,
        history=[],
        user_message="x",
        trace_hook=trace.append,
    ):
        pass
    rendered = repr(trace)
    assert "sk-" not in rendered
    # The trace's result_summary is short — never the full neuron body.
    for entry in trace:
        assert len(entry.get("result_summary", "")) < 200
