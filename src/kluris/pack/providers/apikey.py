"""API-key-shape LLM provider.

Supports two endpoint shapes selected at deploy time via
``KLURIS_PROVIDER_SHAPE``:

- ``anthropic``: ``POST <base>/v1/messages`` with
  ``x-api-key`` + ``anthropic-version`` headers.
- ``openai``: ``POST <base>/v1/chat/completions`` with
  ``Authorization: Bearer``.

Both shapes converge on a common :class:`AsyncIterator[dict]` event
stream so the agent loop and SSE layer don't branch on provider type.
"""

from __future__ import annotations

import json
from typing import Any, AsyncIterator

import httpx

from ..config import Config
from .base import (
    AuthError,
    ContextLimitError,
    LLMProvider,
    RequestError,
)

_SMOKE_TIMEOUT = httpx.Timeout(connect=5.0, read=15.0, write=5.0, pool=5.0)
_STREAM_TIMEOUT = httpx.Timeout(connect=5.0, read=60.0, write=5.0, pool=5.0)

# Tiny tool schema sent during boot smoke-test. We force/select it to
# verify the configured endpoint actually supports tool-calling.
_PING_TOOL_ANTHROPIC = {
    "name": "ping",
    "description": "Echo a single token. Used by Kluris boot smoke-test.",
    "input_schema": {
        "type": "object",
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
    },
}

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


class APIKeyProvider(LLMProvider):
    """Anthropic-shape or OpenAI-shape provider with API-key auth."""

    def __init__(self, config: Config) -> None:
        if config.provider_shape not in {"anthropic", "openai"}:
            raise RequestError(
                f"unsupported provider shape: {config.provider_shape!r}"
            )
        self._cfg = config
        self.shape = config.provider_shape
        self.base_url = config.base_url or ""
        self._api_key = (
            config.api_key.get_secret_value() if config.api_key else ""
        )
        self.model = config.model

    # --- Headers -------------------------------------------------------

    def _headers(self, *, content_type: str = "application/json") -> dict[str, str]:
        if self.shape == "anthropic":
            return {
                "x-api-key": self._api_key,
                "anthropic-version": self._cfg.anthropic_version,
                "content-type": content_type,
            }
        return {
            "Authorization": f"Bearer {self._api_key}",
            "content-type": content_type,
        }

    def _endpoint(self) -> str:
        suffix = "/v1/messages" if self.shape == "anthropic" else "/v1/chat/completions"
        return f"{self.base_url}{suffix}"

    # --- Smoke test ----------------------------------------------------

    async def smoke_test(self) -> None:  # noqa: D401  (interface)
        body = self._smoke_body()
        try:
            async with httpx.AsyncClient(
                timeout=_SMOKE_TIMEOUT, verify=self._cfg.httpx_verify,
            ) as client:
                resp = await client.post(
                    self._endpoint(),
                    headers=self._headers(),
                    json=body,
                )
        except httpx.TimeoutException as exc:
            raise RequestError(f"smoke-test timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            raise RequestError(f"smoke-test http error: {exc}") from exc

        if resp.status_code in (401, 403):
            raise AuthError(
                f"smoke-test auth failed ({resp.status_code}): "
                "check KLURIS_API_KEY"
            )
        if resp.status_code >= 400:
            raise RequestError(
                f"smoke-test non-2xx ({resp.status_code}); "
                f"endpoint may not support tool-calling"
            )

        try:
            data = resp.json()
        except ValueError as exc:
            raise RequestError(f"smoke-test response not JSON: {exc}") from exc

        if not _smoke_response_looks_valid(self.shape, data):
            raise RequestError(
                "smoke-test response missing tool-call shape; "
                "the configured endpoint did not honor the ping tool schema"
            )

    def _smoke_body(self) -> dict[str, Any]:
        if self.shape == "anthropic":
            return {
                "model": self.model,
                "max_tokens": 4,
                "tools": [_PING_TOOL_ANTHROPIC],
                "tool_choice": {"type": "tool", "name": "ping"},
                "messages": [{"role": "user", "content": "ping"}],
            }
        return {
            "model": self.model,
            "max_tokens": 4,
            "tools": [_PING_TOOL_OPENAI],
            "tool_choice": {"type": "function", "function": {"name": "ping"}},
            "messages": [{"role": "user", "content": "ping"}],
        }

    # --- Streaming chat ------------------------------------------------

    async def complete_stream(  # type: ignore[override]
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        body = self._stream_body(messages, tools)
        try:
            async with httpx.AsyncClient(
                timeout=_STREAM_TIMEOUT, verify=self._cfg.httpx_verify,
            ) as client:
                async with client.stream(
                    "POST",
                    self._endpoint(),
                    headers=self._headers(),
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
                        # Look for context-window markers in the body.
                        body_text = (await resp.aread()).decode("utf-8", "replace")
                        if _is_context_limit_error(body_text):
                            raise ContextLimitError(
                                "request exceeded model context window"
                            )
                        raise RequestError(
                            f"streaming non-2xx ({resp.status_code}): "
                            f"{body_text[:200]}"
                        )

                    if self.shape == "anthropic":
                        async for event in _parse_anthropic_stream(resp):
                            yield event
                    else:
                        async for event in _parse_openai_stream(resp):
                            yield event
        except httpx.TimeoutException as exc:
            raise RequestError(f"streaming timeout: {exc}") from exc
        except httpx.HTTPError as exc:
            if isinstance(exc, (AuthError, RequestError)):
                raise
            raise RequestError(f"streaming http error: {exc}") from exc

    def _stream_body(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> dict[str, Any]:
        if self.shape == "anthropic":
            system, anthropic_messages = _messages_for_anthropic(messages)
            return {
                "model": self.model,
                "max_tokens": 4096,
                "stream": True,
                "tools": tools,
                "messages": anthropic_messages,
                **({"system": system} if system else {}),
            }
        return {
            "model": self.model,
            "stream": True,
            "stream_options": {"include_usage": True},
            "max_tokens": 4096,
            "tools": tools,
            "messages": _messages_for_openai(messages),
        }


# --- Smoke-test response validation ---------------------------------------


def _smoke_response_looks_valid(shape: str, data: dict[str, Any]) -> bool:
    """Return True iff ``data`` is structurally a chat completion.

    The boot smoke-test answers a narrower question than originally
    spec'd: "is the configured endpoint reachable, authenticated, and
    speaking the right API contract?" — NOT "does it honor a forced
    ``tool_choice``?"

    Some real-world gateways (Azure OpenAI passthrough, vendor proxies,
    on-prem Bedrock fronts) silently ignore ``tool_choice`` even when
    the underlying model supports tool-calling perfectly well at chat
    time. Failing the boot in those cases punishes deployers for a
    behavioral quirk that doesn't actually break the chat.

    The check is therefore structural only:

    - Anthropic: ``content`` is a non-empty list. (Either a ``text``
      block or a ``tool_use`` block both count.)
    - OpenAI: ``choices`` is a non-empty list. (Tool-calls or plain
      content delta both count.)

    Anything else (HTML error pages from a proxy, a JSON envelope that
    doesn't match the API shape, an empty body) still fails fast.
    Deployers who want to bypass even this can set
    ``KLURIS_SKIP_BOOT_SMOKE=1`` and skip the smoke step entirely.
    """
    if shape == "anthropic":
        content = data.get("content")
        return isinstance(content, list) and len(content) > 0

    choices = data.get("choices")
    return (
        isinstance(choices, list)
        and len(choices) > 0
        and isinstance(choices[0], dict)
    )


# --- Outbound message conversion ---------------------------------------------


def _messages_for_openai(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Convert the agent loop's generic tool-call messages to OpenAI shape."""
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


def _messages_for_anthropic(
    messages: list[dict[str, Any]],
) -> tuple[str, list[dict[str, Any]]]:
    """Convert generic messages to Anthropic Messages API shape.

    The agent loop keeps one provider-neutral transcript. Anthropic wants
    system text as a top-level field and tool results as user content
    blocks, so the conversion happens immediately before the HTTP POST.
    """
    system_parts: list[str] = []
    out: list[dict[str, Any]] = []
    for msg in messages:
        role = msg.get("role")
        if role == "system":
            content = msg.get("content", "")
            if content:
                system_parts.append(str(content))
            continue
        if role == "assistant" and msg.get("tool_calls"):
            content_blocks: list[dict[str, Any]] = []
            content = msg.get("content", "")
            if content:
                content_blocks.append({"type": "text", "text": str(content)})
            content_blocks.extend(
                {
                    "type": "tool_use",
                    "id": call.get("id", ""),
                    "name": call.get("name", ""),
                    "input": call.get("args", {}),
                }
                for call in msg.get("tool_calls", [])
            )
            out.append({
                "role": "assistant",
                "content": content_blocks,
            })
        elif role == "tool":
            out.append({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": msg.get("tool_call_id") or msg.get("tool_use_id"),
                    "content": msg.get("content", ""),
                }],
            })
        elif role in {"user", "assistant"}:
            out.append({
                "role": role,
                "content": msg.get("content", ""),
            })
    return "\n\n".join(system_parts), out


# --- Streaming parsers ----------------------------------------------------


async def _iter_sse_lines(resp: httpx.Response) -> AsyncIterator[str]:
    async for line in resp.aiter_lines():
        if line:
            yield line


async def _parse_anthropic_stream(
    resp: httpx.Response,
) -> AsyncIterator[dict[str, Any]]:
    """Translate Anthropic SSE events into common event dicts.

    Relevant event types:
    - ``message_start`` / ``content_block_start`` — ignored
    - ``content_block_delta`` with ``delta.type == "text_delta"`` —
      yields ``{kind: "token", text: str}``
    - ``content_block_delta`` with ``delta.type == "input_json_delta"``
      — accumulates into a tool_use buffer
    - ``content_block_start`` with ``content_block.type == "tool_use"``
      — opens a tool_use buffer
    - ``content_block_stop`` for tool_use — yields ``{kind: "tool_use",
      name, args}``
    - ``message_delta`` containing usage — yields
      ``{kind: "usage", input, output}``
    - ``message_stop`` — yields ``{kind: "end"}``
    """
    tool_buffers: dict[int, dict[str, Any]] = {}
    event_type: str | None = None
    async for line in _iter_sse_lines(resp):
        if line.startswith("event:"):
            event_type = line.split(":", 1)[1].strip()
            continue
        if not line.startswith("data:"):
            continue
        raw = line.split(":", 1)[1].strip()
        if not raw or raw == "[DONE]":
            continue
        try:
            payload = json.loads(raw)
        except ValueError:
            continue

        kind = payload.get("type") or event_type
        if kind == "content_block_start":
            block = payload.get("content_block", {})
            if block.get("type") == "tool_use":
                tool_buffers[payload.get("index", 0)] = {
                    "name": block.get("name"),
                    "id": block.get("id"),
                    "json": "",
                }
        elif kind == "content_block_delta":
            delta = payload.get("delta", {})
            dtype = delta.get("type")
            if dtype == "text_delta":
                text = delta.get("text", "")
                if text:
                    yield {"kind": "token", "text": text}
            elif dtype == "input_json_delta":
                idx = payload.get("index", 0)
                if idx in tool_buffers:
                    tool_buffers[idx]["json"] += delta.get("partial_json", "")
        elif kind == "content_block_stop":
            idx = payload.get("index", 0)
            buf = tool_buffers.pop(idx, None)
            if buf is not None and buf.get("name"):
                args: dict[str, Any]
                try:
                    args = json.loads(buf["json"]) if buf["json"] else {}
                except ValueError:
                    args = {}
                yield {
                    "kind": "tool_use",
                    "name": buf["name"],
                    "id": buf.get("id"),
                    "args": args,
                }
        elif kind == "message_delta":
            usage = payload.get("usage", {})
            if usage:
                yield {
                    "kind": "usage",
                    "input": int(usage.get("input_tokens", 0)),
                    "output": int(usage.get("output_tokens", 0)),
                }
        elif kind == "message_stop":
            yield {"kind": "end"}


async def _parse_openai_stream(
    resp: httpx.Response,
) -> AsyncIterator[dict[str, Any]]:
    """Translate OpenAI Chat Completions SSE chunks into common dicts.

    Tracks per-call tool buffers so concurrent ``tool_calls`` are
    re-emitted with their full ``arguments`` JSON. Emits
    ``{kind: "usage", input: 0, output: 0}`` at end-of-stream when the
    proxy ignores ``stream_options.include_usage`` so the UI still sees
    a usage event for that turn.
    """
    tool_buffers: dict[int, dict[str, Any]] = {}
    saw_usage = False
    async for line in _iter_sse_lines(resp):
        if not line.startswith("data:"):
            continue
        raw = line.split(":", 1)[1].strip()
        if not raw:
            continue
        if raw == "[DONE]":
            break
        try:
            chunk = json.loads(raw)
        except ValueError:
            continue

        usage = chunk.get("usage")
        if usage:
            saw_usage = True
            yield {
                "kind": "usage",
                "input": int(usage.get("prompt_tokens", 0)),
                "output": int(usage.get("completion_tokens", 0)),
            }

        choices = chunk.get("choices") or []
        if not choices:
            continue
        delta = choices[0].get("delta", {}) or {}

        text = delta.get("content")
        if text:
            yield {"kind": "token", "text": text}

        for tc in delta.get("tool_calls", []) or []:
            idx = tc.get("index", 0)
            buf = tool_buffers.setdefault(
                idx, {"name": None, "id": None, "json": ""}
            )
            fn = tc.get("function", {})
            if fn.get("name"):
                buf["name"] = fn["name"]
            if tc.get("id"):
                buf["id"] = tc["id"]
            args_chunk = fn.get("arguments")
            if args_chunk:
                buf["json"] += args_chunk

        finish = choices[0].get("finish_reason")
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
        # Graceful degradation: some OpenAI-compatible proxies ignore
        # ``stream_options.include_usage`` silently. Emit a zero-usage
        # event so the UI footer still ticks.
        yield {"kind": "usage", "input": 0, "output": 0}
    yield {"kind": "end"}


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
