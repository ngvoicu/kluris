"""TEST-PACK-45 — chat routes."""

from __future__ import annotations

import json

import pytest
from fastapi.testclient import TestClient

from kluris.pack.config import Config
from kluris.pack.main import create_app
from kluris.pack.providers.base import LLMProvider


class _ScriptedProvider(LLMProvider):
    """Provider that emits a single canned response."""

    model = "scripted"

    async def smoke_test(self) -> None:
        return None

    async def complete_stream(self, messages, tools):
        yield {"kind": "token", "text": "hello"}
        yield {"kind": "token", "text": " world"}
        yield {"kind": "usage", "input": 5, "output": 2}
        yield {"kind": "end"}


def _build_app(api_key_config: Config):
    return create_app(
        config=api_key_config,
        provider=_ScriptedProvider(),
        allow_writable_brain=True,
    )


def test_get_root_returns_html(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get("/")
        assert resp.status_code == 200
        assert "<html" in resp.text.lower()
        assert "fixture-brain" in resp.text or "Fixture Brain" in resp.text


def test_get_root_sets_session_cookie(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get("/")
        assert "kluris_session" in resp.cookies


def test_post_chat_streams_tokens(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        # Establish session
        client.get("/")
        resp = client.post("/chat", json={"message": "hi"})
        assert resp.status_code == 200
        assert resp.headers["content-type"].startswith("text/event-stream")
        # Drain the stream
        text = resp.text
        assert "data: " in text
        assert "[DONE]" in text


def test_post_chat_persists_history(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        client.get("/")
        client.post("/chat", json={"message": "first"})
        # Reload to confirm history replays
        resp = client.get("/")
        assert "first" in resp.text


def test_post_chat_persists_agent_error_when_no_text(api_key_config: Config):
    """If the agent emitted only an error (no tokens), the route must
    still persist something to history so a page reload shows the
    user that this turn failed — instead of a blank assistant block.
    """
    from kluris.pack.providers.base import LLMProvider

    class _EmptyResponseProvider(LLMProvider):
        """Provider that returns a bare ``end`` event — no tokens, no
        tool_use. The agent loop turns that into an error.
        """

        model = "empty"

        async def smoke_test(self) -> None:
            return None

        async def complete_stream(self, messages, tools):
            yield {"kind": "end"}

    app = create_app(
        config=api_key_config,
        provider=_EmptyResponseProvider(),
        allow_writable_brain=True,
    )
    with TestClient(app) as client:
        client.get("/")
        client.post("/chat", json={"message": "what is x?"})
        resp = client.get("/")
        # The error must be visible in the replayed history.
        assert "[error:" in resp.text
        assert "no content" in resp.text.lower()


def test_post_chat_empty_message_400(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        client.get("/")
        resp = client.post("/chat", json={"message": "   "})
        assert resp.status_code == 400


def test_post_chat_new_rotates_cookie(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        first = client.get("/")
        old_cookie = first.cookies.get("kluris_session")
        new_resp = client.post("/chat/new")
        assert new_resp.status_code == 200
        new_cookie = new_resp.cookies.get("kluris_session")
        assert new_cookie and new_cookie != old_cookie
