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
import os
import sys
from contextlib import asynccontextmanager
from pathlib import Path
from typing import TYPE_CHECKING

from fastapi import FastAPI
from fastapi.responses import HTMLResponse, JSONResponse, PlainTextResponse

from .config import Config, ConfigError
from .middleware import install_redacting_filter, register_secret
from .readonly import assert_brain_read_only

if TYPE_CHECKING:  # pragma: no cover
    from .providers.base import LLMProvider


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
    """Instantiate the single LiteLLM provider for any auth mode.

    Imports are local so the test suite can monkeypatch this function
    without dragging litellm through every Config-only test. The LiteLLM
    process-wide globals (``drop_params`` + the TLS-aware async session)
    are set here, at boot, before the provider is built.
    """
    from .providers.litellm_provider import LiteLLMProvider, configure_litellm

    configure_litellm(config)
    return LiteLLMProvider(config)


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

    # Value-based redaction: register the ACTUAL configured secrets so a
    # gateway/IdP that echoes one in an error body (in any shape the regex
    # patterns don't recognize) still never reaches logs or the chat UI.
    for secret in (cfg.api_key, cfg.oauth_client_secret, cfg.access_token):
        if secret is not None:
            register_secret(secret.get_secret_value())

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

    # ONE boot walk powers everything. The brain is immutable inside the
    # image, so build_snapshot reads each neuron's frontmatter + body exactly
    # once and every consumer reuses it for the process lifetime:
    # search rows, the persistent FTS index, the wake_up payload, and the
    # snapshot-served tools (related / recent / files / lobe_overview).
    # Every step degrades independently — a failure means slower, never down.
    snapshot = None
    try:
        from kluris_runtime.snapshot import build_snapshot, register_snapshot

        snapshot = build_snapshot(cfg.brain_dir)
        register_snapshot(cfg.brain_dir, snapshot)
        app.state.brain_snapshot = True
    except Exception as exc:  # pragma: no cover (degrades to per-call walks)
        sys.stderr.write(
            f"kluris-pack: brain snapshot build failed "
            f"({type(exc).__name__}); tools fall back to per-call walks\n"
        )
        sys.stderr.flush()
        app.state.brain_snapshot = False

    # In-memory searchable-rows cache (fed by the snapshot's walk when it
    # succeeded). The lookup key is the resolved brain_dir inside search_fts,
    # so the agent path (which only sees Config) reaches it too —
    # app.state.search_index is just a boolean for health.
    try:
        from kluris_runtime.search_fts import build_index

        build_index(
            cfg.brain_dir,
            rows=snapshot["rows"] if snapshot is not None else None,
        )
        app.state.search_index = True
    except Exception as exc:  # pragma: no cover (degrades silently to per-query)
        sys.stderr.write(
            f"kluris-pack: search index build failed "
            f"({type(exc).__name__}); falling back to per-query search\n"
        )
        sys.stderr.flush()
        app.state.search_index = False

    # Persistent on-disk FTS index under the writable data volume: unfiltered
    # queries then skip the per-query table rebuild entirely (the dominant
    # search cost at 10k+ neurons) via per-query read-only connections.
    app.state.search_db = False
    if snapshot is not None:
        try:
            from .search_index import SEARCH_DB_NAME, build_search_db

            app.state.search_db = build_search_db(
                cfg.brain_dir,
                snapshot["rows"],
                cfg.data_dir / "cache" / SEARCH_DB_NAME,
            )
        except Exception as exc:  # pragma: no cover (in-memory path remains)
            sys.stderr.write(
                f"kluris-pack: persistent search index build failed "
                f"({type(exc).__name__}); using in-memory search\n"
            )
            sys.stderr.flush()

    # Same idea for wake_up: precompute the brain payload once so the agent's
    # first-call-of-session wake_up and every brain-tree UI load skip the
    # re-walk. A failure degrades to a per-call snapshot.
    try:
        # Relative import: in the deployed image the pack is the top-level
        # ``app`` package (uvicorn app.main:create_app), NOT ``kluris.pack`` —
        # an absolute ``kluris.pack`` import here raises ModuleNotFoundError in
        # the container, silently disabling the cache.
        from .tools.brain import build_wake_up_cache

        build_wake_up_cache(cfg.brain_dir, snapshot)
        app.state.wake_up_cached = True
    except Exception as exc:  # pragma: no cover (degrades silently to per-call)
        sys.stderr.write(
            f"kluris-pack: wake_up cache build failed "
            f"({type(exc).__name__}); falling back to per-call snapshot\n"
        )
        sys.stderr.flush()
        app.state.wake_up_cached = False

    # Order matters: Starlette runs the LAST-added middleware OUTERMOST, so
    # the access gate must be installed AFTER the rate limit to run FIRST —
    # otherwise an unauthenticated request consumes the per-IP budget before
    # it is rejected.
    _install_rate_limit(app, cfg)
    _install_access_gate(app, cfg)
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


def _install_access_gate(app: FastAPI, cfg: Config) -> None:
    """Optional shared-secret gate over the whole HTTP surface.

    When ``KLURIS_ACCESS_TOKEN`` is set, every route except ``/healthz``
    requires the token via ``Authorization: Bearer``, the ``kluris_access``
    cookie, or a one-time ``?token=`` query parameter (which sets the cookie
    so a browser link like ``http://host:8765/?token=...`` just works).
    When unset, the server stays open — with a loud boot warning, because an
    open /chat lets anyone who can reach the port spend provider tokens.
    """
    import hmac

    from fastapi.responses import RedirectResponse

    if cfg.access_token is None:
        sys.stderr.write(
            "kluris-pack: WARNING — KLURIS_ACCESS_TOKEN is not set; anyone "
            "who can reach this port can chat (and spend provider tokens) "
            "and browse the brain. Set KLURIS_ACCESS_TOKEN to require a "
            "shared secret, especially when exposing beyond localhost.\n"
        )
        sys.stderr.flush()
        return

    token = cfg.access_token.get_secret_value()

    def _matches(candidate: str | None) -> bool:
        return candidate is not None and hmac.compare_digest(candidate, token)

    @app.middleware("http")
    async def _access_gate(request, call_next):
        if request.url.path == "/healthz":
            return await call_next(request)
        auth = request.headers.get("authorization", "")
        if auth.startswith("Bearer ") and _matches(auth[len("Bearer "):]):
            return await call_next(request)
        if _matches(request.cookies.get("kluris_access")):
            return await call_next(request)
        if _matches(request.query_params.get("token")):
            secure = request.url.scheme == "https" or (
                request.headers.get("x-forwarded-proto", "").lower() == "https"
            )
            # For a browser GET, redirect to the SAME path with the token
            # stripped from the query, setting the cookie on the redirect.
            # This keeps the secret out of browser history, the Referer
            # header, and every subsequent access-log line (only the initial
            # request line carries it — and that line is redacted in logs,
            # see _install_uvicorn_access_redaction).
            if request.method in ("GET", "HEAD"):
                clean_qs = "&".join(
                    f"{k}={v}" for k, v in request.query_params.multi_items()
                    if k != "token"
                )
                target = request.url.path + (f"?{clean_qs}" if clean_qs else "")
                redirect = RedirectResponse(target, status_code=303)
                redirect.set_cookie(
                    "kluris_access", token,
                    httponly=True, samesite="lax", secure=secure,
                )
                return redirect
            response = await call_next(request)
            response.set_cookie(
                "kluris_access", token,
                httponly=True, samesite="lax", secure=secure,
            )
            return response
        return JSONResponse(
            {"ok": False, "error": "unauthorized: KLURIS_ACCESS_TOKEN required"},
            status_code=401,
        )


def _install_rate_limit(app: FastAPI, cfg: Config) -> None:
    """Per-IP fixed-window rate limit on POST /chat (0 = disabled).

    A single chat turn can fan out to many provider calls, so this is the
    server-side backstop against one client driving unbounded spend. In-
    process and approximate by design — no extra services in the container.
    """
    if cfg.rate_limit_per_min <= 0:
        return

    import time
    from collections import deque

    buckets: dict[str, deque] = {}

    @app.middleware("http")
    async def _rate_limit(request, call_next):
        if request.method == "POST" and request.url.path == "/chat":
            ip = request.client.host if request.client else "unknown"
            now = time.monotonic()
            if len(buckets) > 10000:
                # Backstop against unbounded per-IP state on a scanned
                # public port; resetting is acceptable for an approximate
                # limiter.
                buckets.clear()
            window = buckets.setdefault(ip, deque())
            while window and now - window[0] > 60.0:
                window.popleft()
            if len(window) >= cfg.rate_limit_per_min:
                return JSONResponse(
                    {"ok": False,
                     "error": "rate limit exceeded; try again in a minute"},
                    status_code=429,
                )
            window.append(now)
        return await call_next(request)


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
