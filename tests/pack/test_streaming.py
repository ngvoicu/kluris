"""TEST-PACK-43 — SSE encoding of agent events."""

from __future__ import annotations

import json
from typing import AsyncIterator

import pytest

from kluris.pack.streaming import encode_sse

pytestmark = pytest.mark.asyncio


async def _from_list(events: list[dict]) -> AsyncIterator[dict]:
    for e in events:
        yield e


async def _collect(stream) -> list[str]:
    return [chunk async for chunk in stream]


def _payloads(frames: list[str]) -> list[dict]:
    out: list[dict] = []
    for frame in frames:
        for line in frame.splitlines():
            if line.startswith("data:"):
                payload = line.split(":", 1)[1].strip()
                if payload and payload != "[DONE]":
                    out.append(json.loads(payload))
    return out


async def test_token_event_encoded():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "token", "text": "hello"},
        {"kind": "end"},
    ])))
    payloads = _payloads(frames)
    assert payloads[0] == {"type": "token", "text": "hello"}
    assert frames[-1].strip() == "data: [DONE]"


async def test_tool_event_encoded():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "tool", "name": "search", "args": {"query": "x"}},
        {"kind": "end"},
    ])))
    payloads = _payloads(frames)
    assert payloads[0] == {"type": "tool", "name": "search", "args": {"query": "x"}}


async def test_tool_result_event_encoded():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "tool_result", "tool": "search", "summary": "3 hits"},
        {"kind": "end"},
    ])))
    payloads = _payloads(frames)
    assert payloads[0] == {
        "type": "tool_result", "tool": "search", "summary": "3 hits",
    }


async def test_usage_event_encoded():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "usage", "input": 9, "output": 4},
        {"kind": "end"},
    ])))
    payloads = _payloads(frames)
    assert payloads[0] == {"type": "usage", "input": 9, "output": 4}


async def test_error_event_encoded():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "error", "message": "boom", "recoverable": True},
        {"kind": "end"},
    ])))
    payloads = _payloads(frames)
    assert payloads[0] == {
        "type": "error", "message": "boom", "recoverable": True,
    }


async def test_done_marker_after_end():
    frames = await _collect(encode_sse(_from_list([
        {"kind": "token", "text": "x"},
        {"kind": "usage", "input": 1, "output": 1},
        {"kind": "end"},
    ])))
    assert frames[-1].strip() == "data: [DONE]"
