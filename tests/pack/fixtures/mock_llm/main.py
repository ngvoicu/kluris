"""Deterministic mock LLM for the Docker e2e test.

Exposes both Anthropic and OpenAI shapes. Canned responses are keyed
off the user message text so the e2e exercises both the smoke-test
path and a typical chat round trip without a real LLM SDK.
"""

from __future__ import annotations

import json
from typing import AsyncIterator

from fastapi import FastAPI, Header, HTTPException, Request
from fastapi.responses import JSONResponse, StreamingResponse

app = FastAPI()


def _user_message(messages: list[dict]) -> str:
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content")
            if isinstance(content, str):
                return content
            if isinstance(content, list):
                for piece in content:
                    if isinstance(piece, dict) and piece.get("type") == "text":
                        return piece.get("text", "")
    return ""


# --- Anthropic shape -------------------------------------------------


@app.post("/v1/messages")
async def anthropic_messages(
    request: Request,
    x_api_key: str = Header(default=""),
    anthropic_version: str = Header(default=""),
):
    if not x_api_key:
        raise HTTPException(status_code=401, detail="missing x-api-key")
    body = await request.json()
    user = _user_message(body.get("messages", []))
    if user == "ping" or body.get("max_tokens", 0) <= 4:
        return JSONResponse({
            "id": "msg-mock",
            "content": [{
                "type": "tool_use", "id": "tu1", "name": "ping",
                "input": {"value": "pong"},
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })

    async def stream():
        for piece in ("Hello ", "from ", "the ", "mock."):
            yield (
                "event: content_block_delta\n"
                "data: " + json.dumps({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": piece},
                }) + "\n\n"
            )
        yield (
            "event: message_delta\n"
            "data: " + json.dumps({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn"},
                "usage": {"input_tokens": 5, "output_tokens": 4},
            }) + "\n\n"
        )
        yield "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"

    return StreamingResponse(stream(), media_type="text/event-stream")


# --- OpenAI shape ----------------------------------------------------


@app.post("/v1/chat/completions")
async def openai_chat(
    request: Request,
    authorization: str = Header(default=""),
):
    if not authorization.lower().startswith("bearer "):
        raise HTTPException(status_code=401, detail="missing bearer")
    body = await request.json()
    user = _user_message(body.get("messages", []))
    include_usage = (body.get("stream_options") or {}).get("include_usage", False)

    if user == "ping" or body.get("max_tokens", 0) <= 4:
        return JSONResponse({
            "id": "cmpl-mock",
            "choices": [{"message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "c1", "type": "function",
                    "function": {
                        "name": "ping",
                        "arguments": json.dumps({"value": "pong"}),
                    },
                }],
            }}],
        })

    async def stream():
        for piece in ("Hello ", "from ", "the ", "mock."):
            yield "data: " + json.dumps({
                "choices": [{"delta": {"content": piece}}]
            }) + "\n\n"
        if include_usage and user != "no-usage":
            yield "data: " + json.dumps({
                "choices": [],
                "usage": {"prompt_tokens": 5, "completion_tokens": 4},
            }) + "\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(stream(), media_type="text/event-stream")
