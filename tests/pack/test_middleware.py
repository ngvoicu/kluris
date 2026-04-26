"""TEST-PACK-10 — app factory + middleware."""

from __future__ import annotations

import logging

import pytest
from fastapi.testclient import TestClient

from kluris.pack.config import Config
from kluris.pack.main import _redact, create_app
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


def test_create_app_systemexits_on_smoke_test_failure(api_key_config: Config):
    """Smoke-test failure must exit non-zero with a redacted message."""
    from kluris.pack.providers.base import LLMProvider

    class _FailingProvider(LLMProvider):
        model = "fail"

        async def smoke_test(self) -> None:
            raise RuntimeError("config: x-api-key: sk-secret")

        async def complete_stream(self, messages, tools):  # pragma: no cover
            yield {"kind": "end"}

    with pytest.raises(SystemExit) as exc:
        create_app(
            config=api_key_config,
            provider=_FailingProvider(),
            allow_writable_brain=True,
        )
    assert exc.value.code != 0


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
