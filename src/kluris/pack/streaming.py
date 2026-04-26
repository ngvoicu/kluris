"""SSE encoding for agent events.

Translates the agent loop's event dicts into ``text/event-stream``
frames the browser receives. Browser-side, ``static/sse.js`` parses
``data:`` lines into JSON and routes them to the chat UI.

Common shapes:
- ``{"type": "token", "text": str}``
- ``{"type": "tool", "name": str, "args": dict}``
- ``{"type": "tool_result", "tool": str, "summary": str}``
- ``{"type": "usage", "input": int, "output": int}``
- ``{"type": "error", "message": str, "recoverable": bool}``
- ``[DONE]`` marker after the last event.
"""

from __future__ import annotations

import json
from typing import Any, AsyncIterator


def _frame(payload: dict[str, Any]) -> str:
    return f"data: {json.dumps(payload, ensure_ascii=False)}\n\n"


async def encode_sse(
    events: AsyncIterator[dict[str, Any]],
) -> AsyncIterator[str]:
    """Translate an agent event stream into SSE frames."""
    async for event in events:
        kind = event.get("kind")
        if kind == "token":
            yield _frame({"type": "token", "text": event.get("text", "")})
        elif kind == "tool":
            yield _frame({
                "type": "tool",
                "name": event.get("name", ""),
                "args": event.get("args", {}),
            })
        elif kind == "tool_result":
            yield _frame({
                "type": "tool_result",
                "tool": event.get("tool", ""),
                "summary": event.get("summary", ""),
            })
        elif kind == "usage":
            yield _frame({
                "type": "usage",
                "input": int(event.get("input", 0)),
                "output": int(event.get("output", 0)),
            })
        elif kind == "error":
            yield _frame({
                "type": "error",
                "message": event.get("message", ""),
                "recoverable": bool(event.get("recoverable", False)),
            })
        elif kind == "end":
            yield "data: [DONE]\n\n"
        else:
            # Unknown event type — pass through with a generic shape so
            # the UI doesn't silently drop it.
            yield _frame({"type": kind or "unknown", **event})
