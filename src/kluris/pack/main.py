"""FastAPI app factory for the kluris pack chat server.

Boot sequence:

1. Read config from environment via :class:`Config.load_from_env`.
   On :class:`ConfigError` the app starts in BRAIN-ONLY mode with a
   minimal config: chat is disabled, the brain explorer routes still
   serve. Deployer fixes ``.env`` and restarts.
2. Verify the bundled brain is read-only (only inside the Docker
   image; tests pass ``allow_writable=True`` via the factory). This
   step is a hard fail — without a brain there's nothing to serve.
3. Run a tool-capability smoke-test against the configured LLM
   endpoint with a 5/15/5/5 ``httpx.Timeout``. A failure flips
   ``app.state.llm_ready`` to False (chat disabled) but the process
   keeps serving brain-only routes.
4. Mount routes (``/healthz``, ``/`` chat UI, ``/chat``,
   ``/api/brain/*``).

Smoke-test scheduling. ``create_app()`` may be called two ways:

- **Sync test path** (``create_app(...)`` from a non-async test): no
  event loop is running, so the smoke test runs synchronously via
  ``asyncio.run()`` inside the factory.
- **uvicorn ``--factory`` path** (production): uvicorn invokes the
  factory from inside its own running event loop, which makes
  ``asyncio.run()`` illegal. We detect the running loop and instead
  attach a FastAPI lifespan that runs the smoke test on app startup,
  inside the same loop. A failure flips ``llm_ready`` rather than
  killing the process.
"""

from __future__ import annotations

import asyncio
import logging
import os
import sys
from contextlib import asynccontextmanager
from pathlib import Path
from typing import TYPE_CHECKING

from fastapi import FastAPI
from fastapi.responses import HTMLResponse, JSONResponse, PlainTextResponse

from .config import Config, ConfigError
from .middleware import install_redacting_filter
from .readonly import assert_brain_read_only

if TYPE_CHECKING:  # pragma: no cover
    from .providers.base import LLMProvider

logger = logging.getLogger("kluris.pack")


def _minimal_config_from_env() -> Config:
    """Config with only the fields the brain-only routes need.

    Used as a fallback when LLM auth isn't configured. The brain
    explorer routes need ``brain_dir``; the session store needs
    ``data_dir``. Every LLM-related field stays at its model default
    (``None`` / ``""``), and ``app.state.llm_ready`` is set to False
    so the chat route refuses requests.
    """
    env = dict(os.environ)
    return Config(
        brain_dir=Path(env.get("KLURIS_BRAIN_DIR", "/app/brain")),
        data_dir=Path(env.get("KLURIS_DATA_DIR", "/data")),
    )


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


def _loop_is_running() -> bool:
    """Return True iff the calling thread already has a running loop."""
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return False
    return True


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

    llm_error: str | None = None

    try:
        cfg = config or Config.load_from_env()
    except ConfigError as exc:
        sys.stderr.write(f"kluris-pack: {exc}\n")
        sys.stderr.write(
            "kluris-pack: starting in BRAIN-ONLY mode — chat is "
            "disabled until LLM auth is configured. Brain explorer "
            "remains available.\n"
        )
        sys.stderr.flush()
        llm_error = str(exc)
        cfg = _minimal_config_from_env()

    try:
        assert_brain_read_only(cfg.brain_dir, allow_writable=allow_writable_brain)
    except RuntimeError as exc:
        sys.stderr.write(f"kluris-pack: {exc}\n")
        raise SystemExit(3) from exc

    for warning in cfg.boot_warnings:
        sys.stderr.write(f"kluris-pack: {warning}\n")
    if cfg.boot_warnings:
        sys.stderr.flush()

    if cfg.tls_insecure:
        sys.stderr.write(
            "kluris-pack: WARNING — KLURIS_TLS_INSECURE=1 is set; LLM "
            "endpoint TLS certificates are NOT being verified. Use "
            "KLURIS_CA_BUNDLE to trust a private root CA instead "
            "wherever possible.\n"
        )
        sys.stderr.flush()

    if cfg.skip_boot_smoke and not skip_smoke_test:
        sys.stderr.write(
            "kluris-pack: WARNING — KLURIS_SKIP_BOOT_SMOKE=1 is set; "
            "boot tool-capability smoke-test is being skipped. The "
            "first chat request will be the first time the LLM "
            "endpoint is exercised — misconfiguration won't surface "
            "until then.\n"
        )
        sys.stderr.flush()
        skip_smoke_test = True

    prov: "LLMProvider | None" = None
    if llm_error is None:
        try:
            prov = provider or _provider_from_config(cfg)
        except Exception as exc:  # pragma: no cover (provider ctors are simple)
            sys.stderr.write(
                f"kluris-pack: provider construction failed "
                f"({type(exc).__name__}): {_redact(str(exc))}; "
                f"chat disabled, brain explorer remains available.\n"
            )
            sys.stderr.flush()
            llm_error = f"{type(exc).__name__}: {_redact(str(exc))}"

    lifespan = None
    if prov is not None and not skip_smoke_test:
        if _loop_is_running():
            # uvicorn --factory: defer to lifespan so smoke runs inside
            # the existing event loop. A failure no longer kills the
            # process — we mark ``llm_ready=False`` and keep serving
            # the brain-only routes. The deployer fixes ``.env`` and
            # restarts.
            @asynccontextmanager
            async def _lifespan(_app: FastAPI):
                try:
                    await prov.smoke_test()
                except Exception as exc:
                    err = f"{type(exc).__name__}: {_redact(str(exc))}"
                    sys.stderr.write(
                        f"kluris-pack: smoke-test failed ({err}); "
                        f"chat disabled, brain explorer remains "
                        f"available.\n"
                    )
                    sys.stderr.flush()
                    _app.state.llm_ready = False
                    _app.state.llm_error = err
                    _app.state.provider = None
                yield

            lifespan = _lifespan
        else:
            try:
                asyncio.run(prov.smoke_test())
            except Exception as exc:
                err = f"{type(exc).__name__}: {_redact(str(exc))}"
                sys.stderr.write(
                    f"kluris-pack: smoke-test failed ({err}); "
                    f"chat disabled, brain explorer remains available.\n"
                )
                sys.stderr.flush()
                llm_error = err
                prov = None

    app = FastAPI(
        title="kluris-pack",
        openapi_url=None,
        docs_url=None,
        lifespan=lifespan,
    )
    app.state.config = cfg
    app.state.provider = prov
    app.state.llm_ready = prov is not None and llm_error is None
    app.state.llm_error = llm_error
    _mount_minimal_routes(app)
    # uvicorn binds to 0.0.0.0:8765 inside the container (required for
    # docker port mapping), but compose maps host->127.0.0.1:8765 only.
    # uvicorn's startup banner shows "0.0.0.0:8765" which some browsers
    # can't resolve on Windows — print the host-reachable URL instead.
    sys.stderr.write(
        "kluris-pack: open http://localhost:8765 in your browser\n"
    )
    sys.stderr.flush()
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
