"""TEST-PACK-14 — LiteLLMProvider: routing, streaming, errors, TLS, OAuth.

The single :class:`LiteLLMProvider` replaces the retired hand-rolled
``APIKeyProvider`` + ``OAuthProvider``. These tests monkeypatch
``litellm.acompletion`` to capture the outgoing kwargs and to simulate streamed
chunks + raised litellm exceptions — everything past that boundary (real OpenAI
behavior) is NOT exercised here and must be checked by hand against a live key.
"""

from __future__ import annotations

import asyncio
import ssl
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
    assert last["store"] is False
    assert last["stream_options"] == {"include_usage": True}


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
    (no text, no tool, finish='stop') is exactly what the line surfaces. Only
    counts/booleans/finish_reason are logged — no payloads."""
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
    assert last["store"] is False


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


async def test_configure_litellm_ca_bundle_passes_sslcontext(monkeypatch):
    bundle = Path(ssl.get_default_verify_paths().cafile or "")
    if not bundle or not bundle.exists():
        pytest.skip("no system CA bundle available to use as a fixture")
    captured = {}

    class _CaptureClient:
        def __init__(self, *a, verify=None, **k):
            captured["verify"] = verify

    monkeypatch.setattr(
        "kluris.pack.providers.litellm_provider.httpx.AsyncClient", _CaptureClient)
    configure_litellm(_api_cfg(KLURIS_CA_BUNDLE=str(bundle)))
    assert isinstance(captured["verify"], ssl.SSLContext)
    assert isinstance(litellm.ssl_verify, ssl.SSLContext)


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

    bundle = Path(ssl.get_default_verify_paths().cafile or "")
    if bundle and bundle.exists():
        configure_litellm(_api_cfg(KLURIS_CA_BUNDLE=str(bundle)))
        assert isinstance(get_ssl_verify(), ssl.SSLContext)
