"""Access gate + rate limit + single-walk boot wiring (2.28.0)."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from kluris.pack.config import Config
from kluris.pack.main import create_app

pytest_plugins: list[str] = []


def _app(api_key_env, fixture_brain, tmp_path, stub_provider, **extra):
    env = dict(
        api_key_env,
        KLURIS_BRAIN_DIR=str(fixture_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        **extra,
    )
    (tmp_path / "data").mkdir(exist_ok=True)
    cfg = Config.load_from_env(env)
    return create_app(
        config=cfg, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )


# --- access gate ---------------------------------------------------------------


def test_no_token_keeps_server_open_with_boot_warning(
    api_key_env, fixture_brain, tmp_path, stub_provider, capsys
):
    app = _app(api_key_env, fixture_brain, tmp_path, stub_provider)
    assert "KLURIS_ACCESS_TOKEN is not set" in capsys.readouterr().err
    with TestClient(app) as client:
        assert client.get("/api/brain/files").status_code == 200


def test_access_token_gates_everything_but_healthz(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    app = _app(
        api_key_env, fixture_brain, tmp_path, stub_provider,
        KLURIS_ACCESS_TOKEN="s3cret",
    )
    with TestClient(app) as client:
        # /healthz stays open for container healthchecks.
        assert client.get("/healthz").status_code == 200
        # Everything else is 401 without the token.
        assert client.get("/api/brain/files").status_code == 401
        assert client.get("/").status_code == 401
        assert client.post("/chat", json={"message": "hi"}).status_code == 401
        # Bearer header passes.
        ok = client.get(
            "/api/brain/files",
            headers={"Authorization": "Bearer s3cret"},
        )
        assert ok.status_code == 200
        # Wrong token still 401.
        bad = client.get(
            "/api/brain/files",
            headers={"Authorization": "Bearer nope"},
        )
        assert bad.status_code == 401


def test_access_token_query_param_sets_cookie_for_browser(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    app = _app(
        api_key_env, fixture_brain, tmp_path, stub_provider,
        KLURIS_ACCESS_TOKEN="s3cret",
    )
    with TestClient(app) as client:
        first = client.get("/api/brain/files", params={"token": "s3cret"})
        assert first.status_code == 200
        assert client.cookies.get("kluris_access") == "s3cret"
        # Subsequent requests ride the cookie — no header, no query.
        assert client.get("/api/brain/files").status_code == 200


# --- rate limit ------------------------------------------------------------------


def test_rate_limit_caps_chat_posts_per_minute(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    app = _app(
        api_key_env, fixture_brain, tmp_path, stub_provider,
        KLURIS_RATE_LIMIT_PER_MIN="2",
    )
    with TestClient(app) as client:
        r1 = client.post("/chat", json={"message": "one"})
        r2 = client.post("/chat", json={"message": "two"})
        assert r1.status_code == 200
        assert r2.status_code == 200
        r3 = client.post("/chat", json={"message": "three"})
        assert r3.status_code == 429
        assert "rate limit" in r3.json()["error"]
        # Read-only routes are never limited.
        assert client.get("/api/brain/files").status_code == 200


def test_rate_limit_disabled_by_default(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    app = _app(api_key_env, fixture_brain, tmp_path, stub_provider)
    with TestClient(app) as client:
        for i in range(5):
            assert client.post(
                "/chat", json={"message": f"m{i}"}
            ).status_code == 200


# --- single-walk boot wiring -------------------------------------------------------


def test_boot_builds_snapshot_persistent_index_and_tagged_wake_up(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    """One boot: snapshot registered, persistent FTS db on disk under
    data_dir/cache, wake_up payload carrying per-lobe top_tags."""
    from kluris.pack.search_index import SEARCH_DB_NAME
    from kluris.pack.tools.brain import wake_up_tool
    from kluris_runtime.snapshot import drop_snapshot, get_snapshot
    from kluris_runtime.search_fts import drop_index

    app = _app(api_key_env, fixture_brain, tmp_path, stub_provider)
    try:
        assert app.state.brain_snapshot is True
        assert app.state.search_index is True
        assert app.state.search_db is True
        assert get_snapshot(fixture_brain) is not None
        assert (tmp_path / "data" / "cache" / SEARCH_DB_NAME).exists()

        payload = wake_up_tool(fixture_brain)
        knowledge = next(
            lobe for lobe in payload["lobes"] if lobe["name"] == "knowledge"
        )
        assert "sql" in knowledge["top_tags"]
    finally:
        drop_snapshot(fixture_brain)
        drop_index(fixture_brain)
        from kluris.pack.tools.brain import drop_wake_up_cache
        drop_wake_up_cache(fixture_brain)


def test_boot_snapshot_failure_degrades_not_fatal(
    api_key_env, fixture_brain, tmp_path, stub_provider, monkeypatch
):
    """A snapshot build failure must not kill boot — tools fall back to
    per-call walks and search still answers."""
    import kluris_runtime.snapshot as snap_mod

    def _boom(_path):
        raise RuntimeError("nope")

    monkeypatch.setattr(snap_mod, "build_snapshot", _boom)
    app = _app(api_key_env, fixture_brain, tmp_path, stub_provider)
    assert app.state.brain_snapshot is False
    with TestClient(app) as client:
        resp = client.get("/api/brain/search", params={"q": "jwt"})
        assert resp.status_code == 200
        assert resp.json()["total"] >= 1


# --- system-prompt lock ----------------------------------------------------------


def test_lock_system_prompt_pins_first_read(tmp_path, fixture_brain):
    """With KLURIS_LOCK_SYSTEM_PROMPT=1, editing the prompt file after the
    first read changes nothing — the boot prompt is pinned for the process."""
    from kluris.pack.system_prompt import _clear_pinned_prompts, load_prompt

    _clear_pinned_prompts()
    try:
        prompt_path = tmp_path / "config" / "system_prompt.md"
        first = load_prompt(prompt_path, brain_name="b", lock=True)
        prompt_path.write_text("HIJACKED", encoding="utf-8")
        assert load_prompt(prompt_path, brain_name="b", lock=True) == first
        # Default (no lock) keeps live-editing.
        assert load_prompt(prompt_path, brain_name="b") == "HIJACKED"
    finally:
        _clear_pinned_prompts()


def test_token_query_redirects_to_clean_url_and_sets_cookie(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    """A browser GET ?token= is 303-redirected to the token-free URL with the
    cookie set — the secret never lingers in history/Referer or repeats in the
    access log on later navigation."""
    app = _app(
        api_key_env, fixture_brain, tmp_path, stub_provider,
        KLURIS_ACCESS_TOKEN="s3cret",
    )
    with TestClient(app) as client:
        resp = client.get(
            "/api/brain/files", params={"token": "s3cret"},
            follow_redirects=False,
        )
        assert resp.status_code == 303
        assert "token=" not in resp.headers["location"]
        assert client.cookies.get("kluris_access") == "s3cret"


def test_uvicorn_access_logger_carries_redaction_filter(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    """The access logger must carry the redacting filter directly (it has its
    own handler + propagate=False), so a ?token= request line is scrubbed in
    docker logs."""
    import logging
    from kluris.pack.middleware import (
        RedactingLogFilter,
        _clear_registered_secrets,
        register_secret,
    )

    _clear_registered_secrets()
    try:
        _app(api_key_env, fixture_brain, tmp_path, stub_provider,
             KLURIS_ACCESS_TOKEN="s3cret")
        acc = logging.getLogger("uvicorn.access")
        assert any(isinstance(f, RedactingLogFilter) for f in acc.filters)
        # And the registered token is scrubbed from a synthetic access line.
        register_secret("s3cret")
        rec = logging.LogRecord(
            "uvicorn.access", logging.INFO, __file__, 0,
            '%s - "%s %s HTTP/%s" %d',
            ("1.2.3.4:0", "GET", "/?token=s3cret", "1.1", 200), None,
        )
        for f in acc.filters:
            f.filter(rec)
        assert "s3cret" not in rec.getMessage()
        # The 5-tuple structure MUST survive the filter: the access logger's
        # AccessFormatter unpacks record.args into exactly five values. Only the
        # path arg is rewritten; the status code stays an int for "%d".
        assert rec.args == ("1.2.3.4:0", "GET", "/?token=***", "1.1", 200)
    finally:
        _clear_registered_secrets()


def test_uvicorn_access_formatter_survives_redaction_filter():
    """Regression: the redaction filter must leave a uvicorn.access record in a
    state its AccessFormatter can format. Clearing record.args (the old
    behavior) made formatMessage raise "not enough values to unpack (expected
    5, got 0)" on every request — spamming the container logs on each /healthz
    probe. Run the REAL AccessFormatter over a post-filter record to prove it
    formats cleanly AND redacts the gating token."""
    import logging
    from uvicorn.logging import AccessFormatter

    from kluris.pack.middleware import (
        RedactingLogFilter,
        _clear_registered_secrets,
        register_secret,
    )

    _clear_registered_secrets()
    try:
        register_secret("s3cret")
        rec = logging.LogRecord(
            "uvicorn.access", logging.INFO, __file__, 0,
            '%s - "%s %s HTTP/%s" %d',
            ("1.2.3.4:0", "GET", "/?token=s3cret", "1.1", 200), None,
        )
        RedactingLogFilter().filter(rec)
        # format() sets record.message then calls AccessFormatter.formatMessage,
        # which unpacks record.args into a 5-tuple — the exact call that raised
        # "not enough values to unpack" in production (uvicorn/logging.py).
        line = AccessFormatter().format(rec)
        assert "s3cret" not in line
        assert '"GET /?token=*** HTTP/1.1" 200' in line
    finally:
        _clear_registered_secrets()


def test_rate_limit_does_not_count_unauthenticated_requests(
    api_key_env, fixture_brain, tmp_path, stub_provider
):
    """The access gate runs OUTSIDE the rate limiter: a 401'd request must not
    consume the per-IP budget, so a valid client is never locked out by an
    attacker's rejected spam."""
    app = _app(
        api_key_env, fixture_brain, tmp_path, stub_provider,
        KLURIS_ACCESS_TOKEN="s3cret",
        KLURIS_RATE_LIMIT_PER_MIN="2",
    )
    with TestClient(app) as client:
        # 5 unauthenticated POSTs — all 401, none counted.
        for _ in range(5):
            assert client.post("/chat", json={"message": "x"}).status_code == 401
        # The legitimate client still has its full budget.
        hdr = {"Authorization": "Bearer s3cret"}
        assert client.post("/chat", json={"message": "1"}, headers=hdr).status_code == 200
        assert client.post("/chat", json={"message": "2"}, headers=hdr).status_code == 200
        assert client.post("/chat", json={"message": "3"}, headers=hdr).status_code == 429
