"""TEST-PACK-14 — APIKeyProvider for Anthropic + OpenAI shapes."""

from __future__ import annotations

import json
from typing import Iterator

import httpx
import pytest
import respx

from kluris.pack.config import Config
from kluris.pack.providers.apikey import APIKeyProvider
from kluris.pack.providers.base import AuthError, ContextLimitError, RequestError


pytestmark = pytest.mark.asyncio


# --- Helpers -----------------------------------------------------------------


def _build_config(env: dict, *, shape: str = "anthropic", base: str = "http://api.test") -> Config:
    e = dict(
        env,
        KLURIS_PROVIDER_SHAPE=shape,
        KLURIS_BASE_URL=base,
        KLURIS_BRAIN_DIR="/app/brain",
    )
    return Config.load_from_env(e)


def _anthropic_smoke_response() -> dict:
    return {
        "id": "msg_1",
        "content": [
            {
                "type": "tool_use",
                "id": "tu_1",
                "name": "ping",
                "input": {"value": "pong"},
            }
        ],
        "usage": {"input_tokens": 1, "output_tokens": 1},
    }


def _openai_smoke_response() -> dict:
    return {
        "id": "chatcmpl-1",
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "ping",
                                "arguments": json.dumps({"value": "pong"}),
                            },
                        }
                    ],
                }
            }
        ],
    }


def _sse_lines(events: Iterator[str]) -> bytes:
    return ("\n".join(events) + "\n").encode("utf-8")


# --- API-key env fixture (overrides conftest's so brain_dir doesn't matter) --


@pytest.fixture
def api_env() -> dict:
    return {
        "KLURIS_API_KEY": "sk-test-secret",
        "KLURIS_MODEL": "fake-model",
    }


# --- Smoke tests -------------------------------------------------------------


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_anthropic_smoke_sets_headers(api_env, respx_mock):
    cfg = _build_config(api_env, shape="anthropic")
    route = respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(200, json=_anthropic_smoke_response())
    )
    await APIKeyProvider(cfg).smoke_test()
    assert route.called
    req = route.calls.last.request
    assert req.headers.get("x-api-key") == "sk-test-secret"
    assert req.headers.get("anthropic-version") == "2023-06-01"


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_anthropic_version_override_honored(api_env, respx_mock):
    env = dict(api_env, KLURIS_ANTHROPIC_VERSION="2099-12-31")
    cfg = _build_config(env, shape="anthropic")
    route = respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(200, json=_anthropic_smoke_response())
    )
    await APIKeyProvider(cfg).smoke_test()
    assert route.calls.last.request.headers.get("anthropic-version") == "2099-12-31"


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_openai_smoke_sets_bearer(api_env, respx_mock):
    cfg = _build_config(api_env, shape="openai")
    route = respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_openai_smoke_response())
    )
    await APIKeyProvider(cfg).smoke_test()
    assert route.calls.last.request.headers.get("Authorization") == "Bearer sk-test-secret"


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_raises_on_401(api_env, respx_mock):
    cfg = _build_config(api_env, shape="anthropic")
    respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(401, json={"error": "bad key"})
    )
    with pytest.raises(AuthError):
        await APIKeyProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_raises_on_500(api_env, respx_mock):
    cfg = _build_config(api_env, shape="openai")
    respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(500, json={"error": "boom"})
    )
    with pytest.raises(RequestError):
        await APIKeyProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_raises_when_tool_call_missing(api_env, respx_mock):
    """Endpoint returns 200 but ignores the ping tool — fail-fast."""
    cfg = _build_config(api_env, shape="anthropic")
    respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(
            200,
            json={"id": "msg_x", "content": [{"type": "text", "text": "hi"}]},
        )
    )
    with pytest.raises(RequestError):
        await APIKeyProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_raises_on_timeout(api_env, respx_mock):
    cfg = _build_config(api_env, shape="anthropic")
    respx_mock.post("http://api.test/v1/messages").mock(
        side_effect=httpx.ConnectTimeout("simulated")
    )
    with pytest.raises(RequestError):
        await APIKeyProvider(cfg).smoke_test()


# --- Streaming tests ---------------------------------------------------------


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_anthropic_stream_yields_token_events(api_env, respx_mock):
    cfg = _build_config(api_env, shape="anthropic")
    body = _sse_lines(iter([
        "event: message_start",
        "data: " + json.dumps({"type": "message_start", "message": {"id": "m1"}}),
        "",
        "event: content_block_start",
        "data: " + json.dumps({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""},
        }),
        "",
        "event: content_block_delta",
        "data: " + json.dumps({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "hello"},
        }),
        "",
        "event: content_block_delta",
        "data: " + json.dumps({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": " world"},
        }),
        "",
        "event: message_delta",
        "data: " + json.dumps({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {"input_tokens": 7, "output_tokens": 5},
        }),
        "",
        "event: message_stop",
        "data: " + json.dumps({"type": "message_stop"}),
        "",
    ]))
    respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(200, content=body, headers={
            "content-type": "text/event-stream",
        })
    )
    events = []
    async for ev in APIKeyProvider(cfg).complete_stream(
        messages=[{"role": "user", "content": "hi"}],
        tools=[],
    ):
        events.append(ev)

    tokens = [e for e in events if e["kind"] == "token"]
    usage = [e for e in events if e["kind"] == "usage"]
    end = [e for e in events if e["kind"] == "end"]
    assert "".join(t["text"] for t in tokens) == "hello world"
    assert usage == [{"kind": "usage", "input": 7, "output": 5}]
    assert len(end) == 1


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_anthropic_stream_emits_tool_use(api_env, respx_mock):
    cfg = _build_config(api_env, shape="anthropic")
    body = _sse_lines(iter([
        "event: content_block_start",
        "data: " + json.dumps({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "tool_use", "id": "tu1", "name": "search"},
        }),
        "",
        "event: content_block_delta",
        "data: " + json.dumps({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": '{"query":'},
        }),
        "",
        "event: content_block_delta",
        "data: " + json.dumps({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": '"auth"}'},
        }),
        "",
        "event: content_block_stop",
        "data: " + json.dumps({"type": "content_block_stop", "index": 0}),
        "",
        "event: message_delta",
        "data: " + json.dumps({
            "type": "message_delta",
            "usage": {"input_tokens": 3, "output_tokens": 4},
        }),
        "",
        "event: message_stop",
        "data: " + json.dumps({"type": "message_stop"}),
        "",
    ]))
    respx_mock.post("http://api.test/v1/messages").mock(
        return_value=httpx.Response(200, content=body, headers={
            "content-type": "text/event-stream",
        })
    )
    events = [
        e
        async for e in APIKeyProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "X"}], tools=[]
        )
    ]
    tool_uses = [e for e in events if e["kind"] == "tool_use"]
    assert tool_uses == [{
        "kind": "tool_use",
        "name": "search",
        "id": "tu1",
        "args": {"query": "auth"},
    }]


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_openai_stream_yields_token_and_usage(api_env, respx_mock):
    cfg = _build_config(api_env, shape="openai")
    body = _sse_lines(iter([
        "data: " + json.dumps({
            "choices": [{"delta": {"content": "hi "}}]
        }),
        "",
        "data: " + json.dumps({
            "choices": [{"delta": {"content": "there"}}]
        }),
        "",
        "data: " + json.dumps({
            "choices": [],
            "usage": {"prompt_tokens": 9, "completion_tokens": 3},
        }),
        "",
        "data: [DONE]",
        "",
    ]))
    respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, content=body)
    )
    events = [
        e
        async for e in APIKeyProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "x"}], tools=[]
        )
    ]
    tokens = [e for e in events if e["kind"] == "token"]
    usage = [e for e in events if e["kind"] == "usage"]
    assert "".join(t["text"] for t in tokens) == "hi there"
    assert usage == [{"kind": "usage", "input": 9, "output": 3}]


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_openai_stream_graceful_zero_usage_on_missing(api_env, respx_mock):
    """Some proxies never emit a usage chunk — provider must emit zero
    usage rather than crash.
    """
    cfg = _build_config(api_env, shape="openai")
    body = _sse_lines(iter([
        "data: " + json.dumps({"choices": [{"delta": {"content": "x"}}]}),
        "",
        "data: [DONE]",
        "",
    ]))
    respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(200, content=body)
    )
    events = [
        e
        async for e in APIKeyProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "x"}], tools=[]
        )
    ]
    usage = [e for e in events if e["kind"] == "usage"]
    assert usage == [{"kind": "usage", "input": 0, "output": 0}]


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_raises_context_limit_on_413(api_env, respx_mock):
    cfg = _build_config(api_env, shape="openai")
    respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(413, json={"error": "too big"})
    )
    with pytest.raises(ContextLimitError):
        async for _ in APIKeyProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_raises_context_limit_on_marker_400(api_env, respx_mock):
    cfg = _build_config(api_env, shape="openai")
    respx_mock.post("http://api.test/v1/chat/completions").mock(
        return_value=httpx.Response(
            400,
            json={"error": "context_length_exceeded: maximum 8192 tokens"},
        )
    )
    with pytest.raises(ContextLimitError):
        async for _ in APIKeyProvider(cfg).complete_stream(messages=[], tools=[]):
            pass
