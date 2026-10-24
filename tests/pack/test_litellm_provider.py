"""TEST-PACK-14 — LiteLLMProvider: routing, streaming, errors, TLS, OAuth.

The single :class:`LiteLLMProvider` replaces the retired hand-rolled
``APIKeyProvider`` + ``OAuthProvider``. These tests monkeypatch
``litellm.acompletion`` to capture the outgoing kwargs and to simulate streamed
chunks + raised litellm exceptions — everything past that boundary (real OpenAI
behavior) is NOT exercised here and must be checked by hand against a live key.
"""

from __future__ import annotations

import asyncio
import json
import ssl
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import httpx
import litellm
import pytest
import respx
from litellm.types.utils import (
    ChatCompletionDeltaToolCall,
    Delta,
    Function as LFunction,
    ModelResponseStream,
    StreamingChoices,
    Usage,
)

from kluris.pack.config import Config
from kluris.pack.providers.base import AuthError, ContextLimitError, RequestError
from kluris.pack.providers.litellm_provider import (
    LiteLLMProvider,
    _PING_TOOL_CHOICE,
    _PING_TOOL_OPENAI,
    _parse_litellm_stream,
    configure_litellm,
)

pytestmark = pytest.mark.asyncio


# --- Config builders ---------------------------------------------------------


def _api_cfg(*, shape="openai", base="https://api.openai.com",
             model="gpt-5.4-mini", **extra) -> Config:
    env = {
        "KLURIS_PROVIDER_SHAPE": shape,
        "KLURIS_BASE_URL": base,
        "KLURIS_API_KEY": "sk-test-key",
        "KLURIS_MODEL": model,
        "KLURIS_BRAIN_DIR": "/app/brain",
    }
    env.update(extra)
    return Config.load_from_env(env)


def _oauth_cfg(**extra) -> Config:
    env = {
        "KLURIS_OAUTH_TOKEN_URL": "https://idp.test/token",
        "KLURIS_OAUTH_API_BASE_URL": "https://gw.test",
        "KLURIS_OAUTH_CLIENT_ID": "client-1",
        "KLURIS_OAUTH_CLIENT_SECRET": "secret-1",
        "KLURIS_MODEL": "internal-model",
        "KLURIS_BRAIN_DIR": "/app/brain",
    }
    env.update(extra)
    return Config.load_from_env(env)


# --- Fake litellm.acompletion ------------------------------------------------


class _FakeLiteLLM:
    """Records the kwargs of every ``acompletion`` call and replays a scripted
    stream / non-stream response or a raised exception."""

    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self.stream_chunks: list[Any] = []
        self.nonstream_response: Any = {
            "choices": [{"index": 0, "message": {"content": "pong"}}]
        }
        self.raise_on_call: Exception | None = None
        self.raise_in_stream: Exception | None = None

    async def __call__(self, **kwargs: Any) -> Any:
        self.calls.append(kwargs)
        if self.raise_on_call is not None:
            raise self.raise_on_call
        if kwargs.get("stream"):
            chunks = list(self.stream_chunks)
            exc = self.raise_in_stream

            async def _gen():
                for chunk in chunks:
                    yield chunk
                if exc is not None:
                    raise exc

            return _gen()
        return self.nonstream_response

    @property
    def last(self) -> dict[str, Any]:
        return self.calls[-1]


@pytest.fixture
def fake_litellm(monkeypatch) -> _FakeLiteLLM:
    fake = _FakeLiteLLM()
    monkeypatch.setattr(litellm, "acompletion", fake)
    return fake


# --- Chunk + exception + drain helpers ---------------------------------------


def _c_text(text: str, finish: str | None = None) -> ModelResponseStream:
    return ModelResponseStream(choices=[StreamingChoices(
        index=0, delta=Delta(content=text, role="assistant"), finish_reason=finish,
    )])


def _c_tool(*, index=0, id=None, name=None, args=None, finish=None) -> ModelResponseStream:
    return ModelResponseStream(choices=[StreamingChoices(
        index=0,
        delta=Delta(tool_calls=[ChatCompletionDeltaToolCall(
            index=index, id=id, type="function",
            function=LFunction(name=name, arguments=args),
        )]),
        finish_reason=finish,
    )])


def _c_usage(prompt: int, completion: int) -> ModelResponseStream:
    chunk = ModelResponseStream(choices=[StreamingChoices(
        index=0, delta=Delta(), finish_reason=None,
    )])
    chunk.usage = Usage(
        prompt_tokens=prompt, completion_tokens=completion,
        total_tokens=prompt + completion,
    )
    return chunk


def _exc(name: str, message: str = "boom") -> Exception:
    cls = getattr(litellm, name)
    if name == "PermissionDeniedError":
        req = httpx.Request("POST", "https://api.test/v1")
        return cls(message=message, llm_provider="openai", model="m",
                   response=httpx.Response(403, request=req))
    return cls(message=message, llm_provider="openai", model="m")


async def _drain(aiter) -> list[dict[str, Any]]:
    return [event async for event in aiter]


# ============================================================================
# Routing + outgoing kwargs (Acceptance C / F)
# ============================================================================


async def test_openai_proper_routes_responses_with_full_kwargs(fake_litellm):
    cfg = _api_cfg(base="https://api.openai.com", KLURIS_REASONING_EFFORT="high")
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop"), _c_usage(1, 1)]
    sentinel_tools = [{"type": "function", "function": {"name": "search"}}]

    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], sentinel_tools,
    ))

    last = fake_litellm.last
    assert last["model"] == "openai/responses/gpt-5.4-mini"
    assert last["store"] is False
    assert last["reasoning_effort"] == "high"
    assert last["stream"] is True
    assert last["stream_options"] == {"include_usage": True}
    assert last["max_tokens"] == cfg.max_output_tokens
    assert last["api_key"] == "sk-test-key"
    assert last["tools"] == sentinel_tools
    # OpenAI-proper resolves api_base to None → omitted entirely.
    assert "api_base" not in last


async def test_configure_litellm_silences_pydantic_usage_serializer_warning():
    """On the Responses path LiteLLM dumps a chat-completions-shaped usage dict
    into its ResponseAPIUsage model, so pydantic emits a benign 'Pydantic
    serializer warnings' UserWarning on every completion. configure_litellm must
    silence THAT specific one (scoped to the ResponseAPIUsage text) while leaving
    unrelated warnings — including OTHER pydantic serializer warnings — visible."""
    import warnings

    cfg = _api_cfg()
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")  # baseline: record everything
        configure_litellm(cfg)           # prepends the scoped "ignore" filter
        warnings.warn(
            "Pydantic serializer warnings:\n  "
            "PydanticSerializationUnexpectedValue(Expected `ResponseAPIUsage`)",
            UserWarning,
        )
        # A pydantic serializer warning about something ELSE must NOT be muted.
        warnings.warn(
            "Pydantic serializer warnings:\n  "
            "PydanticSerializationUnexpectedValue(Expected `SomeOtherModel`)",
            UserWarning,
        )
        warnings.warn("an unrelated UserWarning", UserWarning)

    messages = [str(w.message) for w in caught]
    assert not any("ResponseAPIUsage" in m for m in messages)
    assert any("SomeOtherModel" in m for m in messages)
    assert any("an unrelated UserWarning" in m for m in messages)


async def test_anthropic_gates_off_openai_only_params(fake_litellm):
    cfg = _api_cfg(shape="anthropic", base="https://api.anthropic.com",
                   model="claude-opus-4-7", KLURIS_REASONING_EFFORT="high")
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]

    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))

    last = fake_litellm.last
    assert last["model"] == "anthropic/claude-opus-4-7"
    assert last["api_base"] == "https://api.anthropic.com"
    assert last["max_tokens"] == cfg.max_output_tokens
    # LiteLLM does not accept these for Anthropic — they must NOT be sent.
    assert "store" not in last
    assert "stream_options" not in last
    assert "reasoning_effort" not in last


async def test_openai_gateway_routes_chat_completions(fake_litellm):
    cfg = _api_cfg(base="https://openrouter.ai/api", model="gpt-4o")
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]

    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))

    last = fake_litellm.last
    assert last["model"] == "openai/gpt-4o"
    # /v1 restored so LiteLLM's leaf-only append lands on /v1/chat/completions.
    assert last["api_base"] == "https://openrouter.ai/api/v1"
    # store goes to OpenAI proper only: a gateway may reject an unknown field.
    assert "store" not in last
    assert last["stream_options"] == {"include_usage": True}


async def test_openai_gateway_responses_opt_in_omits_store(fake_litellm):
    cfg = _api_cfg(base="https://gw.example", model="gpt-5",
                   KLURIS_USE_RESPONSES_API="1")
    await LiteLLMProvider(cfg).smoke_test()
    last = fake_litellm.last
    assert last["model"] == "openai/responses/gpt-5"
    assert last["api_base"] == "https://gw.example/v1"
    assert "store" not in last


async def test_temperature_forwarded_only_when_set(fake_litellm):
    cfg = _api_cfg(KLURIS_TEMPERATURE="0.4")
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))
    assert fake_litellm.last["temperature"] == 0.4

    cfg2 = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(LiteLLMProvider(cfg2).complete_stream([{"role": "user", "content": "q"}], []))
    assert "temperature" not in fake_litellm.last


async def test_reasoning_effort_omitted_when_unset(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))
    assert "reasoning_effort" not in fake_litellm.last


async def test_assistant_tool_calls_converted_to_openai_shape(fake_litellm):
    """The generic transcript (role/content/tool_calls/tool_call_id) is
    rendered into OpenAI message shape before the call."""
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("ok", finish="stop")]
    history = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "", "tool_calls": [
            {"id": "tc1", "name": "search", "args": {"query": "x"}}]},
        {"role": "tool", "tool_call_id": "tc1", "content": "{}"},
    ]
    await _drain(LiteLLMProvider(cfg).complete_stream(history, []))
    sent = fake_litellm.last["messages"]
    assistant = next(m for m in sent if m["role"] == "assistant")
    assert assistant["tool_calls"][0]["function"]["name"] == "search"
    assert assistant["tool_calls"][0]["function"]["arguments"] == '{"query": "x"}'
    tool_msg = next(m for m in sent if m["role"] == "tool")
    assert tool_msg["tool_call_id"] == "tc1"


async def test_messages_for_openai_system_passthrough_and_tool_use_id_fallback(fake_litellm):
    """System messages survive as role=system (LiteLLM lifts them for
    Anthropic), and a tool message keyed on the legacy tool_use_id is mapped to
    tool_call_id."""
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("ok", finish="stop")]
    history = [
        {"role": "system", "content": "SYS"},
        {"role": "user", "content": "hi"},
        {"role": "tool", "tool_use_id": "legacy-id", "content": "{}"},
    ]
    await _drain(LiteLLMProvider(cfg).complete_stream(history, []))
    sent = fake_litellm.last["messages"]
    assert sent[0] == {"role": "system", "content": "SYS"}
    tool_msg = next(m for m in sent if m["role"] == "tool")
    assert tool_msg["tool_call_id"] == "legacy-id"


# ============================================================================
# Streaming behavior (Acceptance B) — real litellm chunk objects
# ============================================================================


async def test_stream_text_and_tool_calls_both_surface(fake_litellm):
    """The v1.87.0-fixed bug: text AND tool_calls in the same round both
    surface. Asserted on REAL ModelResponseStream objects (production path)."""
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [
        _c_text("Look"),
        _c_tool(id="call_1", name="search", args='{"que'),
        _c_tool(name=None, args='ry":"auth"}', finish="tool_calls"),
        _c_usage(42, 8),
    ]
    events = await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    assert {"kind": "token", "text": "Look"} in events
    tool_uses = [e for e in events if e["kind"] == "tool_use"]
    assert tool_uses == [{"kind": "tool_use", "name": "search",
                          "id": "call_1", "args": {"query": "auth"}}]
    usage = [e for e in events if e["kind"] == "usage"]
    assert usage == [{"kind": "usage", "input": 42, "output": 8}]
    assert events[-1] == {"kind": "end"}


async def test_stream_zero_usage_fallback(fake_litellm):
    """When the endpoint ignores include_usage, a zero-usage event is still
    emitted so the UI footer ticks."""
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("answer", finish="stop")]
    events = await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    assert {"kind": "usage", "input": 0, "output": 0} in events
    assert events[-1] == {"kind": "end"}


async def test_parse_stream_tolerates_plain_dicts():
    """The parser must also handle dict chunks (keeps it trivially testable)."""
    async def gen():
        yield {"choices": [{"delta": {"content": "hey"}}]}
        yield {"choices": [], "usage": {"prompt_tokens": 3, "completion_tokens": 2}}
    events = [e async for e in _parse_litellm_stream(gen())]
    assert events == [
        {"kind": "token", "text": "hey"},
        {"kind": "usage", "input": 3, "output": 2},
        {"kind": "end"},
    ]


async def test_debug_stream_emits_redacted_summary_line(fake_litellm, capsys):
    """KLURIS_DEBUG_STREAM=1 writes one stderr summary per stream — the supported
    way to diagnose an empty 'model returned no content' turn. The failing shape
    (no text, no tool, finish='stop') is exactly what the line surfaces. The
    summary line itself carries only counts/booleans/finish_reason; empty
    streams additionally dump payload-free chunk shapes (covered below)."""
    cfg = _api_cfg(KLURIS_DEBUG_STREAM="1")
    fake_litellm.stream_chunks = [_c_text("", finish="stop"), _c_usage(120, 40)]
    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    err = capsys.readouterr().err
    assert "kluris-pack: stream" in err
    assert "text=False" in err
    assert "tool=False" in err
    assert "finish='stop'" in err


async def test_debug_stream_off_by_default_writes_nothing(fake_litellm, capsys):
    """Without the knob the diagnostic line must not appear — it is opt-in."""
    cfg = _api_cfg()  # KLURIS_DEBUG_STREAM unset → False
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    assert "kluris-pack: stream" not in capsys.readouterr().err


async def test_debug_stream_dumps_chunk_shapes_on_empty_stream(fake_litellm, capsys):
    """A stream that ends with NO text and NO tool call additionally dumps the
    SHAPE of each chunk (field presence + lengths) — enough to see what an
    empty completion actually carried, without any payload."""
    cfg = _api_cfg(KLURIS_DEBUG_STREAM="1")
    fake_litellm.stream_chunks = [_c_text("", finish="stop"), _c_usage(120, 40)]
    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    err = capsys.readouterr().err
    assert "kluris-pack: empty-stream chunk 1/2:" in err
    assert "kluris-pack: empty-stream chunk 2/2:" in err
    assert "finish='stop'" in err
    assert "usage=present" in err


async def test_debug_stream_empty_dump_never_leaks_reasoning_prose(
    fake_litellm, capsys
):
    """The realistic empty-completion shape for gpt-5-family models is a stream
    of ``reasoning_content`` deltas with no visible ``content`` — hidden
    chain-of-thought. The dump must report its LENGTH, never its text."""
    cot = "SECRET hidden reasoning the model never said out loud"
    chunk = ModelResponseStream(choices=[StreamingChoices(
        index=0,
        delta=Delta(content=None, reasoning_content=cot, role="assistant"),
        finish_reason="stop",
    )])
    cfg = _api_cfg(KLURIS_DEBUG_STREAM="1")
    fake_litellm.stream_chunks = [chunk]
    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    err = capsys.readouterr().err
    assert "kluris-pack: empty-stream chunk 1/1:" in err
    assert f"reasoning_content={len(cot)}ch" in err
    assert "SECRET" not in err  # prose must never reach the logs


async def test_debug_stream_no_shape_dump_when_content_streamed(
    fake_litellm, capsys
):
    """A stream that produced text keeps the one-line summary only — chunk
    shapes are dumped solely for empty streams."""
    cfg = _api_cfg(KLURIS_DEBUG_STREAM="1")
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(LiteLLMProvider(cfg).complete_stream(
        [{"role": "user", "content": "q"}], [],
    ))
    err = capsys.readouterr().err
    assert "kluris-pack: stream" in err
    assert "empty-stream" not in err


# ============================================================================
# Exception mapping (Acceptance D)
# ============================================================================


@pytest.mark.parametrize("exc_name", ["AuthenticationError", "PermissionDeniedError"])
async def test_auth_exceptions_map_to_auth_error(fake_litellm, exc_name):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc(exc_name, "bad key")
    with pytest.raises(AuthError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


async def test_context_window_exception_maps_to_context_limit(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc("ContextWindowExceededError", "too long")
    with pytest.raises(ContextLimitError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


async def test_bad_request_with_context_marker_maps_to_context_limit(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc(
        "BadRequestError", "This model's maximum context length is 8192 tokens")
    with pytest.raises(ContextLimitError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


async def test_generic_bad_request_maps_to_request_error_preserving_message(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc("BadRequestError", "invalid reasoning_effort value xyz")
    with pytest.raises(RequestError) as exc:
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))
    # The provider's real message survives (drives the agent-loop hint).
    assert "reasoning_effort" in str(exc.value)


@pytest.mark.parametrize("exc_name", ["RateLimitError", "APIConnectionError", "NotFoundError"])
async def test_other_exceptions_map_to_request_error(fake_litellm, exc_name):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc(exc_name, "nope")
    with pytest.raises(RequestError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


async def test_exception_raised_mid_stream_is_mapped(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.stream_chunks = [_c_text("partial")]
    fake_litellm.raise_in_stream = _exc("BadRequestError", "stream blew up")
    with pytest.raises(RequestError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


@pytest.mark.parametrize("exc_name,err_cls", [
    ("AuthenticationError", AuthError),
    ("BadRequestError", RequestError),
])
async def test_provider_error_message_redacts_leaked_key(fake_litellm, exc_name, err_cls):
    """LiteLLM/OpenAI error bodies echo a partial key ("Incorrect API key
    provided: sk-proj-..."), and the agent loop surfaces the message to the chat
    UI — so the key must be scrubbed at the error-mapping boundary."""
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc(
        exc_name,
        "OpenAIException - Incorrect API key provided: sk-proj-LEAKED1234secretXYZ. "
        "Find your key at https://platform.openai.com.",
    )
    with pytest.raises(err_cls) as exc:
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))
    msg = str(exc.value)
    assert "sk-proj-LEAKED1234secretXYZ" not in msg
    assert "***" in msg


async def test_provider_error_redacts_bare_jwt_bearer(fake_litellm):
    """A gateway that echoes a bare JWT bearer in an error body must not leak it
    to the chat UI (redaction is path-agnostic — exercised on the api-key path)."""
    cfg = _api_cfg()
    jwt = "eyJhbGciOiJIUzI1Ni1.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpM"
    fake_litellm.raise_on_call = _exc("BadRequestError", f"gateway rejected {jwt} now")
    with pytest.raises(RequestError) as exc:
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))
    assert jwt not in str(exc.value)
    assert "***" in str(exc.value)


# ============================================================================
# Smoke test (Acceptance D)
# ============================================================================


async def test_smoke_routes_and_forces_ping(fake_litellm):
    cfg = _api_cfg(base="https://api.openai.com")
    await LiteLLMProvider(cfg).smoke_test()
    last = fake_litellm.last
    assert last["model"] == "openai/responses/gpt-5.4-mini"
    assert last["stream"] is False
    assert last["tools"] == [_PING_TOOL_OPENAI]
    assert last["tool_choice"] == _PING_TOOL_CHOICE
    assert last["max_tokens"] == 32
    assert last["store"] is False
    # Smoke deliberately stays effort-free.
    assert "reasoning_effort" not in last
    assert "stream_options" not in last


async def test_smoke_anthropic_omits_store(fake_litellm):
    cfg = _api_cfg(shape="anthropic", base="https://api.anthropic.com", model="claude-x")
    await LiteLLMProvider(cfg).smoke_test()
    assert "store" not in fake_litellm.last
    assert fake_litellm.last["model"] == "anthropic/claude-x"


async def test_smoke_empty_choices_raises_request_error(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.nonstream_response = {"choices": []}
    with pytest.raises(RequestError) as exc:
        await LiteLLMProvider(cfg).smoke_test()
    assert "no choices" in str(exc.value)


async def test_smoke_not_found_gives_host_root_hint(fake_litellm):
    cfg = _api_cfg(base="https://openrouter.ai/api", model="bad-model")
    fake_litellm.raise_on_call = _exc("NotFoundError", "model not found")
    with pytest.raises(RequestError) as exc:
        await LiteLLMProvider(cfg).smoke_test()
    msg = str(exc.value)
    assert "host root" in msg and "KLURIS_MODEL" in msg


async def test_smoke_auth_error_maps(fake_litellm):
    cfg = _api_cfg()
    fake_litellm.raise_on_call = _exc("AuthenticationError", "401")
    with pytest.raises(AuthError):
        await LiteLLMProvider(cfg).smoke_test()


# ============================================================================
# OAuth client_credentials (Acceptance C)
# ============================================================================


@respx.mock
async def test_oauth_bearer_passed_as_api_key(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok-abc", "expires_in": 3600})
    )
    cfg = _oauth_cfg()
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]

    await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))

    assert token_route.called
    last = fake_litellm.last
    assert last["model"] == "openai/internal-model"
    assert last["api_base"] == "https://gw.test/v1"
    assert last["api_key"] == "tok-abc"
    assert "store" not in last


@respx.mock
async def test_oauth_token_is_single_flight_cached(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok-1", "expires_in": 3600})
    )
    cfg = _oauth_cfg()
    provider = LiteLLMProvider(cfg)
    await provider.smoke_test()
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(provider.complete_stream([{"role": "user", "content": "q"}], []))
    # Two LLM calls, but the cached token means exactly one token fetch.
    assert token_route.call_count == 1


@respx.mock
async def test_oauth_token_non_2xx_raises_auth_error(fake_litellm, respx_mock):
    respx_mock.post("https://idp.test/token").mock(return_value=httpx.Response(401))
    cfg = _oauth_cfg()
    with pytest.raises(AuthError):
        await _drain(LiteLLMProvider(cfg).complete_stream([{"role": "user", "content": "q"}], []))


@pytest.mark.parametrize("mock_kwargs", [
    {"side_effect": httpx.ConnectTimeout("boom")},
    {"side_effect": httpx.ConnectError("down")},
    {"return_value": httpx.Response(200, text="not json")},
    {"return_value": httpx.Response(200, json={"expires_in": 3600})},  # no access_token
    {"return_value": httpx.Response(200, json={"access_token": "t", "expires_in": "abc"})},
])
async def test_oauth_token_fetch_error_branches_raise_auth_error(fake_litellm, respx_mock, mock_kwargs):
    """Every token-fetch failure mode (timeout, connect error, non-JSON, missing
    token, bad expires_in) must surface as AuthError — coverage ported from the
    retired OAuthProvider tests."""
    respx_mock.post("https://idp.test/token").mock(**mock_kwargs)
    cfg = _oauth_cfg()
    with pytest.raises(AuthError):
        await LiteLLMProvider(cfg)._get_token()


@respx.mock
async def test_oauth_token_refreshes_after_expiry(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok", "expires_in": 3600})
    )
    cfg = _oauth_cfg()
    provider = LiteLLMProvider(cfg)
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await _drain(provider.complete_stream([{"role": "user", "content": "q"}], []))
    assert token_route.call_count == 1
    # Force the cached window to have elapsed; the next call must re-fetch.
    provider._token_expires_at = 0.0
    await _drain(provider.complete_stream([{"role": "user", "content": "q"}], []))
    assert token_route.call_count == 2


@respx.mock
async def test_oauth_expires_in_applies_refresh_leeway(fake_litellm, respx_mock):
    """The 30s leeway means a 60s token is cached for ~30s, not 60s."""
    respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok", "expires_in": 60})
    )
    provider = LiteLLMProvider(_oauth_cfg())
    import time
    before = time.monotonic()
    await provider._get_token()
    window = provider._token_expires_at - before
    assert 25 <= window <= 31  # 60 - 30 leeway


@respx.mock
async def test_oauth_token_is_single_flight_under_concurrency(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok", "expires_in": 3600})
    )
    provider = LiteLLMProvider(_oauth_cfg())
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await asyncio.gather(*(
        _drain(provider.complete_stream([{"role": "user", "content": "q"}], []))
        for _ in range(5)
    ))
    # Single-flight lock → exactly one token fetch despite 5 concurrent calls.
    assert token_route.call_count == 1


@respx.mock
async def test_oauth_two_instances_refresh_independently(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok", "expires_in": 3600})
    )
    p1 = LiteLLMProvider(_oauth_cfg(KLURIS_OAUTH_CLIENT_ID="client-a"))
    p2 = LiteLLMProvider(_oauth_cfg(KLURIS_OAUTH_CLIENT_ID="client-b"))
    fake_litellm.stream_chunks = [_c_text("hi", finish="stop")]
    await asyncio.gather(
        _drain(p1.complete_stream([{"role": "user", "content": "q"}], [])),
        _drain(p2.complete_stream([{"role": "user", "content": "q"}], [])),
    )
    # Per-instance cache/lock → each provider fetches its own token.
    assert token_route.call_count == 2


@respx.mock
async def test_oauth_scope_reaches_token_body(fake_litellm, respx_mock):
    token_route = respx_mock.post("https://idp.test/token").mock(
        return_value=httpx.Response(200, json={"access_token": "tok", "expires_in": 3600})
    )
    provider = LiteLLMProvider(_oauth_cfg(KLURIS_OAUTH_SCOPE="read:brain"))
    await provider._get_token()
    body = token_route.calls.last.request.content.decode()
    assert "scope=read" in body and "brain" in body
    assert "grant_type=client_credentials" in body


# ============================================================================
# configure_litellm — TLS + globals (Acceptance D)
# ============================================================================


async def test_configure_litellm_default_verify_true_and_drop_params(monkeypatch):
    captured = {}

    class _CaptureClient:
        def __init__(self, *a, verify=None, **k):
            captured["verify"] = verify

    monkeypatch.setattr(
        "kluris.pack.providers.litellm_provider.httpx.AsyncClient", _CaptureClient)
    litellm.drop_params = False  # prove configure_litellm sets it

    litellm.suppress_debug_info = False
    configure_litellm(_api_cfg())

    assert captured["verify"] is True
    assert litellm.drop_params is True
    assert litellm.suppress_debug_info is True
    assert isinstance(litellm.aclient_session, _CaptureClient)
    # The Anthropic / generic handler reads litellm.ssl_verify, NOT aclient_session.
    assert litellm.ssl_verify is True


async def test_configure_litellm_insecure_verify_false(monkeypatch):
    captured = {}

    class _CaptureClient:
        def __init__(self, *a, verify=None, **k):
            captured["verify"] = verify

    monkeypatch.setattr(
        "kluris.pack.providers.litellm_provider.httpx.AsyncClient", _CaptureClient)
    configure_litellm(_api_cfg(KLURIS_TLS_INSECURE="1"))
    assert captured["verify"] is False
    assert litellm.ssl_verify is False


_PRIVATE_CA = Path(__file__).parent / "fixtures" / "tls" / "private-ca.pem"


async def test_configure_litellm_ca_bundle_sslcontext_and_path(monkeypatch):
    """A CA bundle reaches aclient_session as an SSLContext (httpx 0.28+ form)
    and litellm.ssl_verify as the bundle PATH (the only non-bool form LiteLLM's
    generic handler honors from the global)."""
    captured = {}

    class _CaptureClient:
        def __init__(self, *a, verify=None, **k):
            captured["verify"] = verify

    monkeypatch.setattr(
        "kluris.pack.providers.litellm_provider.httpx.AsyncClient", _CaptureClient)
    configure_litellm(_api_cfg(KLURIS_CA_BUNDLE=str(_PRIVATE_CA)))
    assert isinstance(captured["verify"], ssl.SSLContext)
    assert litellm.ssl_verify == str(_PRIVATE_CA)


async def test_configure_litellm_ca_bundle_trusted_by_generic_handler(monkeypatch):
    """Regression: the Anthropic and Responses routes build their client from
    LiteLLM's get_ssl_configuration(), which ignored an SSLContext set on
    litellm.ssl_verify and fell back to certifi, so KLURIS_CA_BUNDLE never
    reached them and a private-CA endpoint failed verification. The context
    those routes get must trust the deployer's CA."""
    from litellm.llms.custom_httpx.http_handler import get_ssl_configuration

    # Either env var would override litellm.ssl_verify / the bundle.
    monkeypatch.delenv("SSL_VERIFY", raising=False)
    monkeypatch.delenv("SSL_CERT_FILE", raising=False)

    configure_litellm(_api_cfg(
        shape="anthropic", base="https://llm.internal", model="claude-sonnet-4-5",
        KLURIS_CA_BUNDLE=str(_PRIVATE_CA),
    ))
    ctx = get_ssl_configuration()
    assert isinstance(ctx, ssl.SSLContext)
    subjects = [str(cert.get("subject")) for cert in ctx.get_ca_certs()]
    assert any("kluris-test-private-ca" in s for s in subjects)


async def test_configure_litellm_ssl_verify_is_consumed_by_resolver(monkeypatch):
    """Pin the CONSUMPTION link, not just that the global is set: LiteLLM's
    own get_ssl_verify() (called by the Anthropic/generic handler) must return
    the deployer's TLS choice. This is the link that hid the original bug —
    setting aclient_session looked right but the Anthropic path never read it."""
    from litellm.llms.custom_httpx.http_handler import get_ssl_verify

    # SSL_VERIFY env would override litellm.ssl_verify — assert the image
    # contract (no such override) holds in the test env too.
    monkeypatch.delenv("SSL_VERIFY", raising=False)

    configure_litellm(_api_cfg())
    assert get_ssl_verify() is True

    configure_litellm(_api_cfg(KLURIS_TLS_INSECURE="1"))
    assert get_ssl_verify() is False

    configure_litellm(_api_cfg(KLURIS_CA_BUNDLE=str(_PRIVATE_CA)))
    assert get_ssl_verify() == str(_PRIVATE_CA)


# ============================================================================
# Wire-level request bodies: real LiteLLM against a local mock endpoint
# ============================================================================


def _sse(events: list[dict[str, Any]]) -> str:
    return "".join(
        f"event: {e.get('type', '')}\ndata: {json.dumps(e)}\n\n" for e in events
    )


_RESP_MSG = {
    "type": "message", "id": "m1", "status": "completed", "role": "assistant",
    "content": [{"type": "output_text", "text": "pong", "annotations": []}],
}
_RESP_USAGE = {
    "input_tokens": 1, "output_tokens": 1, "total_tokens": 2,
    "input_tokens_details": {"cached_tokens": 0},
    "output_tokens_details": {"reasoning_tokens": 0},
}


def _resp_obj(status: str, output: list, usage: dict | None) -> dict[str, Any]:
    return {
        "id": "resp1", "object": "response", "created_at": 1, "status": status,
        "model": "m", "output": output, "parallel_tool_calls": True,
        "tool_choice": "auto", "tools": [], "usage": usage, "error": None,
        "incomplete_details": None, "instructions": None, "metadata": {},
        "temperature": None, "top_p": None, "text": {"format": {"type": "text"}},
    }


def _mock_reply(path: str, stream: bool) -> tuple[str, str]:
    """(content-type, body) for one mock LLM request: a one-word "pong"."""
    if path.endswith("/token"):
        return "application/json", json.dumps(
            {"access_token": "tok-wire", "expires_in": 3600})
    if path.endswith("/messages"):
        if not stream:
            return "application/json", json.dumps({
                "id": "msg0", "type": "message", "role": "assistant", "model": "m",
                "content": [{"type": "text", "text": "pong"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1},
            })
        return "text/event-stream", _sse([
            {"type": "message_start", "message": {
                "id": "msg1", "type": "message", "role": "assistant", "model": "m",
                "content": [], "stop_reason": None,
                "usage": {"input_tokens": 1, "output_tokens": 1}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "pong"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "end_turn"},
             "usage": {"output_tokens": 1}},
            {"type": "message_stop"},
        ])
    if path.endswith("/responses"):
        if not stream:
            return "application/json", json.dumps(
                _resp_obj("completed", [_RESP_MSG], _RESP_USAGE))
        return "text/event-stream", _sse([
            {"type": "response.created", "sequence_number": 0,
             "response": _resp_obj("in_progress", [], None)},
            {"type": "response.output_item.added", "sequence_number": 1,
             "output_index": 0,
             "item": {**_RESP_MSG, "status": "in_progress", "content": []}},
            {"type": "response.content_part.added", "sequence_number": 2,
             "output_index": 0, "item_id": "m1", "content_index": 0,
             "part": {"type": "output_text", "text": "", "annotations": []}},
            {"type": "response.output_text.delta", "sequence_number": 3,
             "output_index": 0, "item_id": "m1", "content_index": 0,
             "delta": "pong", "logprobs": []},
            {"type": "response.output_text.done", "sequence_number": 4,
             "output_index": 0, "item_id": "m1", "content_index": 0,
             "text": "pong", "logprobs": []},
            {"type": "response.output_item.done", "sequence_number": 5,
             "output_index": 0, "item": _RESP_MSG},
            {"type": "response.completed", "sequence_number": 6,
             "response": _resp_obj("completed", [_RESP_MSG], _RESP_USAGE)},
        ])
    # /chat/completions
    if not stream:
        return "application/json", json.dumps({
            "id": "c0", "object": "chat.completion", "created": 1, "model": "m",
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": "pong"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        })
    chunks = [
        {"id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "m",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "pong"},
                      "finish_reason": "stop"}]},
        {"id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "m",
         "choices": [],
         "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}},
    ]
    return "text/event-stream", (
        "".join(f"data: {json.dumps(c)}\n\n" for c in chunks) + "data: [DONE]\n\n"
    )


@pytest.fixture
def mock_llm_server():
    """A local plain-HTTP LLM endpoint that records every request body."""
    bodies: list[tuple[str, dict[str, Any]]] = []

    class _Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):  # keep test output quiet
            pass

        def do_POST(self):
            raw = self.rfile.read(int(self.headers.get("content-length") or 0))
            body = {}
            if not self.path.endswith("/token"):
                body = json.loads(raw or b"{}")
                bodies.append((self.path, body))
            ctype, text = _mock_reply(self.path, bool(body.get("stream")))
            data = text.encode("utf-8")
            self.send_response(200)
            self.send_header("content-type", ctype)
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}", bodies
    finally:
        server.shutdown()
        server.server_close()


@pytest.mark.parametrize(
    "route, path, sends_store",
    [
        ("anthropic", "/v1/messages", False),
        ("responses", "/v1/responses", True),
        ("gateway", "/v1/chat/completions", False),
        ("oauth", "/v1/chat/completions", False),
    ],
)
async def test_wire_body_store_per_route(
    mock_llm_server, monkeypatch, route, path, sends_store
):
    """Pin what LiteLLM actually puts on the wire, per route: ``store: false``
    reaches OpenAI proper (the Responses route) and nothing else. LiteLLM
    1.87.0 silently dropped ``store`` on the gateway routes and later versions
    forward it, so a fake ``acompletion`` alone cannot catch a strict gateway
    receiving a field it never got before."""
    base, bodies = mock_llm_server
    for name in ("drop_params", "suppress_debug_info", "ssl_verify", "aclient_session"):
        monkeypatch.setattr(litellm, name, getattr(litellm, name))
    monkeypatch.delenv("OPENAI_API_BASE", raising=False)
    if route == "anthropic":
        cfg = _api_cfg(shape="anthropic", base=base, model="claude-sonnet-4-5")
    elif route == "responses":
        # OpenAI proper: api.openai.com → api_base None → LiteLLM's default
        # OpenAI base, redirected to the mock through its own env override.
        monkeypatch.setenv("OPENAI_BASE_URL", base + "/v1")
        cfg = _api_cfg(base="https://api.openai.com", model="gpt-5")
    elif route == "gateway":
        cfg = _api_cfg(base=base, model="gw-model")
    else:
        cfg = _oauth_cfg(KLURIS_OAUTH_TOKEN_URL=base + "/token",
                         KLURIS_OAUTH_API_BASE_URL=base)
    configure_litellm(cfg)
    provider = LiteLLMProvider(cfg)

    await provider.smoke_test()
    events = await _drain(provider.complete_stream(
        [{"role": "user", "content": "q"}],
        [{"type": "function", "function": {
            "name": "search", "description": "s",
            "parameters": {"type": "object", "properties": {}}}}],
    ))

    assert {"kind": "token", "text": "pong"} in events
    assert [p for p, _ in bodies] == [path, path]
    for _, body in bodies:
        if sends_store:
            assert body["store"] is False
        else:
            assert "store" not in body
