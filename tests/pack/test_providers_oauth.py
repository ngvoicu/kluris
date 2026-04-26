"""TEST-PACK-16 — OAuthProvider client_credentials + caching."""

from __future__ import annotations

import asyncio
import json

import httpx
import pytest
import respx

from kluris.pack.config import Config
from kluris.pack.providers.base import AuthError
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
