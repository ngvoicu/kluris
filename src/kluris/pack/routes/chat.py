"""Chat routes — GET ``/``, POST ``/chat`` (SSE), POST ``/chat/new``.

Single user-facing route surface. No bearer auth, no CSRF — public
exposure is the deployer's responsibility.

Cookie scheme: ``kluris_session`` (httpOnly, sameSite=Lax). New
conversation rotates the cookie + creates a fresh session row.
"""

from __future__ import annotations

import json
import secrets
import uuid
from pathlib import Path
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, JSONResponse, StreamingResponse
from fastapi.staticfiles import StaticFiles
from jinja2 import Environment, FileSystemLoader, select_autoescape

from ..agent import run_agent
from ..config import Config
from ..history import SessionStore
from ..providers.base import LLMProvider
from ..streaming import encode_sse
from ..tools.brain import (
    NotFoundError,
    SandboxError,
    files_tool,
    lobe_overview_tool,
    read_neuron_tool,
    wake_up_tool,
)


_PACKAGE_ROOT = Path(__file__).resolve().parent.parent
_TEMPLATES_DIR = _PACKAGE_ROOT / "templates"
_STATIC_DIR = _PACKAGE_ROOT / "static"

_COOKIE_NAME = "kluris_session"


def _new_session_id() -> str:
    return uuid.uuid4().hex + secrets.token_hex(8)


def _store(app: FastAPI) -> SessionStore:
    cfg: Config = app.state.config
    store = getattr(app.state, "session_store", None)
    if store is None:
        store = SessionStore(cfg.data_dir / "sessions.db")
        app.state.session_store = store
    return store


def _brain_name(cfg: Config) -> str:
    """Return the brain's display name from its ``brain.md`` H1.

    Falls back to the brain directory name when no H1 is present.
    Read once on demand; the chat UI re-fetches per request only via
    the running app, so this is cheap.
    """
    brain_md = cfg.brain_dir / "brain.md"
    if brain_md.exists():
        try:
            for line in brain_md.read_text(encoding="utf-8").splitlines():
                if line.startswith("# "):
                    return line[2:].strip()
        except OSError:
            pass
    return cfg.brain_dir.name


def attach_chat_routes(app: FastAPI) -> None:
    """Mount the chat UI + chat SSE routes onto ``app``.

    Idempotent — safe to call multiple times during testing.
    """
    if getattr(app.state, "_chat_attached", False):
        return
    app.state._chat_attached = True

    if _STATIC_DIR.exists():
        app.mount("/static", StaticFiles(directory=str(_STATIC_DIR)), name="static")

    env = Environment(
        loader=FileSystemLoader(str(_TEMPLATES_DIR)),
        autoescape=select_autoescape(),
    )

    @app.get("/", response_class=HTMLResponse)
    async def chat_page(request: Request):
        cfg: Config = app.state.config
        store = _store(app)
        sid = request.cookies.get(_COOKIE_NAME)
        if not sid or not store.session_exists(sid):
            sid = _new_session_id()
            store.new_session(session_id=sid)
        history = store.replay(sid)
        template = env.get_template("chat.html")
        html = template.render(
            brain_name=_brain_name(cfg),
            history=history,
            llm_ready=getattr(app.state, "llm_ready", False),
            llm_error=getattr(app.state, "llm_error", None),
        )
        resp = HTMLResponse(html)
        resp.set_cookie(
            _COOKIE_NAME, sid,
            httponly=True, samesite="lax",
        )
        return resp

    @app.post("/chat")
    async def chat_post(request: Request):
        if not getattr(app.state, "llm_ready", False):
            return JSONResponse(
                {
                    "ok": False,
                    "error": (
                        "LLM is not configured. Set KLURIS_API_KEY "
                        "(or the OAuth vars) in .env and restart the "
                        "container. The brain explorer in the sidebar "
                        "remains available."
                    ),
                },
                status_code=503,
            )
        cfg: Config = app.state.config
        provider: LLMProvider = app.state.provider
        store = _store(app)

        sid = request.cookies.get(_COOKIE_NAME)
        if not sid or not store.session_exists(sid):
            sid = _new_session_id()
            store.new_session(session_id=sid)

        try:
            payload = await request.json()
        except (ValueError, json.JSONDecodeError):
            payload = {}
        user_message = str(payload.get("message", "")).strip()
        if not user_message:
            return JSONResponse(
                {"ok": False, "error": "message must be a non-empty string"},
                status_code=400,
            )

        history = store.replay(sid)
        # Persist the user's turn before streaming so a refresh-mid-
        # answer doesn't lose the prompt.
        store.append_message(sid, "user", user_message)

        async def event_stream():
            assistant_text_parts: list[str] = []
            agent_errors: list[str] = []
            agent_iter = run_agent(
                config=cfg,
                provider=provider,
                history=[
                    {"role": h["role"], "content": h["content"]}
                    for h in history
                    if h["role"] in {"user", "assistant"}
                ],
                user_message=user_message,
                brain_name=_brain_name(cfg),
            )
            async for frame in encode_sse(_capture_assistant(
                agent_iter, assistant_text_parts, agent_errors,
            )):
                yield frame
            # Persist whatever the assistant produced — even an error
            # — so a page reload shows the same turn the user just saw.
            assistant_text = "".join(assistant_text_parts).strip()
            if not assistant_text and agent_errors:
                assistant_text = "\n\n".join(
                    f"[error: {msg}]" for msg in agent_errors
                )
            if assistant_text:
                store.append_message(sid, "assistant", assistant_text)

        resp = StreamingResponse(event_stream(), media_type="text/event-stream")
        resp.set_cookie(
            _COOKIE_NAME, sid,
            httponly=True, samesite="lax",
        )
        return resp

    # --- Brain explorer (read-only) -----------------------------------
    #
    # These endpoints back the left-sidebar brain tree and the
    # click-to-expand neuron modal. They reuse the same tool
    # dispatchers the LLM agent uses, so the UI sees exactly the
    # same view of the brain the agent does.

    @app.get("/api/brain/tree")
    async def brain_tree():
        cfg: Config = app.state.config
        # wake_up_tool returns the discovered snapshot: lobes,
        # recent neurons, glossary, brain.md body, deprecation
        # diagnostics. The frontend builds the tree from this.
        return JSONResponse(wake_up_tool(cfg.brain_dir))

    @app.get("/api/brain/files")
    async def brain_files():
        cfg: Config = app.state.config
        # Flat list of every neuron path + title + deprecated flag,
        # plus glossary.md as a sibling leaf. The frontend folds this
        # into a nested folder tree — same shape the MRI viewer uses
        # for its left-panel file explorer.
        return JSONResponse(files_tool(cfg.brain_dir))

    @app.get("/api/brain/neuron")
    async def brain_neuron(path: str):
        cfg: Config = app.state.config
        try:
            return JSONResponse(read_neuron_tool(cfg.brain_dir, path))
        except SandboxError as exc:
            return JSONResponse(
                {"ok": False, "error": f"sandbox: {exc}"},
                status_code=400,
            )
        except NotFoundError as exc:
            return JSONResponse(
                {"ok": False, "error": f"not_found: {exc}"},
                status_code=404,
            )

    @app.get("/api/brain/lobe")
    async def brain_lobe(lobe: str):
        cfg: Config = app.state.config
        try:
            # Use a generous budget here — the UI is a human reader,
            # not an LLM context window. 64 KB lets the lobe map_body
            # render verbatim without truncation hints.
            return JSONResponse(
                lobe_overview_tool(cfg.brain_dir, lobe, budget=65536),
            )
        except NotFoundError as exc:
            return JSONResponse(
                {"ok": False, "error": f"not_found: {exc}"},
                status_code=404,
            )

    @app.post("/chat/new")
    async def chat_new(request: Request):
        store = _store(app)
        old_sid = request.cookies.get(_COOKIE_NAME)
        if old_sid and store.session_exists(old_sid):
            store.delete_session(old_sid)
        new_sid = _new_session_id()
        store.new_session(session_id=new_sid)
        resp = JSONResponse({"ok": True, "session_id": new_sid})
        resp.set_cookie(
            _COOKIE_NAME, new_sid,
            httponly=True, samesite="lax",
        )
        return resp


async def _capture_assistant(
    agent_iter,
    sink: list[str],
    errors: list[str] | None = None,
):
    """Pass-through that records assistant text tokens AND error
    messages for persistence so a reload shows the same turn.
    """
    async for ev in agent_iter:
        kind = ev.get("kind")
        if kind == "token":
            sink.append(ev.get("text", ""))
        elif kind == "error" and errors is not None:
            msg = ev.get("message")
            if isinstance(msg, str) and msg:
                errors.append(msg)
        yield ev
