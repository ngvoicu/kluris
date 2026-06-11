"""Chat routes — GET ``/``, POST ``/chat`` (SSE), POST ``/chat/new``.

Single user-facing route surface. No bearer auth, no CSRF — public
exposure is the deployer's responsibility.

Cookie scheme: ``kluris_session`` (httpOnly, sameSite=Lax). New
conversation rotates the cookie + creates a fresh session row.
"""

from __future__ import annotations

import asyncio
import json
import secrets
import uuid
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.responses import (
    HTMLResponse,
    JSONResponse,
    PlainTextResponse,
    StreamingResponse,
)
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
    search_tool,
    wake_up_tool,
)


_PACKAGE_ROOT = Path(__file__).resolve().parent.parent
_TEMPLATES_DIR = _PACKAGE_ROOT / "templates"
_STATIC_DIR = _PACKAGE_ROOT / "static"

_COOKIE_NAME = "kluris_session"


def _new_session_id() -> str:
    return uuid.uuid4().hex + secrets.token_hex(8)


# Re-sweep retention at most this often on a live container. Request-triggered
# rather than a background task: a container with no traffic has nothing
# growing, and the single uvicorn worker makes the check race-free.
_PRUNE_INTERVAL_SECONDS = 6 * 3600


def _maybe_prune(app: FastAPI, store: SessionStore) -> None:
    """Sweep sessions past the retention window — on the first request that
    touches the store (no prior timestamp) and at most once per
    :data:`_PRUNE_INTERVAL_SECONDS` thereafter.

    Opt-in (``KLURIS_SESSION_RETENTION_DAYS``); without it a long-running
    container would accumulate a full transcript per turn forever.
    """
    cfg: Config = app.state.config
    retention = getattr(cfg, "session_retention_days", 0)
    if retention <= 0:
        return
    import time

    now = time.monotonic()
    last = getattr(app.state, "_last_prune_monotonic", None)
    if last is not None and now - last < _PRUNE_INTERVAL_SECONDS:
        return
    app.state._last_prune_monotonic = now
    pruned = store.prune_old_sessions(retention)
    if pruned:
        import sys

        sys.stderr.write(
            f"kluris-pack: pruned {pruned} session(s) older than "
            f"{retention} days\n"
        )
        sys.stderr.flush()


def _store(app: FastAPI) -> SessionStore:
    cfg: Config = app.state.config
    store = getattr(app.state, "session_store", None)
    if store is None:
        store = SessionStore(cfg.data_dir / "sessions.db")
        app.state.session_store = store
    _maybe_prune(app, store)
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
        _store(app)
        # Always open a FRESH conversation on page load — the prior session
        # (if any) is preserved in the store and reachable via "Past
        # conversations", but is not resumed or replayed into the page. The
        # row is created LAZILY on the first POST /chat (see chat_post), so a
        # page load that never sends a message leaves no empty row behind.
        sid = _new_session_id()
        template = env.get_template("chat.html")
        html = template.render(
            brain_name=_brain_name(cfg),
            history=[],
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

        # Reuse the page-load cookie's session id and create its row HERE, on
        # the first message — chat_page sets the cookie but no longer writes a
        # row, so a page load that never chats leaves nothing behind. Only mint
        # a fresh id when there is no cookie at all.
        sid = request.cookies.get(_COOKIE_NAME) or _new_session_id()
        if not store.session_exists(sid):
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
                # A closed SSE connection (navigate away, refresh, LB idle
                # timeout) stops the loop between rounds — an abandoned
                # broad query must not keep burning provider tokens.
                should_cancel=request.is_disconnected,
            )
            try:
                async for frame in encode_sse(_capture_assistant(
                    agent_iter, assistant_text_parts, agent_errors,
                )):
                    yield frame
            finally:
                # Persist whatever the assistant produced — even an error or
                # a disconnect-truncated partial (Starlette aclose()s this
                # generator on disconnect, raising GeneratorExit at the
                # yield) — so a page reload shows the turn as the user last
                # saw it instead of losing it.
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

    # Tool calls below run via asyncio.to_thread: with the boot caches they
    # are usually instant, but any fallback path does blocking file I/O, and
    # the single event loop must keep multiplexing every other chat's SSE
    # stream while one request reads the brain.

    @app.get("/api/brain/tree")
    async def brain_tree():
        cfg: Config = app.state.config
        # wake_up_tool returns the discovered snapshot: lobes,
        # recent neurons, glossary, brain.md body, deprecation
        # diagnostics. The frontend builds the tree from this.
        return JSONResponse(await asyncio.to_thread(wake_up_tool, cfg.brain_dir))

    @app.get("/api/brain/files")
    async def brain_files():
        cfg: Config = app.state.config
        # Flat list of every neuron path + title + deprecated flag,
        # plus glossary.md as a sibling leaf. The frontend folds this
        # into a nested folder tree — same shape the MRI viewer uses
        # for its left-panel file explorer.
        return JSONResponse(await asyncio.to_thread(files_tool, cfg.brain_dir))

    @app.get("/api/brain/search")
    async def brain_search(q: str = "", limit: str = "20"):
        cfg: Config = app.state.config
        # Lexical search across neuron titles + bodies + paths + tags
        # AND every glossary entry (term + definition). Used by the
        # right-panel search input — type a query and the panel
        # populates with ranked result cards, MRI-style.
        #
        # ``limit`` is typed as ``str`` (rather than ``int``) so a
        # bad value falls back to the default instead of returning
        # a 422 — friendlier to the live-typing input the UI uses.
        if not q or not q.strip():
            return JSONResponse(
                {"ok": True, "query": "", "total": 0, "results": []},
            )
        try:
            limit_i = max(1, min(int(limit), 50))
        except (TypeError, ValueError):
            limit_i = 20
        return JSONResponse(
            await asyncio.to_thread(search_tool, cfg.brain_dir, q, limit=limit_i)
        )

    @app.get("/api/brain/neuron")
    async def brain_neuron(path: str):
        cfg: Config = app.state.config
        try:
            return JSONResponse(
                await asyncio.to_thread(read_neuron_tool, cfg.brain_dir, path)
            )
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
                await asyncio.to_thread(
                    lobe_overview_tool, cfg.brain_dir, lobe, budget=65536,
                ),
            )
        except SandboxError as exc:
            # Bad input (empty lobe, path escape) is a 400, not a 404 —
            # mirroring brain_neuron above.
            return JSONResponse(
                {"ok": False, "error": f"sandbox: {exc}"},
                status_code=400,
            )
        except NotFoundError as exc:
            return JSONResponse(
                {"ok": False, "error": f"not_found: {exc}"},
                status_code=404,
            )

    @app.get("/api/sessions")
    async def list_sessions(request: Request):
        # Past-conversations picker — list every session in the DB with
        # date + message count + first-user-message preview. Marks the
        # caller's current session so the UI can highlight it.
        store = _store(app)
        current_sid = request.cookies.get(_COOKIE_NAME) or ""
        sessions = store.list_sessions(limit=200)
        for s in sessions:
            s["is_current"] = (s["id"] == current_sid)
        return JSONResponse({"ok": True, "sessions": sessions})

    @app.get("/api/sessions/{sid}")
    async def get_session(sid: str):
        # Read-only view of a past conversation. Strips tool-use rows
        # so the picker shows only the user-visible turns; the export
        # endpoint preserves them for fidelity.
        store = _store(app)
        if not store.session_exists(sid):
            return JSONResponse(
                {"ok": False, "error": "not_found"}, status_code=404,
            )
        rows = store.replay(sid)
        messages = [
            {
                "role": m["role"],
                "content": m["content"],
                "created_at": m["created_at"],
            }
            for m in rows
            if m["role"] in ("user", "assistant")
        ]
        return JSONResponse({
            "ok": True,
            "session_id": sid,
            "messages": messages,
        })

    @app.get("/api/sessions/{sid}/export")
    async def export_session(sid: str, format: str = "md"):
        # Download a session as ``.md`` (default, human-readable) or
        # ``.json`` (round-trip-friendly, includes tool calls). Fires
        # a Content-Disposition: attachment so the browser saves it
        # rather than rendering inline.
        store = _store(app)
        if not store.session_exists(sid):
            return JSONResponse(
                {"ok": False, "error": "not_found"}, status_code=404,
            )
        rows = store.replay(sid)
        short = sid[:8] if len(sid) >= 8 else sid
        fmt = (format or "md").lower()
        if fmt == "json":
            body = json.dumps(
                {"session_id": sid, "messages": rows},
                indent=2,
                ensure_ascii=False,
            )
            return PlainTextResponse(
                body,
                media_type="application/json",
                headers={
                    "Content-Disposition":
                        f'attachment; filename="kluris-chat-{short}.json"',
                },
            )
        # Markdown: user/assistant turns as `## You` / `## Assistant`
        # blocks, in order. Tool rows are skipped — readers want the
        # conversation, not the trace.
        lines: list[str] = [
            f"# Conversation {short}",
            "",
            f"_session id: `{sid}`_",
            "",
        ]
        for m in rows:
            role = m["role"]
            if role not in ("user", "assistant"):
                continue
            heading = "You" if role == "user" else "Assistant"
            lines.append(f"## {heading}")
            lines.append("")
            lines.append(m["content"] or "")
            lines.append("")
        body = "\n".join(lines)
        return PlainTextResponse(
            body,
            media_type="text/markdown; charset=utf-8",
            headers={
                "Content-Disposition":
                    f'attachment; filename="kluris-chat-{short}.md"',
            },
        )

    @app.post("/chat/new")
    async def chat_new(request: Request):
        # Rotate the cookie onto a fresh session row. The previous
        # session is intentionally LEFT IN THE DATABASE so the deployer
        # can revisit it later via the past-conversations picker. Use
        # the picker's per-row delete (when added) to discard a
        # specific session, or wipe the docker volume to drop them all.
        store = _store(app)
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
