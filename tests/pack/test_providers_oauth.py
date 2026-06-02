"""TEST-PACK-16 — OAuthProvider client_credentials + caching."""

from __future__ import annotations

import asyncio
import json

import httpx
import pytest
import respx

from kluris.pack.config import Config
from kluris.pack.providers.base import AuthError, ContextLimitError, RequestError
from kluris.pack.providers.oauth import OAuthProvider

pytestmark = pytest.mark.asyncio


_TOKEN_URL = "http://idp.test/token"
_API_URL = "http://api.test"


def _build_config(*, client_id: str = "client-a", scope: str | None = None) -> Config:
    env = {
        "KLURIS_OAUTH_TOKEN_URL": _TOKEN_URL,
        "KLURIS_OAUTH_API_BASE_URL": _API_URL,
        "KLURIS_OAUTH_CLIENT_ID": client_id,
        "KLURIS_OAUTH_CLIENT_SECRET": f"secret-{client_id}",
        "KLURIS_MODEL": "test-model",
        "KLURIS_BRAIN_DIR": "/app/brain",
    }
    if scope is not None:
        env["KLURIS_OAUTH_SCOPE"] = scope
    return Config.load_from_env(env)


def _smoke_response() -> dict:
    return {
        "id": "cmpl-1",
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "c1",
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


def _sse_lines(events: list[str]) -> bytes:
    return ("\n".join(events) + "\n").encode("utf-8")


# --- Token fetch ----------------------------------------------------------


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_form_encoded(respx_mock):
    cfg = _build_config()
    route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200,
            json={"access_token": "tok-1", "expires_in": 3600},
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    await OAuthProvider(cfg).smoke_test()
    assert route.called
    sent = route.calls.last.request
    body = sent.read().decode("utf-8")
    assert "grant_type=client_credentials" in body
    assert "client_id=client-a" in body
    assert "client_secret=secret-client-a" in body


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_includes_scope(respx_mock):
    cfg = _build_config(scope="read:brain")
    route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200,
            json={"access_token": "tok-1", "expires_in": 3600},
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    await OAuthProvider(cfg).smoke_test()
    body = route.calls.last.request.read().decode("utf-8")
    assert "scope=read%3Abrain" in body


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_cached_within_ttl(respx_mock):
    cfg = _build_config()
    token_route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok-1", "expires_in": 3600}
        )
    )
    api_route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    provider = OAuthProvider(cfg)
    await provider.smoke_test()
    await provider.smoke_test()
    # Two API calls, but only one token fetch (cached).
    assert token_route.call_count == 1
    assert api_route.call_count == 2


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_refresh_after_expiry(respx_mock):
    cfg = _build_config()
    token_route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok-1", "expires_in": 60}
        )
    )
    api_route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    provider = OAuthProvider(cfg)
    await provider.smoke_test()
    # Force expiry past the 30-s leeway.
    provider._token_expires_at = 0.0
    await provider.smoke_test()
    assert token_route.call_count == 2
    assert api_route.call_count == 2


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_failure_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(401, json={"error": "bad client"})
    )
    with pytest.raises(AuthError):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_timeout_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(side_effect=httpx.ConnectTimeout("simulated"))
    with pytest.raises(AuthError):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_http_error_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(side_effect=httpx.ConnectError("simulated"))
    with pytest.raises(AuthError, match="http error"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_non_json_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(return_value=httpx.Response(200, text="oops"))
    with pytest.raises(AuthError, match="not JSON"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_missing_access_token_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(200, json={"expires_in": 3600})
    )
    with pytest.raises(AuthError, match="missing access_token"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_token_fetch_invalid_expires_in_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": {"bad": "shape"}}
        )
    )
    with pytest.raises(AuthError, match="invalid expires_in"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_two_providers_refresh_independently(respx_mock):
    """Different ``client_id`` values must hit the token endpoint
    concurrently — no module-level lock that serializes refreshes
    across instances.
    """
    cfg_a = _build_config(client_id="client-a")
    cfg_b = _build_config(client_id="client-b")

    token_route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )

    pa = OAuthProvider(cfg_a)
    pb = OAuthProvider(cfg_b)
    await asyncio.gather(pa.smoke_test(), pb.smoke_test())

    # Both providers must have hit the token URL — no cross-instance
    # serialization.
    assert token_route.call_count == 2
    bodies = [c.request.read().decode("utf-8") for c in token_route.calls]
    assert any("client_id=client-a" in body for body in bodies)
    assert any("client_id=client-b" in body for body in bodies)


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_concurrent_refresh_in_one_provider_single_flights(respx_mock):
    """Multiple concurrent calls in ONE provider must issue exactly
    one token fetch (per-instance lock).
    """
    cfg = _build_config()
    token_route = respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    provider = OAuthProvider(cfg)
    await asyncio.gather(*(provider.smoke_test() for _ in range(5)))
    assert token_route.call_count == 1


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_uses_bearer_header(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok-bearer", "expires_in": 3600}
        )
    )
    api_route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    await OAuthProvider(cfg).smoke_test()
    assert api_route.calls.last.request.headers.get("Authorization") == "Bearer tok-bearer"


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_passes_on_empty_tool_calls(respx_mock):
    """Some real-world gateways silently ignore ``tool_choice`` and
    return an empty ``tool_calls`` array. The boot smoke-test no longer
    rejects this — the structural shape (non-empty ``choices[]``) is
    what we verify; the chat itself fails clearly later if tool-calling
    is genuinely broken on this endpoint.
    """
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(
            200,
            json={"choices": [{"message": {"tool_calls": []}}]},
        )
    )
    # No exception expected.
    await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_passes_on_wrong_tool_call_name(respx_mock):
    """Same rationale: the boot probe doesn't enforce ``ping`` as the
    forced tool name. As long as the response is structurally a chat
    completion (non-empty ``choices[]``), boot succeeds.
    """
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    wrong = _smoke_response()
    wrong["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = "search"
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=wrong)
    )
    await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_rejects_non_completion_response(respx_mock):
    """A 200 that's not a chat-completion shape (HTML proxy error,
    misrouted JSON) still fails fast.
    """
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(
            200,
            json={"unexpected": "envelope", "choices": []},  # empty choices[]
        )
    )
    with pytest.raises(RequestError):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_auth_status_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(401, json={"error": "bad token"})
    )
    with pytest.raises(AuthError, match="auth failed"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_non_2xx_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(500, json={"error": "upstream"})
    )
    with pytest.raises(RequestError, match="non-2xx"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_non_json_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, text="<html>proxy</html>")
    )
    with pytest.raises(RequestError, match="not JSON"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_timeout_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        side_effect=httpx.ReadTimeout("simulated")
    )
    with pytest.raises(RequestError, match="timeout"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_http_error_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        side_effect=httpx.ConnectError("simulated")
    )
    with pytest.raises(RequestError, match="http error"):
        await OAuthProvider(cfg).smoke_test()


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_yields_openai_events_and_sends_bearer(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok-stream", "expires_in": 3600}
        )
    )
    route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(
            200,
            content=_sse_lines([
                "data: " + json.dumps({"choices": [{"delta": {"content": "hi"}}]}),
                "",
                "data: " + json.dumps({
                    "choices": [],
                    "usage": {"prompt_tokens": 2, "completion_tokens": 1},
                }),
                "",
                "data: [DONE]",
                "",
            ]),
        )
    )

    events = [
        e async for e in OAuthProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "hello"}],
            tools=[],
        )
    ]

    assert route.calls.last.request.headers.get("Authorization") == "Bearer tok-stream"
    sent = json.loads(route.calls.last.request.content)
    assert sent["messages"] == [{"role": "user", "content": "hello"}]
    assert [e for e in events if e["kind"] == "token"] == [
        {"kind": "token", "text": "hi"}
    ]
    assert [e for e in events if e["kind"] == "usage"] == [
        {"kind": "usage", "input": 2, "output": 1}
    ]


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_auth_status_raises_auth_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(403, json={"error": "forbidden"})
    )
    with pytest.raises(AuthError, match="streaming auth failed"):
        async for _ in OAuthProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_raises_context_limit_on_413(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(413, json={"error": "too big"})
    )
    with pytest.raises(ContextLimitError):
        async for _ in OAuthProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_raises_context_limit_on_marker_400(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(
            400,
            json={"error": "context_length_exceeded: maximum 8192 tokens"},
        )
    )
    with pytest.raises(ContextLimitError):
        async for _ in OAuthProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_non_2xx_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(500, text="upstream broke")
    )
    with pytest.raises(RequestError, match="streaming non-2xx"):
        async for _ in OAuthProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_http_error_raises_request_error(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        side_effect=httpx.ConnectError("simulated")
    )
    with pytest.raises(RequestError, match="streaming http error"):
        async for _ in OAuthProvider(cfg).complete_stream(messages=[], tools=[]):
            pass


# --- Completion-budget param -------------------------------------------------
#
# The OAuth provider drives ``/v1/chat/completions`` (OpenAI shape), so it
# must send ``max_completion_tokens`` — newer GPT reasoning models reject
# ``max_tokens`` with HTTP 400. The ``not in`` half is load-bearing: a body
# carrying both keys would still 400.


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_body_uses_max_completion_tokens(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    await OAuthProvider(cfg).smoke_test()
    body = json.loads(route.calls.last.request.content)
    assert body["max_completion_tokens"] == 32
    assert "max_tokens" not in body


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_body_uses_max_completion_tokens(respx_mock):
    cfg = _build_config()
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, content=_sse_lines(["data: [DONE]", ""]))
    )
    _events = [
        e async for e in OAuthProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "x"}], tools=[]
        )
    ]
    body = json.loads(route.calls.last.request.content)
    assert body["max_completion_tokens"] == 4096
    assert "max_tokens" not in body


def _oauth_env(**extra) -> dict:
    return {
        "KLURIS_OAUTH_TOKEN_URL": _TOKEN_URL,
        "KLURIS_OAUTH_API_BASE_URL": _API_URL,
        "KLURIS_OAUTH_CLIENT_ID": "client-a",
        "KLURIS_OAUTH_CLIENT_SECRET": "secret-client-a",
        "KLURIS_MODEL": "test-model",
        "KLURIS_BRAIN_DIR": "/app/brain",
        **extra,
    }


async def _drain_oauth_stream(cfg, respx_mock):
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, content=_sse_lines(["data: [DONE]", ""]))
    )
    _events = [
        e async for e in OAuthProvider(cfg).complete_stream(
            messages=[{"role": "user", "content": "x"}], tools=[]
        )
    ]
    return json.loads(route.calls.last.request.content)


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_honors_configured_output_tokens(respx_mock):
    cfg = Config.load_from_env(_oauth_env(KLURIS_MAX_OUTPUT_TOKENS="7777"))
    body = await _drain_oauth_stream(cfg, respx_mock)
    assert body["max_completion_tokens"] == 7777


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_omits_temperature_by_default(respx_mock):
    cfg = Config.load_from_env(_oauth_env())
    body = await _drain_oauth_stream(cfg, respx_mock)
    assert "temperature" not in body


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_includes_temperature_when_set(respx_mock):
    cfg = Config.load_from_env(_oauth_env(KLURIS_TEMPERATURE="0.5"))
    body = await _drain_oauth_stream(cfg, respx_mock)
    assert body["temperature"] == 0.5


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_stream_sends_temperature_zero_not_dropped_as_falsy(respx_mock):
    """OAuth builds its body with its OWN inline `is not None` guard (it does
    not reuse apikey._stream_body), so temperature=0.0 must be SENT here too."""
    cfg = Config.load_from_env(_oauth_env(KLURIS_TEMPERATURE="0"))
    body = await _drain_oauth_stream(cfg, respx_mock)
    assert body["temperature"] == 0.0


@respx.mock(assert_all_mocked=True, assert_all_called=False)
async def test_smoke_body_omits_temperature_and_keeps_boot_budget(respx_mock):
    """The boot smoke probe is a fixed tiny ping — it must NOT inherit the
    deployer's temperature (would break reasoning models at boot) nor the
    KLURIS_MAX_OUTPUT_TOKENS budget; it stays at max_completion_tokens=32."""
    cfg = Config.load_from_env(
        _oauth_env(KLURIS_TEMPERATURE="0.7", KLURIS_MAX_OUTPUT_TOKENS="9000")
    )
    respx_mock.post(_TOKEN_URL).mock(
        return_value=httpx.Response(
            200, json={"access_token": "tok", "expires_in": 3600}
        )
    )
    route = respx_mock.post(_API_URL + "/v1/chat/completions").mock(
        return_value=httpx.Response(200, json=_smoke_response())
    )
    await OAuthProvider(cfg).smoke_test()
    body = json.loads(route.calls.last.request.content)
    assert "temperature" not in body
    assert body["max_completion_tokens"] == 32
