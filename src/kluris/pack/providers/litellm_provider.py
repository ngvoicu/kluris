"""Single LiteLLM-backed LLM provider for the kluris pack chat server.

One :class:`LiteLLMProvider` replaces the two hand-rolled ``httpx`` providers
(``apikey.py`` + ``oauth.py``). Routing is expressed as LiteLLM **model
strings** computed in :mod:`kluris.pack.config` (``Config.litellm_model`` /
``Config.litellm_api_base`` — see those for the per-shape translation table),
so today's ``.env`` keeps working unchanged.

The agent loop and SSE layer are untouched: ``complete_stream`` still yields the
common ``{kind: token|tool_use|usage|end}`` event dicts, and ``smoke_test``
still fails fast at boot.

LiteLLM normalizes every provider to OpenAI-shaped messages, tool schemas, and
streamed chunks, so a single ``_messages_for_openai`` converter and a single
``_parse_litellm_stream`` parser cover all paths.
"""

from __future__ import annotations

import asyncio
import json
import time
from typing import Any, AsyncIterator

import httpx
import litellm

from ..config import Config
from ..middleware import redact_secrets
from .base import (
    AuthError,
    ContextLimitError,
    LLMProvider,
    RequestError,
)

_TOKEN_TIMEOUT = httpx.Timeout(connect=5.0, read=15.0, write=5.0, pool=5.0)
_TOKEN_REFRESH_LEEWAY_S = 30.0

# Boot smoke-test output budget. Larger than a single token because a reasoning
# model can spend output tokens on hidden reasoning before it emits the forced
# ``ping`` tool-call. The smoke deliberately sends NO ``reasoning_effort`` (an
# effort + tools call on a tiny budget can false-fail a perfectly good model).
_SMOKE_MAX_TOKENS = 32

# Tiny tool sent during the boot smoke-test, in OpenAI function shape. LiteLLM
# translates it to the Anthropic tool shape for the ``anthropic/`` path.
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
# Force the ping tool. On the ``openai/responses/`` path LiteLLM does not yet
# pass ``tool_choice`` through and drops it (``drop_params=True``); the smoke is
# structural (a non-empty completion), so a dropped force is harmless there.
_PING_TOOL_CHOICE = {"type": "function", "function": {"name": "ping"}}


def configure_litellm(config: Config) -> None:
    """Set the process-wide LiteLLM globals once at boot.

    - ``drop_params=True`` — silently drop params a given model/endpoint does
      not accept (e.g. ``tool_choice`` on the Responses API). Load-bearing for
      the Responses smoke-test, and a safety net behind the explicit per-shape
      gating in :meth:`LiteLLMProvider._auth_route_kwargs`.
    - TLS is set on TWO globals because they reach different handlers.
      ``aclient_session`` (a shared async httpx client) is read ONLY by
      LiteLLM's OpenAI handler — OpenAI Chat Completions, the Responses API, and
      OAuth gateways. The generic/Anthropic handler builds its own client and
      resolves TLS from ``ssl_verify``. Setting both makes every path honor
      ``KLURIS_CA_BUNDLE`` / ``KLURIS_TLS_INSECURE``. ``httpx_verify`` returns
      exactly the ``True | False | ssl.SSLContext`` forms both globals accept.
    """
    litellm.drop_params = True
    # Quiet LiteLLM's stdout debug banner (model/provider hints printed on some
    # errors) — it bypasses the logging-redaction filter and could echo request
    # detail to container logs.
    litellm.suppress_debug_info = True
    litellm.ssl_verify = config.httpx_verify
    litellm.aclient_session = httpx.AsyncClient(verify=config.httpx_verify)


class LiteLLMProvider(LLMProvider):
    """LiteLLM-backed provider covering api-key (Anthropic/OpenAI) + OAuth."""

    def __init__(self, config: Config) -> None:
        self._cfg = config
        self.model = config.model
        self._model_string = config.litellm_model
        self._api_base = config.litellm_api_base
        self._is_anthropic = config.is_anthropic_shape
        # Escape hatch for gateways that need more than a bearer; no env knob
        # today, kept as the documented seam.
        self._extra_headers: dict[str, str] | None = None

        # Static api-key bearer (api-key path). The OAuth path resolves a fresh
        # bearer per call via the token manager below.
        self._static_api_key = (
            config.api_key.get_secret_value() if config.api_key else ""
        )

        # --- OAuth client_credentials token manager (per-instance) ----------
        # Per-INSTANCE lock so two providers with different client_id refresh
        # independently. Single-flight + 30 s refresh leeway + in-memory cache,
        # lifted verbatim from the retired OAuthProvider.
        self._oauth = config.auth_mode == "oauth"
        self._refresh_lock = asyncio.Lock()
        self._cached_token: str | None = None
        self._token_expires_at: float = 0.0

    # --- Auth ----------------------------------------------------------------

    async def _resolve_api_key(self) -> str:
        if self._oauth:
            return await self._get_token()
        return self._static_api_key

    async def _get_token(self) -> str:
        now = time.monotonic()
        if self._cached_token and now < self._token_expires_at:
            return self._cached_token

        async with self._refresh_lock:
            # Recheck inside the lock — another waiter may have just refreshed.
            now = time.monotonic()
            if self._cached_token and now < self._token_expires_at:
                return self._cached_token
            await self._refresh_token()
            assert self._cached_token is not None
            return self._cached_token

    async def _refresh_token(self) -> None:
        cfg = self._cfg
        form = {
            "grant_type": "client_credentials",
            "client_id": cfg.oauth_client_id or "",
            "client_secret": (
                cfg.oauth_client_secret.get_secret_value()
                if cfg.oauth_client_secret
                else ""
            ),
        }
        if cfg.oauth_scope:
            form["scope"] = cfg.oauth_scope
        try:
            async with httpx.AsyncClient(
                timeout=_TOKEN_TIMEOUT, verify=cfg.httpx_verify,
            ) as client:
                resp = await client.post(cfg.oauth_token_url or "", data=form)
        except httpx.TimeoutException as exc:
            raise AuthError(f"oauth token timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            raise AuthError(f"oauth token http error: {exc}") from exc

        if resp.status_code >= 400:
            raise AuthError(f"oauth token non-2xx ({resp.status_code})")
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
            raise AuthError("oauth token response has invalid expires_in") from exc

        self._cached_token = access_token
        self._token_expires_at = (
            time.monotonic() + max(0.0, expires_in - _TOKEN_REFRESH_LEEWAY_S)
        )

    # --- Shared call kwargs --------------------------------------------------

    def _auth_route_kwargs(self, api_key: str) -> dict[str, Any]:
        """Auth + routing kwargs shared by smoke + stream.

        ``store=False`` (privacy: never retain the conversation) is sent only on
        the OpenAI-shaped paths — LiteLLM does not accept ``store`` for
        Anthropic, and the explicit gate avoids relying on ``drop_params``.
        """
        kwargs: dict[str, Any] = {"api_key": api_key}
        if self._api_base:
            kwargs["api_base"] = self._api_base
        if self._extra_headers:
            kwargs["extra_headers"] = self._extra_headers
        if not self._is_anthropic:
            kwargs["store"] = False
        return kwargs

    # --- Smoke test ----------------------------------------------------------

    async def smoke_test(self) -> None:  # noqa: D401  (interface)
        api_key = await self._resolve_api_key()
        kwargs = self._auth_route_kwargs(api_key)
        try:
            response = await litellm.acompletion(
                model=self._model_string,
                messages=[{"role": "user", "content": "ping"}],
                tools=[_PING_TOOL_OPENAI],
                tool_choice=_PING_TOOL_CHOICE,
                max_tokens=_SMOKE_MAX_TOKENS,
                stream=False,
                **kwargs,
            )
        except (AuthError, RequestError):
            raise
        except litellm.NotFoundError as exc:
            # 404/405 almost always means KLURIS_BASE_URL is the full endpoint
            # URL instead of the host root, or the model name is wrong.
            raise RequestError(
                f"smoke-test got 404 for model {self._model_string!r}: the "
                "endpoint or model is probably wrong. KLURIS_BASE_URL should be "
                "just the host root (e.g. https://openrouter.ai/api), and "
                "KLURIS_MODEL must name a model the endpoint serves."
            ) from exc
        except Exception as exc:  # noqa: BLE001  (mapped below)
            raise _mapped_error(exc) from exc

        choices = _get(response, "choices")
        if not (isinstance(choices, list) and len(choices) > 0):
            raise RequestError(
                "smoke-test response had no choices; the configured endpoint "
                "did not return a chat completion"
            )

    # --- Streaming chat ------------------------------------------------------

    async def complete_stream(  # type: ignore[override]
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        api_key = await self._resolve_api_key()
        kwargs = self._auth_route_kwargs(api_key)
        kwargs["max_tokens"] = self._cfg.max_output_tokens
        # Temperature is opt-in (omitted unless the deployer set it) so models
        # that reject an explicit temperature work by default.
        if self._cfg.temperature is not None:
            kwargs["temperature"] = self._cfg.temperature
        # ``stream_options`` and ``reasoning_effort`` are OpenAI-isms LiteLLM
        # rejects for Anthropic — gate them off there explicitly.
        if not self._is_anthropic:
            kwargs["stream_options"] = {"include_usage": True}
            if self._cfg.reasoning_effort:
                kwargs["reasoning_effort"] = self._cfg.reasoning_effort
        try:
            response = await litellm.acompletion(
                model=self._model_string,
                messages=_messages_for_openai(messages),
                tools=tools,
                stream=True,
                **kwargs,
            )
            async for event in _parse_litellm_stream(response):
                yield event
        except (AuthError, RequestError):
            raise
        except Exception as exc:  # noqa: BLE001  (mapped below)
            raise _mapped_error(exc) from exc


# --- Error mapping -----------------------------------------------------------


def _mapped_error(exc: Exception) -> Exception:
    """Map a LiteLLM/OpenAI exception onto the pack's provider error types.

    LiteLLM normalizes every provider's errors to the OpenAI exception
    hierarchy and preserves the provider's original message via ``str(exc)``,
    so the unmasked-error behavior (2.26.1) is kept: the deployer still sees the
    real reason in the chat error.
    """
    # ``str(exc)`` reaches the chat UI; OpenAI echoes a partial key in some
    # error bodies, so redact before surfacing it.
    detail = redact_secrets(str(exc))
    if isinstance(exc, (litellm.AuthenticationError, litellm.PermissionDeniedError)):
        return AuthError(detail)
    if isinstance(exc, litellm.ContextWindowExceededError):
        return ContextLimitError("request exceeded model context window")
    if isinstance(exc, litellm.BadRequestError) and _is_context_limit_error(str(exc)):
        return ContextLimitError("request exceeded model context window")
    return RequestError(detail)


def _is_context_limit_error(body_text: str) -> bool:
    """Heuristic detection of context-window errors across providers."""
    lower = body_text.lower()
    return any(
        marker in lower
        for marker in (
            "context_length_exceeded",
            "maximum context length",
            "too many tokens",
            "prompt is too long",
            "tokens exceeds",
        )
    )


# --- Outbound message conversion ---------------------------------------------


def _messages_for_openai(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Convert the agent loop's generic tool-call messages to OpenAI shape.

    LiteLLM takes OpenAI-shaped messages for EVERY provider and translates to
    each native shape, so this single converter covers all paths. ``system``
    stays ``role=system``; LiteLLM lifts it to Anthropic's top-level ``system``
    field internally.
    """
    out: list[dict[str, Any]] = []
    for msg in messages:
        role = msg.get("role")
        if role == "assistant" and msg.get("tool_calls"):
            out.append({
                "role": "assistant",
                "content": msg.get("content", ""),
                "tool_calls": [
                    {
                        "id": call.get("id", ""),
                        "type": "function",
                        "function": {
                            "name": call.get("name", ""),
                            "arguments": json.dumps(call.get("args", {})),
                        },
                    }
                    for call in msg.get("tool_calls", [])
                ],
            })
        elif role == "tool":
            out.append({
                "role": "tool",
                "tool_call_id": msg.get("tool_call_id") or msg.get("tool_use_id"),
                "content": msg.get("content", ""),
            })
        else:
            out.append({
                "role": role,
                "content": msg.get("content", ""),
            })
    return out


# --- Streaming parser --------------------------------------------------------


def _get(obj: Any, key: str, default: Any = None) -> Any:
    """Read ``key`` from a LiteLLM chunk object OR a plain dict.

    Production hits the ``getattr`` branch (real ``ModelResponseStream`` /
    ``Delta`` / ``ChatCompletionDeltaToolCall`` objects); the dict branch keeps
    the parser testable with plain dicts.
    """
    if obj is None:
        return default
    if isinstance(obj, dict):
        return obj.get(key, default)
    return getattr(obj, key, default)


async def _parse_litellm_stream(
    response: AsyncIterator[Any],
) -> AsyncIterator[dict[str, Any]]:
    """Translate LiteLLM's OpenAI-shaped streamed chunks into common events.

    Tracks per-call tool buffers so concurrent ``tool_calls`` are re-emitted
    with their full ``arguments`` JSON. Streaming text AND tool_calls in the
    same round both surface (the v1.87.0-fixed bug). Emits a zero-usage event
    at end-of-stream when the endpoint ignored ``stream_options.include_usage``
    so the UI footer still ticks.
    """
    tool_buffers: dict[int, dict[str, Any]] = {}
    saw_usage = False
    async for chunk in response:
        usage = _get(chunk, "usage")
        if usage:
            saw_usage = True
            yield {
                "kind": "usage",
                "input": int(_get(usage, "prompt_tokens", 0) or 0),
                "output": int(_get(usage, "completion_tokens", 0) or 0),
            }

        choices = _get(chunk, "choices") or []
        if not choices:
            continue
        choice0 = choices[0]
        delta = _get(choice0, "delta")

        text = _get(delta, "content")
        if text:
            yield {"kind": "token", "text": text}

        for tc in _get(delta, "tool_calls") or []:
            idx = _get(tc, "index", 0) or 0
            buf = tool_buffers.setdefault(
                idx, {"name": None, "id": None, "json": ""}
            )
            fn = _get(tc, "function")
            name = _get(fn, "name")
            if name:
                buf["name"] = name
            tc_id = _get(tc, "id")
            if tc_id:
                buf["id"] = tc_id
            args_chunk = _get(fn, "arguments")
            if args_chunk:
                buf["json"] += args_chunk

        finish = _get(choice0, "finish_reason")
        if finish:
            for buf in tool_buffers.values():
                if buf["name"]:
                    try:
                        args = json.loads(buf["json"]) if buf["json"] else {}
                    except ValueError:
                        args = {}
                    yield {
                        "kind": "tool_use",
                        "name": buf["name"],
                        "id": buf["id"],
                        "args": args,
                    }
            tool_buffers.clear()

    if not saw_usage:
        # Graceful degradation: some endpoints ignore include_usage silently.
        yield {"kind": "usage", "input": 0, "output": 0}
    yield {"kind": "end"}
