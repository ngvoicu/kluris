"""TEST-PACK-LIVE — opt-in live contract test for the OpenAI Responses path.

This is the ONE thing the mocked suite cannot prove: that gpt-5.x reasoning
models accept ``reasoning_effort`` + function tools across MULTIPLE tool rounds
on the Responses API without a 400. It settles the spec's gating risks:

- R1: "function_call provided without its required 'reasoning' item" on
  stateless multi-round tool calling (the pack carries no reasoning items).
- R2: strict-schema 400 on the optional-param tools (fixed by ``strict:false``).

It is SKIPPED unless ``KLURIS_LIVE_OPENAI_KEY`` is set in the environment — the
key must NEVER be pasted into source or chat. Run it explicitly with::

    KLURIS_LIVE_OPENAI_KEY=sk-... ./.venv/bin/python -m pytest \
        tests/pack/test_litellm_contract.py -m live_openai -v

When it passes, the headline feature is proven end-to-end. Until then, the
default suite proves call-construction only, not real provider behavior.
"""

from __future__ import annotations

import json
import os

import pytest

from kluris.pack.config import Config
from kluris.pack.providers.litellm_provider import LiteLLMProvider, configure_litellm
from kluris.pack.tools.schemas import openai_schemas

pytestmark = [
    pytest.mark.asyncio,
    pytest.mark.live_openai,
    pytest.mark.skipif(
        not os.environ.get("KLURIS_LIVE_OPENAI_KEY"),
        reason="set KLURIS_LIVE_OPENAI_KEY to run the live OpenAI Responses contract test",
    ),
]

_LIVE_MODEL = os.environ.get("KLURIS_LIVE_OPENAI_MODEL", "gpt-5.4-mini")


def _live_config() -> Config:
    return Config.load_from_env({
        "KLURIS_PROVIDER_SHAPE": "openai",
        "KLURIS_BASE_URL": "https://api.openai.com",
        "KLURIS_API_KEY": os.environ["KLURIS_LIVE_OPENAI_KEY"],
        "KLURIS_MODEL": _LIVE_MODEL,
        "KLURIS_REASONING_EFFORT": "medium",
        "KLURIS_BRAIN_DIR": "/app/brain",
    })


async def _collect(stream):
    events = []
    async for event in stream:
        events.append(event)
    return events


async def test_responses_reasoning_effort_plus_tools_survives_two_rounds():
    cfg = _live_config()
    configure_litellm(cfg)
    provider = LiteLLMProvider(cfg)
    assert provider._model_string == f"openai/responses/{_LIVE_MODEL}"

    tools = openai_schemas(max_multi_read=5)
    messages = [
        {"role": "system", "content": "You are a brain assistant. Use the search tool."},
        {"role": "user", "content": "Search the brain for 'authentication' and summarize."},
    ]

    # Round 1 — must come back with a tool_use, not a 400 (proves effort+tools
    # are accepted together on the Responses API).
    round1 = await _collect(provider.complete_stream(messages, tools))
    tool_uses = [e for e in round1 if e["kind"] == "tool_use"]
    assert tool_uses, f"round 1 produced no tool_use: {round1}"
    call = tool_uses[0]

    # Round 2 — feed a tool result back and continue. The key assertion is that
    # this does NOT 400 with a missing-reasoning-item error (R1).
    messages.append({
        "role": "assistant", "content": "",
        "tool_calls": [{"id": call.get("id") or "call_1",
                        "name": call["name"], "args": call["args"]}],
    })
    messages.append({
        "role": "tool", "tool_call_id": call.get("id") or "call_1",
        "content": json.dumps({"ok": True, "results": [
            {"path": "knowledge/auth.md", "snippet": "JWT issued by Keycloak."}]}),
    })
    round2 = await _collect(provider.complete_stream(messages, tools))
    kinds = {e["kind"] for e in round2}
    assert "error" not in kinds, f"round 2 surfaced an error: {round2}"
    assert kinds & {"token", "tool_use", "end"}, f"round 2 produced nothing usable: {round2}"
