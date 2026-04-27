"""OAuth 2.0 ``client_credentials`` provider.

Fetches a bearer token from ``KLURIS_OAUTH_TOKEN_URL`` once, caches it
in memory until 30 s before ``expires_in`` elapses, then drives the
configured ``KLURIS_OAUTH_API_BASE_URL`` (which speaks the OpenAI Chat
Completions shape) for chat. Tokens never persist to disk.

Concurrent refreshes inside a single :class:`OAuthProvider` are
single-flighted via a per-instance :class:`asyncio.Lock` so a burst of
parallel requests issues exactly one token fetch. Two ``OAuthProvider``
instances (different ``client_id``) refresh independently.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any, AsyncIterator

import httpx

from ..config import Config
from .apikey import _messages_for_openai, _parse_openai_stream, _smoke_response_looks_valid
from .base import (
    AuthError,
    ContextLimitError,
    LLMProvider,
    RequestError,
)

_TOKEN_TIMEOUT = httpx.Timeout(connect=5.0, read=15.0, write=5.0, pool=5.0)
_SMOKE_TIMEOUT = httpx.Timeout(connect=5.0, read=15.0, write=5.0, pool=5.0)
_STREAM_TIMEOUT = httpx.Timeout(connect=5.0, read=60.0, write=5.0, pool=5.0)
_TOKEN_REFRESH_LEEWAY_S = 30.0


_PING_TOOL_OPENAI = {
    "type": "function",
    "function": {
        "name": "ping",
        "description": "Echo a single token. Used by Kluris boot smoke-test.",
        "parameters": {
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
        },
    },
}


class OAuthProvider(LLMProvider):
    """OAuth client_credentials → OpenAI-shape API."""

    def __init__(self, config: Config) -> None:
        self._cfg = config
        self.token_url = config.oauth_token_url or ""
        self.api_base_url = config.oauth_api_base_url or ""
        self.client_id = config.oauth_client_id or ""
        self._client_secret = (
            config.oauth_client_secret.get_secret_value()
            if config.oauth_client_secret
            else ""
        )
        self.scope = config.oauth_scope
        self.model = config.model

        # Per-INSTANCE lock; do not move to module level — two providers
        # with different client_id must refresh independently.
        self._refresh_lock = asyncio.Lock()
        self._cached_token: str | None = None
        self._token_expires_at: float = 0.0

    # --- Token lifecycle ---------------------------------------------

    async def _get_token(self) -> str:
        now = time.monotonic()
        if self._cached_token and now < self._token_expires_at:
            return self._cached_token

        async with self._refresh_lock:
            # Recheck inside the lock — another waiter may have just
            # refreshed.
            now = time.monotonic()
            if self._cached_token and now < self._token_expires_at:
                return self._cached_token
            await self._refresh_token()
            assert self._cached_token is not None
            return self._cached_token

    async def _refresh_token(self) -> None:
        form = {
            "grant_type": "client_credentials",
            "client_id": self.client_id,
            "client_secret": self._client_secret,
        }
        if self.scope:
            form["scope"] = self.scope
        try:
            async with httpx.AsyncClient(
                timeout=_TOKEN_TIMEOUT, verify=self._cfg.httpx_verify,
            ) as client:
                resp = await client.post(self.token_url, data=form)
        except httpx.TimeoutException as exc:
            raise AuthError(f"oauth token timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            raise AuthError(f"oauth token http error: {exc}") from exc

        if resp.status_code >= 400:
            raise AuthError(
                f"oauth token non-2xx ({resp.status_code})"
            )
        try:
            data = resp.json()
        except ValueError as exc:
            raise AuthError(f"oauth token response not JSON: {exc}") from exc

        access_token = data.get("access_token")
        if not isinstance(access_token, str) or not access_token:
            raise AuthError("oauth token response missing access_token")
        try:
            expires_in = float(data.get("expires_in", 0))
        except (TypeError, ValueError) as exc:
            raise AuthError(
                "oauth token response has invalid expires_in"
            ) from exc

        self._cached_token = access_token
        self._token_expires_at = (
            time.monotonic() + max(0.0, expires_in - _TOKEN_REFRESH_LEEWAY_S)
        )

    # --- Smoke + complete --------------------------------------------

    def _api_endpoint(self) -> str:
        return f"{self.api_base_url}/v1/chat/completions"

    async def _bearer_headers(
        self, *, content_type: str = "application/json"
    ) -> dict[str, str]:
        token = await self._get_token()
        return {
            "Authorization": f"Bearer {token}",
            "content-type": content_type,
        }

    async def smoke_test(self) -> None:  # noqa: D401
        body = {
            "model": self.model,
            "max_tokens": 4,
            "tools": [_PING_TOOL_OPENAI],
            "tool_choice": {"type": "function", "function": {"name": "ping"}},
            "messages": [{"role": "user", "content": "ping"}],
        }
        try:
            headers = await self._bearer_headers()
            async with httpx.AsyncClient(
                timeout=_SMOKE_TIMEOUT, verify=self._cfg.httpx_verify,
            ) as client:
                resp = await client.post(
                    self._api_endpoint(),
                    headers=headers,
                    json=body,
                )
        except AuthError:
            raise
        except httpx.TimeoutException as exc:
            raise RequestError(f"smoke-test timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            raise RequestError(f"smoke-test http error: {exc}") from exc

        if resp.status_code in (401, 403):
            raise AuthError(f"smoke-test auth failed ({resp.status_code})")
        if resp.status_code >= 400:
            raise RequestError(
                f"smoke-test non-2xx ({resp.status_code})"
            )

        try:
            data = resp.json()
        except ValueError as exc:
            raise RequestError(f"smoke-test response not JSON: {exc}") from exc

        if not _smoke_response_looks_valid("openai", data):
            raise RequestError(
                "smoke-test response missing tool-call shape; the configured "
                "endpoint did not honor the ping tool schema"
            )

    async def complete_stream(  # type: ignore[override]
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        body = {
            "model": self.model,
            "stream": True,
            "stream_options": {"include_usage": True},
            "max_tokens": 4096,
            "tools": tools,
            "messages": _messages_for_openai(messages),
        }
        try:
            headers = await self._bearer_headers()
            async with httpx.AsyncClient(
                timeout=_STREAM_TIMEOUT, verify=self._cfg.httpx_verify,
            ) as client:
                async with client.stream(
                    "POST",
                    self._api_endpoint(),
                    headers=headers,
                    json=body,
                ) as resp:
                    if resp.status_code in (401, 403):
                        raise AuthError(
                            f"streaming auth failed ({resp.status_code})"
                        )
                    if resp.status_code == 413:
                        raise ContextLimitError(
                            "request exceeded model context window"
                        )
                    if resp.status_code >= 400:
                        body_text = (await resp.aread()).decode("utf-8", "replace")
                        from .apikey import _is_context_limit_error
                        if _is_context_limit_error(body_text):
                            raise ContextLimitError(
                                "request exceeded model context window"
                            )
                        raise RequestError(
                            f"streaming non-2xx ({resp.status_code}): "
                            f"{body_text[:200]}"
                        )

                    async for event in _parse_openai_stream(resp):
                        yield event
        except httpx.TimeoutException as exc:
            raise RequestError(f"streaming timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            if isinstance(exc, (AuthError, RequestError)):
                raise
            raise RequestError(f"streaming http error: {exc}") from exc
