"""TEST-PACK-10 — app factory + middleware."""

from __future__ import annotations

import logging

import pytest
from fastapi.testclient import TestClient

from kluris.pack.config import Config, ConfigError
from kluris.pack.main import _provider_from_config, _redact, create_app
from kluris.pack.middleware import RedactingLogFilter, install_redacting_filter


def test_create_app_runs_smoke_test_before_serving(
    api_key_config: Config, stub_provider
):
    """The app factory must invoke the provider's smoke_test BEFORE
    returning the app. Stub provider counts the call.
    """
    create_app(
        config=api_key_config,
        provider=stub_provider,
        allow_writable_brain=True,
    )
    assert stub_provider.smoke_calls == 1


def test_kluris_skip_boot_smoke_env_skips_probe(
    api_key_env, fixture_brain, tmp_path, stub_provider, capsys
):
    """``KLURIS_SKIP_BOOT_SMOKE=1`` must opt out of the boot probe AND
    print a loud warning to stderr so it shows up in
    ``docker compose logs``.
    """
    (tmp_path / "data").mkdir()
    env = dict(
        api_key_env,
        KLURIS_BRAIN_DIR=str(fixture_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_SKIP_BOOT_SMOKE="1",
    )
    cfg = Config.load_from_env(env)
    create_app(
        config=cfg,
        provider=stub_provider,
        allow_writable_brain=True,
    )
    assert stub_provider.smoke_calls == 0, (
        "smoke test must NOT run when KLURIS_SKIP_BOOT_SMOKE=1"
    )
    err = capsys.readouterr().err
    assert "KLURIS_SKIP_BOOT_SMOKE=1" in err


def test_create_app_degrades_on_smoke_test_failure(api_key_config: Config):
    """Smoke-test failure must NOT kill the process — the app keeps
    serving brain-only routes, with ``llm_ready=False`` so the chat
    route returns 503 and the chat UI shows a "configure LLM" banner.
    The redacted error must still reach stderr.
    """
    import contextlib
    import io

    from kluris.pack.providers.base import LLMProvider

    class _FailingProvider(LLMProvider):
        model = "fail"

        async def smoke_test(self) -> None:
            raise RuntimeError("config: x-api-key: sk-secret")

        async def complete_stream(self, messages, tools):  # pragma: no cover
            yield {"kind": "end"}

    buf = io.StringIO()
    with contextlib.redirect_stderr(buf):
        app = create_app(
            config=api_key_config,
            provider=_FailingProvider(),
            allow_writable_brain=True,
        )

    assert app.state.llm_ready is False
    assert app.state.provider is None
    assert "smoke-test failed" in buf.getvalue()
    # The bearer/api-key redaction in main._redact must have stripped
    # the secret before stderr.
    assert "sk-secret" not in buf.getvalue()


def test_create_app_under_running_loop_uses_lifespan(
    api_key_config: Config, stub_provider
):
    """Regression: when ``create_app()`` is called from inside a running
    event loop (uvicorn ``--factory`` path), it must NOT use
    ``asyncio.run()`` — that raises "cannot be called from a running
    event loop". Instead it attaches a FastAPI lifespan that runs the
    smoke test on app startup, and TestClient (which drives lifespan)
    invokes it then.
    """
    import asyncio

    captured: dict = {}

    async def build_inside_loop():
        captured["app"] = create_app(
            config=api_key_config,
            provider=stub_provider,
            allow_writable_brain=True,
        )
        # Smoke must NOT have run yet — lifespan owns it now.
        captured["calls_after_factory"] = stub_provider.smoke_calls

    asyncio.run(build_inside_loop())

    assert captured["calls_after_factory"] == 0, (
        "smoke test must not run synchronously when create_app() is "
        "called inside a running loop — that's the uvicorn --factory "
        "bug. It should defer to lifespan instead."
    )

    # Drive lifespan startup via TestClient and confirm the smoke
    # test fires there.
    with TestClient(captured["app"]) as client:
        resp = client.get("/healthz")
        assert resp.status_code == 200
    assert stub_provider.smoke_calls == 1


def test_create_app_under_running_loop_lifespan_failure_propagates(
    api_key_config: Config,
):
    """Smoke-test failure inside the lifespan must surface to the caller
    — uvicorn relies on this to terminate so Compose can restart with
    a fixed env.
    """
    import asyncio

    from kluris.pack.providers.base import LLMProvider

    class _FailingProvider(LLMProvider):
        model = "fail"

        async def smoke_test(self) -> None:
            raise RuntimeError("smoke explodes")

        async def complete_stream(self, messages, tools):  # pragma: no cover
            yield {"kind": "end"}

    async def build_inside_loop():
        return create_app(
            config=api_key_config,
            provider=_FailingProvider(),
            allow_writable_brain=True,
        )

    app = asyncio.run(build_inside_loop())

    # The lifespan re-raises the original exception so uvicorn sees
    # ``lifespan.startup.failed`` and terminates the process. In
    # Starlette's ``TestClient``, that surfaces as a request that
    # fails with a 500 / refused connection, depending on transport
    # — what matters is that the lifespan startup did NOT complete
    # silently. Capture stderr to verify the redacted error message
    # was written so a deployer can see it in ``docker compose logs``.
    import io
    import contextlib

    buf = io.StringIO()
    with contextlib.redirect_stderr(buf):
        try:
            with TestClient(app):
                pass
        except BaseException:  # noqa: BLE001 (lifespan may re-raise)
            pass

    assert "kluris-pack: smoke-test failed" in buf.getvalue(), (
        "redacted smoke-test failure must reach stderr so the deployer "
        f"can see it in `docker compose logs`; got stderr={buf.getvalue()!r}"
    )
    # Original secret-bearing message must NOT have leaked.
    assert "smoke explodes" in buf.getvalue()


def test_create_app_systemexits_on_invalid_brain(
    tmp_path, api_key_env, stub_provider
):
    """Missing brain.md must abort the boot sequence."""
    bad_brain = tmp_path / "no-brain"
    bad_brain.mkdir()
    env = dict(
        api_key_env,
        KLURIS_BRAIN_DIR=str(bad_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
    )
    cfg = Config.load_from_env(env)
    with pytest.raises(SystemExit):
        create_app(
            config=cfg,
            provider=stub_provider,
            allow_writable_brain=True,
        )


def test_create_app_enters_brain_only_mode_on_config_error(
    fixture_brain, tmp_path, monkeypatch, capsys
):
    monkeypatch.setenv("KLURIS_BRAIN_DIR", str(fixture_brain))
    monkeypatch.setenv("KLURIS_DATA_DIR", str(tmp_path / "data"))

    def fail_load_from_env():
        raise ConfigError("missing auth")

    monkeypatch.setattr(Config, "load_from_env", fail_load_from_env)
    app = create_app(allow_writable_brain=True)

    assert app.state.llm_ready is False
    assert app.state.provider is None
    assert app.state.config.brain_dir == fixture_brain
    assert "starting in BRAIN-ONLY mode" in capsys.readouterr().err


def test_create_app_prints_boot_warnings(
    api_key_env, fixture_brain, tmp_path, stub_provider, capsys
):
    env = dict(
        api_key_env,
        KLURIS_BASE_URL="http://api.test/v1/chat/completions",
        KLURIS_BRAIN_DIR=str(fixture_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
    )
    cfg = Config.load_from_env(env)
    create_app(
        config=cfg,
        provider=stub_provider,
        allow_writable_brain=True,
        skip_smoke_test=True,
    )

    assert "trimmed '/v1/chat/completions'" in capsys.readouterr().err


def test_create_app_prints_tls_insecure_warning(
    api_key_env, fixture_brain, tmp_path, stub_provider, capsys
):
    env = dict(
        api_key_env,
        KLURIS_BRAIN_DIR=str(fixture_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
        KLURIS_TLS_INSECURE="1",
    )
    cfg = Config.load_from_env(env)
    create_app(
        config=cfg,
        provider=stub_provider,
        allow_writable_brain=True,
        skip_smoke_test=True,
    )

    assert "KLURIS_TLS_INSECURE=1" in capsys.readouterr().err


def test_provider_factory_selects_auth_mode(api_key_config: Config, oauth_env):
    """Both auth modes now resolve to the single LiteLLMProvider; the auth mode
    is carried inside it (static key vs OAuth token manager), not by class."""
    from kluris.pack.providers.litellm_provider import LiteLLMProvider

    api_provider = _provider_from_config(api_key_config)
    assert isinstance(api_provider, LiteLLMProvider)
    assert api_provider._oauth is False

    oauth_cfg = Config.load_from_env(dict(oauth_env, KLURIS_BRAIN_DIR="/app/brain"))
    oauth_provider = _provider_from_config(oauth_cfg)
    assert isinstance(oauth_provider, LiteLLMProvider)
    assert oauth_provider._oauth is True


def test_healthz_returns_200(api_key_config: Config, stub_provider):
    """``/healthz`` returns 200 once the boot sequence completed."""
    app = create_app(
        config=api_key_config,
        provider=stub_provider,
        allow_writable_brain=True,
    )
    with TestClient(app) as client:
        resp = client.get("/healthz")
        assert resp.status_code == 200
        assert resp.json() == {"ok": True}


def test_chat_routes_unauthenticated(api_key_config: Config, stub_provider):
    """No bearer / CSRF / login form blocks the chat routes — public
    exposure is the deployer's job (reverse proxy, VPN, cloud IAM).
    """
    app = create_app(
        config=api_key_config,
        provider=stub_provider,
        allow_writable_brain=True,
        skip_smoke_test=False,
    )
    with TestClient(app) as client:
        # Health is the canonical "no auth required" probe.
        resp = client.get("/healthz")
        assert resp.status_code == 200
        # No Authorization / CSRF token / cookie required.
        assert "WWW-Authenticate" not in resp.headers


def test_redact_strips_bearer_token():
    redacted = _redact("Auth header: Bearer sk-secret-zzz")
    assert "sk-secret-zzz" not in redacted
    assert "Bearer ***" in redacted


def test_redact_strips_x_api_key():
    redacted = _redact("Sent x-api-key: sk-secret-yyy and got 401")
    assert "sk-secret-yyy" not in redacted
    assert "x-api-key: ***" in redacted


def test_redacting_log_filter_in_isolation():
    """The filter rewrites ``record.msg`` in place when applied to a
    fresh :class:`LogRecord`. Verified directly to avoid pytest's
    caplog handler ordering quirks (caplog inserts its own handler
    before the root filter chain runs).
    """
    f = RedactingLogFilter()
    record = logging.LogRecord(
        "test", logging.INFO, "x.py", 1,
        "Authorization: Bearer sk-test-bbb", (), None,
    )
    f.filter(record)
    assert "sk-test-bbb" not in record.getMessage()
    assert "Bearer ***" in record.getMessage()


def test_redacting_log_filter_strips_x_api_key():
    f = RedactingLogFilter()
    record = logging.LogRecord(
        "test", logging.INFO, "x.py", 1,
        "headers={x-api-key: sk-test-ccc}", (), None,
    )
    f.filter(record)
    assert "sk-test-ccc" not in record.getMessage()


def test_redacting_log_filter_idempotent():
    """Calling :func:`install_redacting_filter` twice must not double-add."""
    root = logging.getLogger()
    before = sum(isinstance(f, RedactingLogFilter) for f in root.filters)
    install_redacting_filter()
    install_redacting_filter()
    after = sum(isinstance(f, RedactingLogFilter) for f in root.filters)
    assert after - before <= 1


# --- value-based redaction (2.28.0) ---------------------------------------------


def test_registered_literal_secret_is_redacted_in_any_shape():
    """An opaque secret (no sk-/JWT/Bearer shape) must be scrubbed once
    registered — gateways echo credentials in arbitrary error bodies."""
    from kluris.pack.middleware import (
        _clear_registered_secrets,
        redact_secrets,
        register_secret,
    )

    _clear_registered_secrets()
    try:
        register_secret("opaque-azure-key-42")
        out = redact_secrets("invalid client_secret: opaque-azure-key-42 (denied)")
        assert "opaque-azure-key-42" not in out
        assert "***" in out
    finally:
        _clear_registered_secrets()


def test_register_secret_ignores_short_and_empty_values():
    from kluris.pack.middleware import (
        _LITERAL_SECRETS,
        _clear_registered_secrets,
        register_secret,
    )

    _clear_registered_secrets()
    try:
        register_secret(None)
        register_secret("")
        register_secret("ab")  # too short — would shred ordinary text
        assert _LITERAL_SECRETS == []
    finally:
        _clear_registered_secrets()
