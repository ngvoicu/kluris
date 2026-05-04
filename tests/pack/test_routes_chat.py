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


def test_brain_tree_endpoint_returns_lobes_and_glossary(api_key_config: Config):
    """``GET /api/brain/tree`` must return the same wake_up payload
    the LLM sees — lobes, recent, glossary, brain.md body. The
    sidebar in the chat UI builds the tree from this.
    """
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get("/api/brain/tree")
        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True
        lobe_names = {l["name"] for l in data["lobes"]}
        # fixture brain has projects / knowledge / infrastructure
        assert {"projects", "knowledge", "infrastructure"} <= lobe_names
        # glossary terms come through
        gloss_terms = {e["term"] for e in data["glossary"]}
        assert "JWT" in gloss_terms
        # brain.md body present
        assert isinstance(data["brain_md"], str)


def test_brain_neuron_endpoint_returns_frontmatter_and_body(
    api_key_config: Config,
):
    """``GET /api/brain/neuron?path=…`` returns the neuron's
    frontmatter, body, and ``deprecated`` flag — sandboxed under
    the brain root.
    """
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get(
            "/api/brain/neuron",
            params={"path": "knowledge/jwt.md"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True
        assert "JSON Web Tokens" in data["body"]
        assert data["deprecated"] is False
        assert isinstance(data["frontmatter"], dict)


def test_brain_neuron_endpoint_404_on_missing_path(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get(
            "/api/brain/neuron",
            params={"path": "knowledge/does-not-exist.md"},
        )
        assert resp.status_code == 404
        data = resp.json()
        assert data["ok"] is False
        assert "not_found" in data["error"]


def test_brain_neuron_endpoint_400_on_path_traversal(api_key_config: Config):
    """The sandbox rejects ``../`` traversal; the route maps that to
    a 400 — the chat UI never sees host filesystem content.
    """
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get(
            "/api/brain/neuron",
            params={"path": "../../etc/passwd"},
        )
        assert resp.status_code == 400
        data = resp.json()
        assert data["ok"] is False
        assert "sandbox" in data["error"]


def test_brain_lobe_endpoint_returns_overview(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get("/api/brain/lobe", params={"lobe": "knowledge"})
        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True
        assert data["lobe"] == "knowledge"
        # map_body comes through verbatim (large budget set on the
        # endpoint so the human reader doesn't get truncation).
        assert isinstance(data["map_body"], str)
        # at least the JWT neuron is reachable from the knowledge lobe
        paths = {n["path"] for n in data["neurons"]}
        assert "knowledge/jwt.md" in paths


def test_brain_lobe_endpoint_404_on_missing_lobe(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        resp = client.get("/api/brain/lobe", params={"lobe": "does-not-exist"})
        assert resp.status_code == 404
        assert resp.json()["ok"] is False


def test_brain_endpoints_dont_require_auth(api_key_config: Config):
    """The brain explorer is intentionally unauthenticated — same
    threat model as the chat UI. Public exposure is the deployer's
    responsibility (reverse proxy / VPN / cloud IAM).
    """
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        for path in ("/api/brain/tree",
                     "/api/brain/files",
                     "/api/brain/neuron?path=knowledge/jwt.md",
                     "/api/brain/lobe?lobe=knowledge",
                     "/api/brain/search?q=jwt"):
            resp = client.get(path)
            assert resp.status_code == 200, (
                f"{path} should be reachable without auth, got {resp.status_code}"
            )
            assert "WWW-Authenticate" not in resp.headers


def test_brain_search_endpoint_returns_ranked_results(api_key_config: Config):
    """``GET /api/brain/search?q=…`` returns the same ranked-result
    shape the agent's ``search`` tool emits — neurons + glossary
    matches, scored by ``search_brain``. Empty query returns an
    empty result list without 400'ing.
    """
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        # Empty query short-circuits to an empty result list.
        empty = client.get("/api/brain/search", params={"q": ""}).json()
        assert empty["ok"] is True
        assert empty["total"] == 0
        assert empty["results"] == []

        # Real query — fixture brain has a JWT neuron in `knowledge`,
        # so a `jwt` query must surface it.
        resp = client.get("/api/brain/search", params={"q": "jwt"})
        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True
        assert data["query"] == "jwt"
        assert isinstance(data["results"], list)
        assert data["total"] >= 1
        files = {r["file"] for r in data["results"]}
        assert "knowledge/jwt.md" in files
        # Each result must carry the contract the result-card renderer reads.
        for r in data["results"]:
            assert {"file", "title", "matched_fields",
                    "snippet", "score", "deprecated"} <= set(r.keys())


def test_brain_search_endpoint_finds_glossary_entries(
    api_key_config: Config, tmp_path,
):
    """Right-panel search must reach inside ``glossary.md`` — not just
    file names. A query that matches a glossary term (or its
    definition) shows up as a result with ``file == "glossary.md"``
    and the term as the ``title``.
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text(
        "# Glossary\n\n"
        "**OAuth** -- Open authorization protocol used for SSO.\n"
        "**JWT** -- JSON Web Token.\n",
        encoding="utf-8",
    )
    cfg = api_key_config.model_copy(update={"brain_dir": brain})
    app = _build_app(cfg)
    with TestClient(app) as client:
        # Term match → matched_fields includes "title".
        oauth = client.get(
            "/api/brain/search", params={"q": "oauth"},
        ).json()
        oauth_glossary = [
            r for r in oauth["results"] if r["file"] == "glossary.md"
        ]
        assert oauth_glossary, "OAuth term must surface from glossary"
        assert oauth_glossary[0]["title"].lower() == "oauth"

        # Definition match → matched_fields includes "body".
        token = client.get(
            "/api/brain/search", params={"q": "token"},
        ).json()
        token_glossary = [
            r for r in token["results"] if r["file"] == "glossary.md"
        ]
        assert token_glossary, "JWT definition body must surface from glossary"


def test_brain_search_endpoint_clamps_limit(api_key_config: Config):
    """``limit`` is clamped to [1, 50] so callers can't request huge
    result lists. Out-of-range / non-int input falls back to the
    default of 20."""
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        # A real query ensures we don't trip the empty-query short-circuit
        # before the clamp logic runs.
        resp = client.get(
            "/api/brain/search",
            params={"q": "knowledge", "limit": "9999"},
        )
        assert resp.status_code == 200
        assert len(resp.json()["results"]) <= 50

        resp = client.get(
            "/api/brain/search",
            params={"q": "knowledge", "limit": "not-a-number"},
        )
        assert resp.status_code == 200


def test_post_chat_new_rotates_cookie(api_key_config: Config):
    app = _build_app(api_key_config)
    with TestClient(app) as client:
        first = client.get("/")
        old_cookie = first.cookies.get("kluris_session")
        new_resp = client.post("/chat/new")
        assert new_resp.status_code == 200
        new_cookie = new_resp.cookies.get("kluris_session")
        assert new_cookie and new_cookie != old_cookie
