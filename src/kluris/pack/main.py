"""FastAPI app factory for the kluris pack chat server.

Boot sequence:

1. Read config from environment via :class:`Config.load_from_env`.
2. Verify the bundled brain is read-only (only inside the Docker
   image; tests pass ``allow_writable=True`` via the factory).
3. Run a tool-capability smoke-test against the configured LLM
   endpoint with a 5/15/5/5 ``httpx.Timeout``.
4. Mount routes (``/healthz``, ``/`` chat UI, ``/chat``).

Any failure in steps 1–3 calls :func:`sys.exit` with a redacted error
message on stderr — Compose's ``restart: unless-stopped`` will keep
cycling until the deployer fixes the env or rebuilds the image.
"""

from __future__ import annotations

import asyncio
import logging
import sys
from typing import TYPE_CHECKING

from fastapi import FastAPI
from fastapi.responses import HTMLResponse, JSONResponse, PlainTextResponse

from .config import Config, ConfigError
from .middleware import install_redacting_filter
from .readonly import assert_brain_read_only

if TYPE_CHECKING:  # pragma: no cover
    from .providers.base import LLMProvider

logger = logging.getLogger("kluris.pack")


def _provider_from_config(config: Config) -> "LLMProvider":
    """Instantiate the right provider for ``config``'s auth mode.

    Imports are local so the test suite can monkeypatch this function
    without dragging the provider modules through every Config-only
    test.
    """
    if config.auth_mode == "oauth":
        from .providers.oauth import OAuthProvider

        return OAuthProvider(config)
    from .providers.apikey import APIKeyProvider

    return APIKeyProvider(config)


def create_app(
    *,
    config: Config | None = None,
    provider: "LLMProvider | None" = None,
    allow_writable_brain: bool = False,
    skip_smoke_test: bool = False,
) -> FastAPI:
    """Build and return the chat server's :class:`FastAPI` app.

    Parameters are all test-time conveniences; production code only
    calls ``create_app()`` with no args.

    - ``config``: pre-built config; defaults to ``Config.load_from_env``.
    - ``provider``: pre-built provider; defaults to the one matching
      ``config.auth_mode``.
    - ``allow_writable_brain``: skip the brain-writability probe (tests
      operate on a writable ``tmp_path`` brain).
    - ``skip_smoke_test``: skip the boot tool-capability smoke-test
      (Config-only / route-shape tests don't need a live mock).
    """
    install_redacting_filter()

    try:
        cfg = config or Config.load_from_env()
    except ConfigError as exc:
        sys.stderr.write(f"kluris-pack: {exc}\n")
        raise SystemExit(2) from exc

    try:
        assert_brain_read_only(cfg.brain_dir, allow_writable=allow_writable_brain)
    except RuntimeError as exc:
        sys.stderr.write(f"kluris-pack: {exc}\n")
        raise SystemExit(3) from exc

    prov = provider or _provider_from_config(cfg)

    if not skip_smoke_test:
        try:
            asyncio.run(prov.smoke_test())
        except Exception as exc:
            # Redact: only the exception type + message, never the
            # secret-bearing config repr or HTTP body.
            sys.stderr.write(
                f"kluris-pack: smoke-test failed ({type(exc).__name__}): "
                f"{_redact(str(exc))}\n"
            )
            raise SystemExit(4) from exc

    app = FastAPI(title="kluris-pack", openapi_url=None, docs_url=None)
    app.state.config = cfg
    app.state.provider = prov
    _mount_minimal_routes(app)
    return app


def _redact(text: str) -> str:
    """Last-line redaction of obvious credentials in error messages.

    The provider modules already redact at their boundary; this is
    belt-and-suspenders for any string that slips through.
    """
    import re

    text = re.sub(r"Bearer\s+\S+", "Bearer ***", text, flags=re.IGNORECASE)
    text = re.sub(r"x-api-key:\s*\S+", "x-api-key: ***", text, flags=re.IGNORECASE)
    return text


def _mount_minimal_routes(app: FastAPI) -> None:
    """Attach the tiny route surface every test/contract assumes.

    Real ``/`` chat UI and POST ``/chat`` are wired up in
    :mod:`kluris.pack.routes.chat`; that import is local so config-only
    tests don't pull in Jinja2 templates.
    """

    @app.get("/healthz")
    async def healthz() -> JSONResponse:
        return JSONResponse({"ok": True})

    try:
        from .routes.chat import attach_chat_routes

        attach_chat_routes(app)
    except ImportError:  # pragma: no cover (chat routes optional in some test paths)
        @app.get("/", response_class=HTMLResponse)
        async def _placeholder() -> PlainTextResponse:
            return PlainTextResponse("kluris-pack chat UI not yet mounted")
