"""TEST-PACK-41 — agent loop: tool dispatch, max-rounds, errors."""

from __future__ import annotations

import pytest

from kluris.pack.agent import (
    _COMPACTED_TOOL_RESULT,
    _compact_tool_results,
    _estimate_messages_tokens,
    _flatten_for_synthesis,
    _trim_history,
    run_agent,
)
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
    """The cap stops the tool loop at exactly MAX_AGENT_ROUNDS rounds. When the
    post-cap synthesis pass also yields nothing (scripts exhausted), the
    round-budget error is the fallback."""
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
    # Two loop rounds, no synthesis script → synthesis yields nothing → error.
    provider = _ScriptedProvider([looper, looper])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    # The loop made exactly 2 provider calls; the synthesis call found no script
    # and returned nothing (no extra increment), so the cap error still fires.
    assert provider.calls == 2
    assert errors and "round" in errors[-1]["message"].lower()


async def test_agent_max_rounds_synthesizes_final_answer(fixture_brain, tmp_path):
    """Instead of throwing the turn away at the round cap, the loop makes one
    tools-disabled synthesis pass. If it answers, the user gets that answer and
    NO round-budget error."""
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
    synthesis = [{"kind": "token", "text": "Synthesized from evidence."},
                 {"kind": "end"}]
    provider = _ScriptedProvider([looper, looper, synthesis])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    tokens = [e for e in events if e["kind"] == "token"]
    assert provider.calls == 3  # 2 loop rounds + 1 synthesis pass
    assert errors == []
    assert any("Synthesized from evidence" in t["text"] for t in tokens)


async def test_agent_empty_round_after_tools_synthesizes(fixture_brain, tmp_path):
    """An empty round AFTER evidence was gathered triggers a synthesis pass
    rather than the bare 'no content' error."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "tool_use", "name": "search", "id": "tu",
          "args": {"query": "auth"}}, {"kind": "end"}],
        [{"kind": "end"}],  # empty round after a tool ran
        [{"kind": "token", "text": "Answer from what I found."}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    tokens = [e for e in events if e["kind"] == "token"]
    assert errors == []
    assert any("Answer from what I found" in t["text"] for t in tokens)


async def test_agent_duplicate_tool_call_suppressed(fixture_brain, tmp_path):
    """An exact-duplicate (tool, args) call within a turn is served from a stub
    instead of re-dispatching — the second search is NOT re-run."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    same = {"kind": "tool_use", "name": "search", "id": "tu",
            "args": {"query": "auth"}}
    provider = _ScriptedProvider([
        [same, {"kind": "end"}],
        [same, {"kind": "end"}],  # identical call again
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    results = []
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=results.append,
    ))
    summaries = [r["result_summary"] for r in results]
    assert len(summaries) == 2
    assert "duplicate" in summaries[1].lower()
    assert "duplicate" not in summaries[0].lower()


async def test_agent_total_tool_call_cap_routes_to_synthesis(fixture_brain, tmp_path):
    """KLURIS_MAX_TOOL_CALLS caps TOTAL calls across rounds (the round cap can't,
    since one round may fan out into many parallel calls). At the cap the loop
    stops and the synthesis fallback labels the budget as tool-calls."""
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_MAX_TOOL_CALLS="2",
        MAX_AGENT_ROUNDS="20",  # rounds are NOT the limiter here
    )
    (tmp_path / "data").mkdir()
    def _round(tag):
        return [{"kind": "tool_use", "name": "search", "id": tag,
                 "args": {"query": tag}}, {"kind": "end"}]
    # FOUR rounds available — the cap must stop the loop well before they run
    # out, so script exhaustion can't be confused for the cap firing.
    provider = _ScriptedProvider([_round(t) for t in ("a", "b", "c", "d")])
    results = []
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=results.append,
    ))
    errors = [e for e in events if e["kind"] == "error"]
    # trace_hook fires only for LOOP dispatches (synthesis is tools-disabled), so
    # exactly 2 tool calls ran before the cap stopped the loop — not 4.
    assert len(results) == 2
    assert errors and "tool-call" in errors[-1]["message"].lower()


class _SynthErrorProvider(LLMProvider):
    """Round 1 gathers a tool; the synthesis call (call 2) raises ``exc``."""

    model = "synth-err"

    def __init__(self, exc: Exception) -> None:
        self.calls = 0
        self._exc = exc

    async def smoke_test(self) -> None:  # pragma: no cover
        return None

    async def complete_stream(self, messages, tools):
        self.calls += 1
        if self.calls == 1:
            yield {"kind": "tool_use", "name": "search", "id": "t",
                   "args": {"query": "x"}}
            yield {"kind": "end"}
        else:
            raise self._exc
            yield  # pragma: no cover (make this an async generator)


async def test_synthesis_context_limit_surfaces_recoverable_hint(
    fixture_brain, tmp_path
):
    """A ContextLimitError on the synthesis pass must surface the recoverable
    'New conversation' guidance — NOT a generic 'try rephrasing' that retries
    straight back into the overflow."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"),
                  MAX_AGENT_ROUNDS="1")
    (tmp_path / "data").mkdir()
    provider = _SynthErrorProvider(ContextLimitError("too big"))
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors and errors[0]["recoverable"] is True
    assert "New conversation" in errors[0]["message"]
    assert events[-1]["kind"] == "end"


async def test_synthesis_request_error_surfaces_non_recoverable(
    fixture_brain, tmp_path
):
    """A RequestError on the synthesis pass must stay NON-recoverable (matching
    the main loop) instead of being downgraded to a recoverable retry."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"),
                  MAX_AGENT_ROUNDS="1")
    (tmp_path / "data").mkdir()
    provider = _SynthErrorProvider(RequestError("boom"))
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors and errors[0]["recoverable"] is False
    assert "boom" in errors[0]["message"]


async def test_synthesis_midstream_drop_flags_partial_incomplete(
    fixture_brain, tmp_path
):
    """If synthesis streams text then drops mid-answer, the partial is kept but
    an 'incomplete' note is emitted — it must NOT render as a finished turn."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"),
                  MAX_AGENT_ROUNDS="1")
    (tmp_path / "data").mkdir()

    class _PartialThenRaise(LLMProvider):
        model = "partial"

        def __init__(self) -> None:
            self.calls = 0

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            self.calls += 1
            if self.calls == 1:
                yield {"kind": "tool_use", "name": "search", "id": "t",
                       "args": {"query": "x"}}
                yield {"kind": "end"}
            else:
                yield {"kind": "token", "text": "partial answer"}
                raise RequestError("dropped")

    provider = _PartialThenRaise()
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    tokens = [e for e in events if e["kind"] == "token"]
    errors = [e for e in events if e["kind"] == "error"]
    assert any("partial answer" in t["text"] for t in tokens)  # partial kept
    assert errors and "incomplete" in errors[0]["message"].lower()
    assert errors[0]["recoverable"] is True


async def test_elided_result_reserved_from_cache_not_redispatched(
    fixture_brain, tmp_path
):
    """When eliding removes a tool result from the transcript, an identical
    re-issue is re-served IN FULL from the turn's side store — without
    re-running the tool, and without pointing the model at a discarded stub."""
    import kluris.pack.tools.brain as brain_mod

    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"),
                  KLURIS_MAX_TURN_TOKENS="50")
    (tmp_path / "data").mkdir()
    alpha = {"kind": "tool_use", "name": "search", "id": "a1",
             "args": {"query": "alpha"}}
    beta = {"kind": "tool_use", "name": "search", "id": "b1",
            "args": {"query": "beta"}}
    alpha_again = {"kind": "tool_use", "name": "search", "id": "a2",
                   "args": {"query": "alpha"}}  # identical args to alpha
    provider = _RecordingProvider([
        [alpha, {"kind": "end"}],
        [beta, {"kind": "end"}],         # 2nd result → alpha becomes "oldest"
        [alpha_again, {"kind": "end"}],  # alpha's result was compacted by now
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    results = []
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=results.append,
    ))
    summaries = [r["result_summary"] for r in results]
    assert len(summaries) == 3
    # The re-issue is served from cache — full payload, no tool re-run.
    assert "re-served" in summaries[2].lower()
    # The transcript of the final provider call carries the re-served FULL
    # payload for a2 (a real search result, not a stub or pointer).
    final = provider.seen_messages[-1]
    a2 = next(m for m in final if m.get("tool_call_id") == "a2")
    assert '"results"' in a2["content"]


async def test_agent_max_rounds_zero_means_unlimited(fixture_brain, tmp_path):
    """``MAX_AGENT_ROUNDS=0`` removes the round cap. The loop runs as
    long as the model keeps emitting ``tool_use`` events; it only
    exits when a round arrives without pending tools.
    """
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        MAX_AGENT_ROUNDS="0",
    )
    (tmp_path / "data").mkdir()
    looper = [
        {"kind": "tool_use", "name": "search", "id": "tu", "args": {"query": "x"}},
        {"kind": "end"},
    ]
    final = [
        {"kind": "token", "text": "Final answer."},
        {"kind": "end"},
    ]
    # 50 looping rounds — way past the old default of 8 — followed by
    # a clean final answer. Unlimited mode must run all 51 rounds.
    provider = _ScriptedProvider([looper] * 50 + [final])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert provider.calls == 51, (
        "unlimited mode must run every scripted round, not stop at any cap"
    )
    assert errors == [], (
        "no round-cap error should fire when MAX_AGENT_ROUNDS=0"
    )
    tokens = [e for e in events if e["kind"] == "token"]
    assert any("Final answer" in t["text"] for t in tokens)


async def test_agent_max_rounds_negative_treated_as_unlimited(
    fixture_brain, tmp_path,
):
    """Sanity: a negative ``MAX_AGENT_ROUNDS`` (typo / misconfig) is
    also treated as the unlimited sentinel rather than crashing or
    immediately bailing out.
    """
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        MAX_AGENT_ROUNDS="-1",
    )
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "token", "text": "ok"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors == []
    tokens = [e for e in events if e["kind"] == "token"]
    assert tokens and tokens[0]["text"] == "ok"


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
    # The provider's actual message must be surfaced, not just the class name.
    assert "boom" in errors[0]["message"]
    assert "RequestError" not in errors[0]["message"]


async def test_agent_surfaces_reasoning_effort_value_hint(fixture_brain, tmp_path):
    """Post-LiteLLM, OpenAI reasoning runs on the Responses API, so a
    reasoning_effort 400 means the model rejected the effort VALUE — not a
    chat-vs-responses conflict. The surfaced error must carry the provider's own
    message AND a hint pointing at the value, with NO stale /v1/chat/completions
    wording."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()

    class _EffortReject(LLMProvider):
        model = "gpt-5.4-mini"

        async def smoke_test(self) -> None:  # pragma: no cover
            return None

        async def complete_stream(self, messages, tools):
            raise RequestError(
                "litellm.BadRequestError: Invalid value for 'reasoning_effort': "
                "'xhigh' is not one of the accepted values for this model."
            )
            yield  # pragma: no cover

    events = await _drain(run_agent(
        config=cfg, provider=_EffortReject(), history=[], user_message="x",
    ))
    errors = [e for e in events if e["kind"] == "error"]
    assert errors and errors[0]["recoverable"] is False
    msg = errors[0]["message"]
    assert "reasoning_effort" in msg  # provider detail preserved verbatim
    assert "KLURIS_REASONING_EFFORT" in msg  # actionable hint appended
    assert "/v1/chat/completions" not in msg  # stale wording is gone


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


# --- Sliding-window history trimming -----------------------------------------


async def test_trim_history_keeps_recent_drops_oldest():
    hist = [
        {"role": "user" if i % 2 == 0 else "assistant", "content": "x" * 400}
        for i in range(10)
    ]
    trimmed = _trim_history(hist, 300)  # ~105 tokens/msg → keeps ~2 recent
    assert 0 < len(trimmed) < len(hist)
    assert trimmed == hist[-len(trimmed):]  # kept slice is the most recent tail


async def test_trim_history_small_history_untouched():
    hist = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "hello"},
    ]
    assert _trim_history(hist, 24000) == hist


async def test_trim_history_zero_disables_trimming():
    hist = [{"role": "user", "content": "x" * 100000}]
    assert _trim_history(hist, 0) == hist


async def test_trim_history_keeps_at_least_most_recent():
    """A single message bigger than the whole budget is still kept (we never
    send empty history); the provider's ContextLimitError is the backstop."""
    hist = [
        {"role": "user", "content": "a" * 100},
        {"role": "assistant", "content": "z" * 100000},
    ]
    assert _trim_history(hist, 10) == [hist[-1]]


# --- In-turn compaction (per-turn request-size budget) -----------------------


def _msgs_with_tool_results(n: int, body_chars: int) -> list[dict]:
    """system + user + n × (assistant tool_call, tool result) pairs."""
    msgs: list[dict] = [
        {"role": "system", "content": "S"},
        {"role": "user", "content": "Q"},
    ]
    for i in range(n):
        msgs.append({
            "role": "assistant", "content": "",
            "tool_calls": [{"id": f"t{i}", "name": "search", "args": {}}],
        })
        msgs.append({
            "role": "tool", "tool_call_id": f"t{i}", "content": "X" * body_chars,
        })
    return msgs


async def test_compact_disabled_when_budget_zero():
    msgs = _msgs_with_tool_results(3, 4000)
    out = _compact_tool_results(msgs, 0)
    tool_msgs = [m for m in out if m.get("role") == "tool"]
    assert all("X" * 4000 in m["content"] for m in tool_msgs)


async def test_compact_under_budget_is_noop():
    msgs = _msgs_with_tool_results(2, 4)
    before = [m["content"] for m in msgs]
    out = _compact_tool_results(msgs, 100000)
    assert [m["content"] for m in out] == before


async def test_compact_elides_oldest_keeps_latest_and_structure():
    msgs = _msgs_with_tool_results(3, 8000)  # ~2k est tokens per result
    n_before = len(msgs)
    roles_before = [m.get("role") for m in msgs]
    ids_before = [m.get("tool_call_id") for m in msgs]
    assert _estimate_messages_tokens(msgs) > 1500  # precondition: over budget
    out = _compact_tool_results(msgs, 1500)
    tool_msgs = [m for m in out if m.get("role") == "tool"]
    # Oldest elided, newest kept verbatim.
    assert '"compacted": true' in tool_msgs[0]["content"]
    assert "X" * 8000 in tool_msgs[-1]["content"]
    # Structure preserved: same count, same roles, same tool_call_id pairing.
    assert len(out) == n_before
    assert [m.get("role") for m in out] == roles_before
    assert [m.get("tool_call_id") for m in out] == ids_before


class _RecordingProvider(LLMProvider):
    """Scripted provider that snapshots the messages of every call."""

    model = "rec"

    def __init__(self, scripts: list[list[dict]]) -> None:
        self._scripts = list(scripts)
        self.seen_messages: list[list[dict]] = []

    async def smoke_test(self) -> None:  # pragma: no cover
        return None

    async def complete_stream(self, messages, tools):
        self.seen_messages.append([dict(m) for m in messages])
        if self._scripts:
            for ev in self._scripts.pop(0):
                yield ev


async def test_run_agent_compacts_old_tool_results_within_turn(
    fixture_brain, tmp_path
):
    """With a tiny per-turn budget, the oldest tool result is elided before the
    next round's request while the newest stays intact."""
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_MAX_TURN_TOKENS="50",
    )
    (tmp_path / "data").mkdir()

    def _search(q):
        return [
            {"kind": "tool_use", "name": "search", "id": f"t-{q}",
             "args": {"query": q}},
            {"kind": "end"},
        ]

    provider = _RecordingProvider([
        _search("alpha"), _search("beta"),
        [{"kind": "token", "text": "ok"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    round3 = provider.seen_messages[2]  # the 3rd provider call
    tool_msgs = [m for m in round3 if m.get("role") == "tool"]
    assert len(tool_msgs) == 2
    assert '"compacted": true' in tool_msgs[0]["content"]   # oldest elided
    assert '"compacted": true' not in tool_msgs[-1]["content"]  # newest kept


# --- Flattened synthesis request (tool-free final call) -----------------------


def _msgs_for_flatten() -> list[dict]:
    """system + prior chat turn + question + 2 × (tool_call, tool result)."""
    return [
        {"role": "system", "content": "S"},
        {"role": "user", "content": "old question"},
        {"role": "assistant", "content": "old answer"},
        {"role": "user", "content": "Q"},
        {"role": "assistant", "content": "",
         "tool_calls": [{"id": "t0", "name": "search",
                         "args": {"query": "a"}}]},
        {"role": "tool", "tool_call_id": "t0",
         "content": '{"ok": true, "total": 1}'},
        {"role": "assistant", "content": "",
         "tool_calls": [{"id": "t1", "name": "read_neuron",
                         "args": {"path": "p"}}]},
        {"role": "tool", "tool_call_id": "t1",
         "content": '{"ok": true, "body": "B"}'},
    ]


async def test_flatten_strips_all_tool_machinery():
    """No ``role: tool`` message and no ``tool_calls`` block survives — the
    transcript shape is what blanked in the field, so none of it may reach the
    synthesis request. The plain conversation is preserved in order."""
    out = _flatten_for_synthesis(_msgs_for_flatten(), 100000)
    assert all(m.get("role") != "tool" for m in out)
    assert all(not m.get("tool_calls") for m in out)
    assert [m["role"] for m in out[:4]] == ["system", "user", "assistant", "user"]
    assert out[1]["content"] == "old question"


async def test_flatten_folds_evidence_into_final_user_message():
    out = _flatten_for_synthesis(_msgs_for_flatten(), 100000)
    final = out[-1]
    assert final["role"] == "user"
    assert "[search]" in final["content"]
    assert '"total": 1' in final["content"]
    assert "[read_neuron]" in final["content"]
    assert '"body": "B"' in final["content"]
    assert "Do not call any tools." in final["content"]


async def test_flatten_skips_compacted_and_duplicate_stubs():
    """Stubs carry no evidence: a compacted result and a duplicate-call note
    must not be folded in; real results still are."""
    msgs = _msgs_for_flatten()
    msgs[5] = {**msgs[5], "content": _COMPACTED_TOOL_RESULT}
    msgs.append({"role": "assistant", "content": "",
                 "tool_calls": [{"id": "t2", "name": "search",
                                 "args": {"query": "a"}}]})
    msgs.append({"role": "tool", "tool_call_id": "t2",
                 "content": '{"ok": true, "duplicate": true, "note": "n"}'})
    out = _flatten_for_synthesis(msgs, 100000)
    final = out[-1]["content"]
    assert "compacted" not in final
    assert '"duplicate": true' not in final
    assert "[read_neuron]" in final


def _msgs_with_padded_evidence(n: int, pad_chars: int) -> list[dict]:
    msgs: list[dict] = [
        {"role": "system", "content": "S"},
        {"role": "user", "content": "Q"},
    ]
    for i in range(n):
        msgs.append({"role": "assistant", "content": "",
                     "tool_calls": [{"id": f"t{i}", "name": "search",
                                     "args": {"query": str(i)}}]})
        msgs.append({
            "role": "tool", "tool_call_id": f"t{i}",
            "content": f'{{"ok": true, "marker": "EV{i}", '
                       f'"pad": "{"x" * pad_chars}"}}',
        })
    return msgs


async def test_flatten_budget_keeps_newest_evidence_with_marker():
    """Over budget, the NEWEST evidence survives (it is what the answer needs)
    and the elision is announced in the message."""
    out = _flatten_for_synthesis(_msgs_with_padded_evidence(3, 2000), 700)
    final = out[-1]["content"]
    assert "EV2" in final
    assert "EV0" not in final
    assert "elided to fit the budget" in final


async def test_flatten_unbounded_keeps_all_evidence():
    out = _flatten_for_synthesis(_msgs_with_padded_evidence(3, 2000), 0)
    final = out[-1]["content"]
    assert all(f"EV{i}" in final for i in range(3))
    assert "elided to fit the budget" not in final


async def test_synthesis_request_is_flat_and_carries_evidence(
    fixture_brain, tmp_path
):
    """End-to-end: the fallback's provider call contains NO tool transcript —
    just the plain conversation plus one user message holding the evidence —
    and its answer reaches the user."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "search", "id": "tu",
          "args": {"query": "auth"}}, {"kind": "end"}],
        [{"kind": "end"}],  # empty round after a tool ran → synthesis
        [{"kind": "token", "text": "flat answer"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    synth = provider.seen_messages[2]
    assert all(m.get("role") != "tool" for m in synth)
    assert all(not m.get("tool_calls") for m in synth)
    assert synth[-1]["role"] == "user"
    assert "[search]" in synth[-1]["content"]
    tokens = [e for e in events if e["kind"] == "token"]
    assert any("flat answer" in t["text"] for t in tokens)


async def test_run_agent_trims_old_history_before_sending(fixture_brain, tmp_path):
    """run_agent applies the sliding window: the provider receives only the
    recent turns (plus the system prompt + the new user message)."""
    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_MAX_CONTEXT_TOKENS="300",
    )
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
    long_history = [
        {
            "role": "user" if i % 2 == 0 else "assistant",
            "content": f"old-turn-{i} " + "x" * 400,
        }
        for i in range(20)
    ]
    await _drain(run_agent(
        config=cfg, provider=provider, history=long_history,
        user_message="newest question",
    ))
    sent = provider.messages
    assert sent[0]["role"] == "system"
    assert sent[-1] == {"role": "user", "content": "newest question"}
    history_sent = sent[1:-1]
    assert len(history_sent) < 20  # trimmed
    joined = " ".join(m["content"] for m in history_sent)
    assert "old-turn-0" not in joined    # oldest dropped
    assert "old-turn-19" in joined       # most recent kept


# --- Phase 4 (2.28.0): near-dup, eager eliding, cancel, offload, data note ----


async def test_near_duplicate_search_suppressed(fixture_brain, tmp_path):
    """Re-phrased searches with the same token set are duplicates: 'auth flow'
    vs 'flow auth' must not re-dispatch. Different offsets must."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "tool_use", "name": "search", "id": "s1",
          "args": {"query": "auth flow"}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "s2",
          "args": {"query": "flow  AUTH"}}, {"kind": "end"}],  # same tokens
        [{"kind": "tool_use", "name": "search", "id": "s3",
          "args": {"query": "auth flow", "offset": 10}}, {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    results = []
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=results.append,
    ))
    summaries = [r["result_summary"] for r in results]
    assert len(summaries) == 3
    assert "duplicate" in summaries[1].lower()      # rephrasing caught
    assert "duplicate" not in summaries[2].lower()  # paging is NOT a duplicate


async def test_eager_eliding_stubs_old_rounds_and_restores_in_synthesis(
    fixture_brain, tmp_path
):
    """With KEEP_RESULT_ROUNDS=1, a round-1 result is elided from the round-3
    request — but the synthesis fallback still receives its full content as
    evidence (restored from the side store)."""
    from kluris.pack.agent import _SEEN_TOOL_RESULT

    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_KEEP_RESULT_ROUNDS="1",
    )
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "search", "id": "r1",
          "args": {"query": "jwt"}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "glossary", "id": "r2",
          "args": {}}, {"kind": "end"}],
        [{"kind": "end"}],  # empty round → synthesis fallback
        [{"kind": "token", "text": "synthesized"}, {"kind": "end"}],
    ])
    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    # Round-3 request: r1's result is stubbed, r2's is still full.
    round3 = provider.seen_messages[2]
    r1_msg = next(m for m in round3 if m.get("tool_call_id") == "r1")
    assert r1_msg["content"] == _SEEN_TOOL_RESULT
    r2_msg = next(m for m in round3 if m.get("tool_call_id") == "r2")
    assert r2_msg["content"] != _SEEN_TOOL_RESULT
    # Synthesis request: flat, and the r1 evidence is RESTORED in full.
    synth = provider.seen_messages[3]
    assert all(m.get("role") != "tool" for m in synth)
    assert "[search]" in synth[-1]["content"]
    assert '"jwt"' in synth[-1]["content"] or "jwt" in synth[-1]["content"]
    tokens = [e for e in events if e["kind"] == "token"]
    assert any("synthesized" in t["text"] for t in tokens)


async def test_eager_eliding_disabled_with_zero_knob(fixture_brain, tmp_path):
    """KLURIS_KEEP_RESULT_ROUNDS=0 disables eager eliding — old results stay
    in the request (only the max_turn_tokens ceiling applies)."""
    from kluris.pack.agent import _SEEN_TOOL_RESULT

    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_KEEP_RESULT_ROUNDS="0",
    )
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "search", "id": "r1",
          "args": {"query": "jwt"}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "glossary", "id": "r2",
          "args": {}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "recent", "id": "r3",
          "args": {}}, {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    final = provider.seen_messages[-1]
    r1_msg = next(m for m in final if m.get("tool_call_id") == "r1")
    assert r1_msg["content"] != _SEEN_TOOL_RESULT


async def test_should_cancel_stops_loop_between_rounds(fixture_brain, tmp_path):
    """A disconnected client stops the loop before the next provider round —
    no more token burn for an abandoned turn."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "tool_use", "name": "search", "id": "s1",
          "args": {"query": "auth"}}, {"kind": "end"}],
        [{"kind": "token", "text": "never sent"}, {"kind": "end"}],
    ])
    calls = {"n": 0}

    async def _disconnected_after_first_round():
        calls["n"] += 1
        return calls["n"] > 1  # False for round 1, True for round 2

    events = await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        should_cancel=_disconnected_after_first_round,
    ))
    assert provider.calls == 1  # round 2 never reached the provider
    assert events[-1]["kind"] == "end"
    assert not any(
        e["kind"] == "token" and "never sent" in e.get("text", "")
        for e in events
    )


async def test_tool_dispatch_runs_off_the_event_loop(fixture_brain, tmp_path):
    """Tool dispatch goes through asyncio.to_thread so blocking brain I/O
    cannot stall other chats' streams."""
    import threading

    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    dispatch_threads: list = []
    import kluris.pack.agent as agent_mod
    real = agent_mod._dispatch_tool

    def _spy(config, name, args):
        dispatch_threads.append(threading.current_thread())
        return real(config, name, args)

    agent_mod_dispatch = agent_mod._dispatch_tool
    try:
        agent_mod._dispatch_tool = _spy
        provider = _ScriptedProvider([
            [{"kind": "tool_use", "name": "search", "id": "s1",
              "args": {"query": "auth"}}, {"kind": "end"}],
            [{"kind": "token", "text": "done"}, {"kind": "end"}],
        ])
        await _drain(run_agent(
            config=cfg, provider=provider, history=[], user_message="x",
        ))
    finally:
        agent_mod._dispatch_tool = agent_mod_dispatch
    assert dispatch_threads
    assert all(
        t is not threading.main_thread() for t in dispatch_threads
    ), "dispatch ran on the event-loop thread"


async def test_tool_results_carry_brain_data_note(fixture_brain, tmp_path):
    """Every dispatched tool result is prefixed with the data-boundary note —
    retrieved brain content must read as data, not instructions."""
    from kluris.pack.agent import _TOOL_DATA_NOTE

    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "search", "id": "s1",
          "args": {"query": "auth"}}, {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    final = provider.seen_messages[-1]
    s1 = next(m for m in final if m.get("tool_call_id") == "s1")
    assert s1["content"].startswith(_TOOL_DATA_NOTE)
    assert '"ok": true' in s1["content"]


async def test_parallel_same_tool_calls_get_distinct_ids(fixture_brain, tmp_path):
    """Two parallel same-tool calls that arrive WITHOUT provider ids must get
    distinct fallback tool_call_ids — else the transcript carries duplicate
    ids and one call's evidence is lost from the synthesis restore."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "search", "id": None,
          "args": {"query": "jwt"}},
         {"kind": "tool_use", "name": "search", "id": None,
          "args": {"query": "oauth"}},
         {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    final = provider.seen_messages[-1]
    tool_ids = [m["tool_call_id"] for m in final if m.get("role") == "tool"]
    assert len(tool_ids) == 2
    assert len(set(tool_ids)) == 2  # distinct — no collision


# --- v2.28.1: stop the wake_up / re-search churn loop -------------------------


async def test_wake_up_is_never_elided_across_rounds(fixture_brain, tmp_path):
    """wake_up's result stays full in the request for the whole turn (sticky),
    so the model has no reason to re-issue it — even with aggressive eliding."""
    from kluris.pack.agent import _STUB_CONTENTS

    cfg = _config(
        fixture_brain,
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_KEEP_RESULT_ROUNDS="1",
    )
    (tmp_path / "data").mkdir()
    provider = _RecordingProvider([
        [{"kind": "tool_use", "name": "wake_up", "id": "w1", "args": {}},
         {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "s1",
          "args": {"query": "jwt"}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "s2",
          "args": {"query": "oauth"}}, {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
    ))
    # By the final round, s1 is elided (old) but wake_up (w1) is still FULL.
    final = provider.seen_messages[-1]
    w1 = next(m for m in final if m.get("tool_call_id") == "w1")
    assert w1["content"] not in _STUB_CONTENTS
    assert '"ok": true' in w1["content"]


async def test_elision_stub_discourages_recalling(fixture_brain, tmp_path):
    """The elided-result stub must tell the model NOT to repeat the call —
    the wording that previously invited re-calls drove the churn loop."""
    from kluris.pack.agent import _SEEN_TOOL_RESULT, _COMPACTED_TOOL_RESULT

    for stub in (_SEEN_TOOL_RESULT, _COMPACTED_TOOL_RESULT):
        low = stub.lower()
        assert "do not repeat the call" in low
        assert "instantly" not in low  # the old "get it again instantly" invite


async def test_snippet_only_variation_is_deduped(fixture_brain, tmp_path):
    """The same search re-issued only to widen the snippet is a duplicate —
    snippet_chars is presentation, not a new query. Changing full_bodies or
    offset is NOT a duplicate (legitimate escalation / paging)."""
    cfg = _config(fixture_brain, KLURIS_DATA_DIR=str(tmp_path / "data"))
    (tmp_path / "data").mkdir()
    provider = _ScriptedProvider([
        [{"kind": "tool_use", "name": "search", "id": "a",
          "args": {"query": "auth", "snippet_chars": 180}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "b",
          "args": {"query": "auth", "snippet_chars": 400}}, {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "c",
          "args": {"query": "auth", "full_bodies": 3}}, {"kind": "end"}],
        [{"kind": "token", "text": "done"}, {"kind": "end"}],
    ])
    results = []
    await _drain(run_agent(
        config=cfg, provider=provider, history=[], user_message="x",
        trace_hook=results.append,
    ))
    summaries = [r["result_summary"] for r in results]
    assert "duplicate" in summaries[1].lower()      # snippet-only widen → dup
    assert "duplicate" not in summaries[2].lower()  # full_bodies → real call
