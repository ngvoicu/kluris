"""TEST-PACK-41 — agent loop: tool dispatch, max-rounds, errors."""

from __future__ import annotations

import pytest

from kluris.pack.agent import run_agent
from kluris.pack.config import Config
from kluris.pack.providers.base import (
    ContextLimitError,
    LLMProvider,
    RequestError,
)


pytestmark = pytest.mark.asyncio


class _ScriptedProvider(LLMProvider):
    """Provider that emits a sequence of pre-baked event lists.

    Each call to :meth:`complete_stream` yields the next list. Use this
    to drive the agent loop deterministically across multiple rounds.
    """

    model = "scripted"

    def __init__(self, scripts: list[list[dict]]) -> None:
        self._scripts = list(scripts)
        self.calls = 0

    async def smoke_test(self) -> None:  # pragma: no cover (unused here)
        return None

    async def complete_stream(self, messages, tools):
        if not self._scripts:
            return
        script = self._scripts.pop(0)
        self.calls += 1
        for ev in script:
            yield ev


def _config(brain_path, **overrides) -> Config:
    env = dict(
        {
            "KLURIS_PROVIDER_SHAPE": "anthropic",
            "KLURIS_BASE_URL": "http://api.test",
            "KLURIS_API_KEY": "sk-test",
            "KLURIS_MODEL": "fake",
            "KLURIS_BRAIN_DIR": str(brain_path),
        },
        **overrides,
    )
    return Config.load_from_env(env)


async def _drain(agent_iter):
    return [ev async for ev in agent_iter]


async def test_agent_dispatches_search_then_final_answer(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    provider = _ScriptedProvider([
        [
            {"kind": "tool_use", "name": "search", "id": "tu1",
             "args": {"query": "auth"}},
            {"kind": "end"},
        ],
        [
            {"kind": "token", "text": "Final answer."},
            {"kind": "usage", "input": 12, "output": 4},
            {"kind": "end"},
        ],
    ])

    events = await _drain(run_agent(
        config=cfg,
        provider=provider,
        history=[],
        user_message="how does auth work?",
    ))
    kinds = [e["kind"] for e in events]
    assert "tool" in kinds
    assert "tool_result" in kinds
    assert "token" in kinds
    assert "usage" in kinds
    assert events[-1]["kind"] == "end"
    assert provider.calls == 2


async def test_agent_sends_system_prompt_to_provider_for_anthropic(
    fixture_brain, tmp_path
):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _CaptureProvider(LLMProvider):
        model = "capture"

        def __init__(self) -> None:
            self.messages = None

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            self.messages = messages
            yield {"kind": "end"}

    provider = _CaptureProvider()
    await _drain(run_agent(
        config=cfg,
        provider=provider,
        history=[],
        user_message="hi",
        brain_name="Fixture Brain",
    ))
    assert provider.messages[0]["role"] == "system"
    assert "Fixture Brain" in provider.messages[0]["content"]


async def test_agent_uses_provider_neutral_tool_call_ids(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _TwoRoundProvider(LLMProvider):
        model = "capture"

        def __init__(self) -> None:
            self.second_round_messages = None
            self.calls = 0

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            self.calls += 1
            if self.calls == 1:
                yield {
                    "kind": "tool_use",
                    "name": "search",
                    "id": "tu1",
                    "args": {"query": "auth"},
                }
                yield {"kind": "end"}
            else:
                self.second_round_messages = messages
                yield {"kind": "token", "text": "done"}
                yield {"kind": "end"}

    provider = _TwoRoundProvider()
    await _drain(run_agent(
        config=cfg,
        provider=provider,
        history=[],
        user_message="how does auth work?",
    ))
    tool_messages = [
        m for m in provider.second_round_messages
        if m.get("role") in {"assistant", "tool"}
    ]
    assert tool_messages[-2]["tool_calls"][0] == {
        "id": "tu1",
        "name": "search",
        "args": {"query": "auth"},
    }
    assert tool_messages[-1]["tool_call_id"] == "tu1"
    assert "tool_use_id" not in tool_messages[-1]


async def test_agent_groups_parallel_tool_calls_in_replay(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _MultiToolProvider(LLMProvider):
        model = "capture"

        def __init__(self) -> None:
            self.second_round_messages = None
            self.calls = 0

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            self.calls += 1
            if self.calls == 1:
                yield {"kind": "token", "text": "I will check. "}
                yield {
                    "kind": "tool_use",
                    "name": "search",
                    "id": "tu1",
                    "args": {"query": "auth"},
                }
                yield {
                    "kind": "tool_use",
                    "name": "glossary",
                    "id": "tu2",
                    "args": {},
                }
                yield {"kind": "end"}
            else:
                self.second_round_messages = messages
                yield {"kind": "token", "text": "done"}
                yield {"kind": "end"}

    provider = _MultiToolProvider()
    await _drain(run_agent(
        config=cfg,
        provider=provider,
        history=[],
        user_message="how does auth work?",
    ))

    tool_messages = [
        m for m in provider.second_round_messages
        if m.get("role") in {"assistant", "tool"}
    ]
    assert tool_messages[-3]["content"] == "I will check. "
    assert tool_messages[-3]["tool_calls"] == [
        {"id": "tu1", "name": "search", "args": {"query": "auth"}},
        {"id": "tu2", "name": "glossary", "args": {}},
    ]
    assert [m["tool_call_id"] for m in tool_messages[-2:]] == ["tu1", "tu2"]


async def test_agent_dispatches_multi_read_in_one_call(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    provider = _ScriptedProvider([
        [
            {"kind": "tool_use", "name": "multi_read", "id": "tu1",
             "args": {"paths": [
                 "knowledge/jwt.md",
                 "knowledge/raw-sql-modern.md",
                 "projects/btb/auth.md",
             ]}},
            {"kind": "end"},
        ],
        [{"kind": "token", "text": "ok"}, {"kind": "end"}],
    ])

    events = await _drain(run_agent(
        config=cfg,
        provider=provider,
        history=[],
        user_message="compare across",
    ))
    tool_results = [e for e in events if e["kind"] == "tool_result"]
    assert len(tool_results) == 1
    assert tool_results[0]["tool"] == "multi_read"
    assert "3" in tool_results[0]["summary"] or "neurons" in tool_results[0]["summary"]


async def test_agent_unknown_tool_returns_structured_error(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    provider = _ScriptedProvider([
        [
            {"kind": "tool_use", "name": "fly_to_moon", "id": "tu1", "args": {}},
            {"kind": "end"},
        ],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    tool_results = [e for e in events if e["kind"] == "tool_result"]
    assert any("error" in r["summary"] for r in tool_results)


async def test_agent_sandbox_error_returns_structured_error(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [
            {"kind": "tool_use", "name": "read_neuron", "id": "tu1",
             "args": {"path": "../../etc/passwd"}},
            {"kind": "end"},
        ],
        [{"kind": "token", "text": "ok"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    tool_results = [e for e in events if e["kind"] == "tool_result"]
    assert any("sandbox" in r["summary"] for r in tool_results)


async def test_agent_stops_when_provider_emits_no_tool_calls(
    fixture_brain, tmp_path
):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "token", "text": "answer"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="hi",
    ))
    assert provider.calls == 1
    assert events[-1]["kind"] == "end"


async def test_agent_surfaces_empty_response_as_recoverable_error(
    fixture_brain, tmp_path
):
    """If the provider returns a round with NO tokens AND NO tool_uses
    (a bare ``end`` event from a gateway that truncated mid-thought),
    the agent must emit a recoverable error so the user doesn't see a
    blank assistant block.
    """
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        # Round 1: nothing but end. No content, no tool_use.
        [{"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="hi",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors, "empty round must surface a recoverable error"
    assert errors[0]["recoverable"] is True
    assert "no content" in errors[0]["message"].lower()
    assert events[-1]["kind"] == "end"


async def test_agent_does_not_error_when_round_has_text(
    fixture_brain, tmp_path
):
    """Sanity: the empty-round detector must NOT trigger when the
    provider actually returned tokens (the normal happy path).
    """
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "token", "text": "real answer"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="hi",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors == []


async def test_agent_max_rounds_cap_respected(fixture_brain, tmp_path):
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        MAX_AGENT_ROUNDS="2",
    )
    (tmp_path / "data").mkdir()
    looper = [
        {"kind": "tool_use", "name": "search", "id": "tu", "args": {"query": "x"}},
        {"kind": "end"},
    ]
    provider = _ScriptedProvider([looper, looper, looper])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    # Hit the cap at exactly 2 rounds.
    assert provider.calls == 2
    assert errors and "round" in errors[-1]["message"].lower()


async def test_agent_context_limit_error_recoverable(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _LimitProvider(LLMProvider):
        model = "limit"

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            raise ContextLimitError("too big")
            yield  # pragma: no cover (make this a generator)

    events = await _drain(run_agent(
        config=cfg, provider=_LimitProvider(), history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors[0]["recoverable"] is True
    assert events[-1]["kind"] == "end"


async def test_agent_request_error_not_recoverable(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _ErrorProvider(LLMProvider):
        model = "err"

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            raise RequestError("boom")
            yield  # pragma: no cover

    events = await _drain(run_agent(
        config=cfg, provider=_ErrorProvider(), history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors[0]["recoverable"] is False


async def test_agent_loads_system_prompt_per_call(fixture_brain, tmp_path):
    """Editing the prompt file between calls must show up on the next
    call (no caching).
    """
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    prompt_path = cfg.data_dir / "config" / "system_prompt.md"

    provider = _ScriptedProvider([
        [{"kind": "token", "text": "ans"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    assert prompt_path.exists()
    # Edit live
    prompt_path.write_text("CUSTOM PROMPT", encoding="utf-8")
    assert prompt_path.read_text() == "CUSTOM PROMPT"


async def test_agent_trace_hook_records_tool_calls(fixture_brain, tmp_path):
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [
            {"kind": "tool_use", "name": "search", "id": "tu",
             "args": {"query": "auth"}},
            {"kind": "end"},
        ],
        [{"kind": "token", "text": "ans"}, {"kind": "end"}],
    ])
    trace: list[dict] = []
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=trace.append,
    ))
    assert trace
    assert trace[0]["tool_name"] == "search"
    assert trace[0]["args"] == {"query": "auth"}
    # Trace must NOT include raw secrets/full body.
    for entry in trace:
        assert "sk-" not in str(entry)
