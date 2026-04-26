"""Provider-agnostic interface for the chat server's LLM boundary.

Concrete providers (:class:`APIKeyProvider`, :class:`OAuthProvider`)
implement two methods:

- :meth:`smoke_test` — sends a tiny ``ping`` tool schema at boot and
  raises if the configured endpoint cannot be reached, refuses the
  tool schema, or times out. Result drives the fail-fast at app boot.
- :meth:`complete_stream` — streams a chat completion as
  ``{kind: "token"|"tool_use"|"usage"|"end", ...}`` events, normalizing
  Anthropic and OpenAI shapes into one dict shape so the agent loop and
  SSE streaming layer stay provider-agnostic.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any, AsyncIterator


class ProviderError(RuntimeError):
    """Base class for any provider-level failure."""


class AuthError(ProviderError):
    """Authentication failed: bad key, expired token, refresh failed."""


class RequestError(ProviderError):
    """Provider HTTP / protocol error not attributable to bad credentials."""


class ContextLimitError(RequestError):
    """The request exceeded the model's context window.

    Surfaced to the chat UI as a clear "start a new conversation"
    message so the user does not see raw provider tracebacks.
    """


class LLMProvider(ABC):
    """The minimal contract the agent loop relies on."""

    model: str

    @abstractmethod
    async def smoke_test(self) -> None:
        """Validate the configured endpoint can serve tool-calling requests.

        Sends a tiny ``ping`` tool schema. Raises :class:`AuthError`
        for non-2xx auth failures, :class:`RequestError` for any other
        non-2xx response or timeout, or for malformed/missing tool-call
        responses.
        """

    @abstractmethod
    def complete_stream(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        """Stream a chat completion as event dicts.

        Yields dicts with ``kind`` ∈ {``"token"``, ``"tool_use"``,
        ``"tool_result_request"``, ``"usage"``, ``"end"``,
        ``"error"``}. The agent loop dispatches on ``kind``.
        """
