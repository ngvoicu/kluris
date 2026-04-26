"""Scripted provider for offline answer-quality evals.

Drives the agent loop with deterministic event sequences so CI can
assert on tool traces and answer shape without hitting a real LLM.
The provider exposes both Anthropic-like and OpenAI-like emission
modes; the chat server doesn't care which because the agent loop
already normalizes events at the provider boundary.
"""

from __future__ import annotations

from typing import Any, AsyncIterator

from kluris.pack.providers.base import LLMProvider


class ScriptedProvider(LLMProvider):
    """Plays back a list of pre-baked event lists, one per turn."""

    model = "scripted-eval"

    def __init__(self, scripts: list[list[dict[str, Any]]]) -> None:
        self._scripts = list(scripts)
        self.calls = 0

    async def smoke_test(self) -> None:
        return None

    async def complete_stream(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        if not self._scripts:
            return
        script = self._scripts.pop(0)
        self.calls += 1
        for ev in script:
            yield ev
