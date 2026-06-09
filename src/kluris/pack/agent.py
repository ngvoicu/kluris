"""Provider-agnostic tool-calling loop.

The loop:

1. Send the system prompt + conversation history to the provider.
2. Receive a stream of token / tool_use / usage / end events.
3. For each ``tool_use`` event, dispatch to
   :data:`kluris.pack.tools.brain.TOOLS`, append the tool result to the
   conversation, and re-enter the loop.
4. Stop when the provider emits ``end`` without a pending tool call,
   or when ``MAX_AGENT_ROUNDS`` rounds have elapsed.

Streaming output is forwarded to the SSE layer via the ``yield``
contract — every event the provider emits is yielded back, plus
synthetic ``tool_result`` events the loop generates after dispatching
each tool call. The chat route in
:mod:`kluris.pack.routes.chat` owns the SSE encoding.
"""

from __future__ import annotations

import json
import sys
from typing import Any, AsyncIterator, Callable

from .config import Config
from .providers.base import (
    AuthError,
    ContextLimitError,
    LLMProvider,
    RequestError,
)
from .system_prompt import load_prompt
from .tools.brain import (
    NotFoundError,
    SandboxError,
    TOOLS,
)
from .tools.schemas import openai_schemas


# Per-call test-only trace hook. Production code never reads this; the
# scripted-provider eval harness sets a callable that records every
# tool call/result for assertions.
ToolTraceHook = Callable[[dict[str, Any]], None]


def _system_prompt(config: Config, brain_name: str) -> str:
    prompt_path = config.data_dir / "config" / "system_prompt.md"
    return load_prompt(prompt_path, brain_name=brain_name)


def _tool_schemas(config: Config) -> list[dict[str, Any]]:
    # LiteLLM takes OpenAI-format tool schemas for EVERY provider (translating to
    # the Anthropic tool shape internally), so the pack emits one shape for all.
    return openai_schemas(max_multi_read=config.max_multi_read_paths)


def _estimate_tokens(text: str) -> int:
    """Rough token estimate. The pack ships no tokenizer; ~4 chars/token is a
    standard heuristic for English prose. Rounds up so we under-fill rather
    than overflow."""
    return len(text) // 4 + 1


def _trim_history(
    history: list[dict[str, Any]], max_tokens: int
) -> list[dict[str, Any]]:
    """Sliding window over conversation history.

    Keeps the most RECENT turns whose estimated tokens fit ``max_tokens`` and
    drops the oldest, so a long conversation never overflows the model's
    context window — it just forgets its oldest turns (facts are re-fetched
    from the brain each turn anyway). ``max_tokens <= 0`` disables trimming.

    Always keeps at least the single most recent message (even if it alone
    exceeds the budget) so a turn is never sent with empty history; the
    provider's ContextLimitError stays the backstop for that edge case. The
    ~4-tokens-per-message overhead approximates role/formatting tokens.
    """
    if max_tokens <= 0:
        return history
    kept: list[dict[str, Any]] = []
    total = 0
    for msg in reversed(history):
        cost = _estimate_tokens(str(msg.get("content", ""))) + 4
        if kept and total + cost > max_tokens:
            break
        total += cost
        kept.append(msg)
    kept.reverse()
    return kept


def _estimate_messages_tokens(messages: list[dict[str, Any]]) -> int:
    """Rough token estimate of a full request payload — content plus any
    ``tool_calls`` block, plus ~4 tokens/message of role/formatting overhead.
    Used to bound the in-turn transcript, not to bill anyone.
    """
    total = 0
    for msg in messages:
        total += _estimate_tokens(str(msg.get("content", "")))
        if msg.get("tool_calls"):
            total += _estimate_tokens(str(msg["tool_calls"]))
        total += 4
    return total


# Stub that replaces an elided tool-result body. The tool_call_id and message
# position are preserved (we only swap ``content``), so the assistant↔tool
# pairing OpenAI requires stays intact — only the payload shrinks.
_COMPACTED_TOOL_RESULT = (
    '{"compacted": true, "note": "earlier tool result elided to fit the turn '
    'budget; call the tool again if you still need it"}'
)


def _compact_tool_results(
    messages: list[dict[str, Any]], max_tokens: int
) -> list[dict[str, Any]]:
    """Bound a single turn's request size by eliding the OLDEST tool-result
    payloads until the estimate fits ``max_tokens``.

    Only the ``content`` of ``role: tool`` messages is replaced — never the
    system prompt, the user turn, or an assistant ``tool_calls`` block — so the
    message structure (and tool_call_id pairing) is preserved. The most recent
    tool result is always kept verbatim so the model still has fresh evidence to
    answer from. ``max_tokens <= 0`` disables compaction (the unbounded legacy
    behaviour). Mutates ``messages`` in place and returns it.
    """
    if max_tokens <= 0 or _estimate_messages_tokens(messages) <= max_tokens:
        return messages
    tool_idxs = [i for i, m in enumerate(messages) if m.get("role") == "tool"]
    # Keep the last tool result intact; compact older ones oldest-first.
    for i in tool_idxs[:-1]:
        if _estimate_messages_tokens(messages) <= max_tokens:
            break
        if messages[i].get("content") != _COMPACTED_TOOL_RESULT:
            messages[i] = {**messages[i], "content": _COMPACTED_TOOL_RESULT}
    return messages


# Injected as a user turn for the tools-disabled synthesis pass (see
# ``_stream_final_synthesis``) — used instead of throwing the turn away when the
# round budget is hit or a round comes back empty after evidence was gathered.
_SYNTHESIS_NUDGE = (
    "You have gathered enough information from the tools above. Answer the "
    "user's question now using ONLY that evidence. If the brain does not "
    "contain what they asked for, say so plainly and list what is missing. "
    "Do not call any tools."
)


def _provider_error_event(exc: Exception) -> dict[str, Any]:
    """Normalize a provider exception to an error event, classified exactly as
    the main loop does — so the synthesis fallback can't silently downgrade a
    fatal ``AuthError`` / ``RequestError`` to "recoverable" or mislabel a context
    overflow. ``ContextLimitError`` is a ``RequestError`` subclass, so it must be
    checked first.
    """
    if isinstance(exc, ContextLimitError):
        return {
            "kind": "error",
            "message": (
                "Conversation has grown beyond the model's context window. "
                "Click 'New conversation' to start fresh."
            ),
            "recoverable": True,
        }
    detail = str(exc)
    message = f"Provider error: {detail}"
    if "reasoning_effort" in detail.lower():
        message += (
            " — check KLURIS_REASONING_EFFORT: this model may not accept "
            "that effort value. Try low / medium / high, or unset it."
        )
    return {"kind": "error", "message": message, "recoverable": False}


async def _stream_final_synthesis(
    provider: LLMProvider, messages: list[dict[str, Any]]
) -> AsyncIterator[dict[str, Any]]:
    """One tools-disabled completion that forces a final answer from the
    evidence already gathered this turn.

    Yields the normalized ``token`` / ``usage`` events, then a terminal
    ``{"kind": "_synth", "produced", "completed", "error"}`` sentinel the caller
    consumes (never forwards):

    - ``produced``  — at least one non-empty token streamed.
    - ``completed`` — the stream finished without raising.
    - ``error``     — a classified error event if a provider error was raised
      BEFORE any token (else ``None``). A drop AFTER tokens leaves
      ``produced=True, completed=False`` so the caller can flag the partial as
      incomplete rather than render it as a finished answer.
    """
    produced = False
    completed = False
    error_event: dict[str, Any] | None = None
    try:
        async for ev in provider.complete_stream(messages, []):
            kind = ev.get("kind")
            if kind == "token":
                if ev.get("text"):
                    produced = True
                yield ev
            elif kind == "usage":
                yield ev
            # tool_use / end from the synthesis call are intentionally ignored
        completed = True
    except (ContextLimitError, AuthError, RequestError) as exc:
        # Surface a real provider failure only when nothing streamed yet; a drop
        # mid-answer is reported via produced=True, completed=False instead.
        if not produced:
            error_event = _provider_error_event(exc)
    except Exception:
        # Unknown failure → benign non-completion; the caller uses its generic
        # recoverable fallback (or the incomplete-partial note).
        pass
    yield {
        "kind": "_synth",
        "produced": produced,
        "completed": completed,
        "error": error_event,
    }


async def _run_synthesis_fallback(
    provider: LLMProvider,
    messages: list[dict[str, Any]],
    config: Config,
    empty_error: dict[str, Any],
) -> AsyncIterator[dict[str, Any]]:
    """Run one tools-disabled synthesis and yield the correct terminal events.

    Outcomes, in precedence order:

    - a clean answer streamed           → just ``end``;
    - a drop mid-answer (partial text)  → an "incomplete" note, then ``end``;
    - a fatal provider error, no tokens → the classified error, then ``end``;
    - a clean empty completion          → ``empty_error`` (the caller's generic
      recoverable message), then ``end``.

    Always terminates the turn with ``{"kind": "end"}``.
    """
    messages = _compact_tool_results(messages, config.max_turn_tokens)
    messages.append({"role": "user", "content": _SYNTHESIS_NUDGE})
    produced = False
    completed = False
    error_event: dict[str, Any] | None = None
    async for ev in _stream_final_synthesis(provider, messages):
        if ev.get("kind") == "_synth":
            produced = bool(ev.get("produced"))
            completed = bool(ev.get("completed"))
            error_event = ev.get("error")
        else:
            yield ev
    if produced and not completed:
        yield {
            "kind": "error",
            "message": (
                "The answer above may be incomplete — the model's connection "
                "dropped mid-response. Try asking again."
            ),
            "recoverable": True,
        }
    elif not produced and error_event is not None:
        yield error_event
    elif not produced:
        yield empty_error
    yield {"kind": "end"}


def _dispatch_tool(
    config: Config,
    name: str,
    args: dict[str, Any],
) -> dict[str, Any]:
    """Run one tool dispatcher and return its result dict."""
    fn = TOOLS.get(name)
    if fn is None:
        return {"ok": False, "error": f"unknown_tool: {name!r}"}

    try:
        if name == "wake_up":
            return fn(config.brain_dir)
        if name == "search":
            return fn(
                config.brain_dir,
                args.get("query", ""),
                limit=int(args.get("limit", 10) or 10),
                lobe=args.get("lobe"),
                tag=args.get("tag"),
            )
        if name == "read_neuron":
            return fn(
                config.brain_dir,
                args.get("path", ""),
                max_bytes=config.max_neuron_bytes,
            )
        if name == "multi_read":
            return fn(
                config.brain_dir,
                args.get("paths", []) or [],
                max_paths=config.max_multi_read_paths,
                max_bytes=config.max_neuron_bytes,
            )
        if name == "related":
            return fn(config.brain_dir, args.get("path", ""))
        if name == "recent":
            return fn(
                config.brain_dir,
                limit=int(args.get("limit", 10) or 10),
                lobe=args.get("lobe"),
                include_deprecated=bool(args.get("include_deprecated", False)),
            )
        if name == "glossary":
            return fn(config.brain_dir, args.get("term"))
        if name == "lobe_overview":
            return fn(
                config.brain_dir,
                args.get("lobe", ""),
                budget=config.lobe_overview_budget,
            )
    except SandboxError as exc:
        return {"ok": False, "error": f"sandbox: {exc}"}
    except NotFoundError as exc:
        return {"ok": False, "error": f"not_found: {exc}"}
    except Exception as exc:  # pragma: no cover (defensive)
        return {"ok": False, "error": f"tool_error: {exc}"}
    return {"ok": False, "error": f"unknown_tool: {name!r}"}


def _summarize_tool_result(name: str, result: dict[str, Any]) -> str:
    """Short, human-readable summary of a tool result for the SSE
    payload — full result is too big to send to the UI on every call.
    """
    if not result.get("ok", True):
        return f"error: {result.get('error', 'unknown')}"
    if name == "wake_up":
        return (
            f"{result.get('total_neurons', 0)} neurons across "
            f"{len(result.get('lobes', []))} lobes"
        )
    if name == "search":
        return f"{result.get('total', 0)} hits for {result.get('query')!r}"
    if name == "read_neuron":
        return f"{result.get('path')} ({len(result.get('body', ''))} chars)"
    if name == "multi_read":
        return f"{len(result.get('results', []))} neurons read"
    if name == "related":
        out = len(result.get("outbound", []))
        ins = len(result.get("inbound", []))
        return f"{out} outbound, {ins} inbound"
    if name == "recent":
        return f"{len(result.get('results', []))} neurons"
    if name == "glossary":
        if result.get("entries") is not None:
            return f"{len(result['entries'])} terms"
        match = result.get("match")
        return f"match: {match['term'] if match else 'none'}"
    if name == "lobe_overview":
        return (
            f"{result.get('lobe')}: {len(result.get('neurons', []))} neurons"
            + (" (truncated)" if result.get("truncated") else "")
        )
    return ""


async def run_agent(
    *,
    config: Config,
    provider: LLMProvider,
    history: list[dict[str, Any]],
    user_message: str,
    brain_name: str = "the",
    trace_hook: ToolTraceHook | None = None,
) -> AsyncIterator[dict[str, Any]]:
    """Stream a full agent turn for ``user_message``.

    Yields normalized event dicts:
    - ``{kind: "token", text: str}``
    - ``{kind: "tool", name: str, args: dict}``
    - ``{kind: "tool_result", tool: str, summary: str}``
    - ``{kind: "usage", input: int, output: int}``
    - ``{kind: "end"}``
    - ``{kind: "error", message: str, recoverable: bool}``
    """
    system = _system_prompt(config, brain_name)
    tools = _tool_schemas(config)

    messages: list[dict[str, Any]] = []
    # The providers translate this generic system message into their
    # native shape: OpenAI keeps role=system, Anthropic lifts it to the
    # top-level `system` request field.
    messages.append({"role": "system", "content": system})
    # Sliding-window trim so a long conversation never overflows the model's
    # context window — the oldest turns are dropped to fit the budget.
    messages.extend(_trim_history(history, config.max_context_tokens))
    messages.append({"role": "user", "content": user_message})

    rounds = 0
    # Tracked across the WHOLE turn (not per round): whether any tool ran (gates
    # the synthesis fallback) and which (tool, args) pairs we've already served
    # (so a flailing model can't re-issue the same call and re-bloat the
    # transcript).
    any_tools_used = False
    seen_calls: set[tuple[str, str]] = set()
    # tool_call_id -> its (tool, args) key, so when compaction elides a result we
    # can forget that key and let an identical re-issue actually re-fetch.
    dup_key_by_call_id: dict[str, tuple[str, str]] = {}
    # ``max_agent_rounds <= 0`` is the "unlimited" sentinel — keep
    # looping until the provider emits an end with no pending
    # tool_uses, regardless of round count.
    unlimited = config.max_agent_rounds <= 0
    while unlimited or rounds < config.max_agent_rounds:
        rounds += 1
        # Bound this single request: elide the oldest tool-result payloads when
        # the accumulated transcript would exceed the per-turn budget. Without it
        # a broad query re-sends every prior full result each round (quadratic).
        messages = _compact_tool_results(messages, config.max_turn_tokens)
        if config.max_turn_tokens > 0:
            # A result we just elided can no longer be "reused": drop its
            # dedup key so an identical re-issue re-dispatches instead of being
            # suppressed with a pointer to a stub we already discarded. ``pop``
            # (not ``get``) so each elided stub prunes its key exactly once — a
            # later re-fetch re-registers its own (new) call_id and stays
            # dedup-able.
            for _m in messages:
                if (
                    _m.get("role") == "tool"
                    and _m.get("content") == _COMPACTED_TOOL_RESULT
                ):
                    _k = dup_key_by_call_id.pop(_m.get("tool_call_id"), None)
                    if _k is not None:
                        seen_calls.discard(_k)
        pending_tools: list[dict[str, Any]] = []
        round_text: list[str] = []
        try:
            async for event in provider.complete_stream(messages, tools):
                kind = event.get("kind")
                if kind == "token":
                    round_text.append(str(event.get("text", "")))
                    yield event
                elif kind == "usage":
                    yield event
                elif kind == "tool_use":
                    pending_tools.append(event)
                    yield {
                        "kind": "tool",
                        "name": event.get("name", ""),
                        "args": event.get("args", {}),
                    }
                elif kind == "end":
                    pass  # we'll decide below whether to continue
                else:
                    yield event
        except (ContextLimitError, AuthError, RequestError) as exc:
            # Surface the provider's actual message (the RequestError carries the
            # response body, capped to 200 chars), not just the class name, and
            # classify recoverability — context overflow is recoverable ("New
            # conversation"), auth / bad-request is not. Shared with the
            # synthesis fallback via ``_provider_error_event`` so both paths
            # classify identically.
            yield _provider_error_event(exc)
            yield {"kind": "end"}
            return

        if config.debug_stream:
            # Per-round diagnostic (pairs with the stream-level line in the
            # provider): the input size actually SENT this round plus what came
            # back. The failing round in an empty-turn report is the last one
            # whose ``text=False`` and (here) no tools. Redaction-safe.
            sys.stderr.write(
                f"kluris-pack: round={rounds} "
                f"est_input_tokens={_estimate_messages_tokens(messages)} "
                f"tools={[c.get('name') for c in pending_tools]} "
                f"text={bool(round_text)}\n"
            )
            sys.stderr.flush()

        if not pending_tools:
            # The model is done calling tools. If it produced text, that's the
            # answer — end cleanly.
            if round_text:
                yield {"kind": "end"}
                return
            # ZERO text AND ZERO tool calls — an empty completion. If we already
            # gathered evidence this turn, the model likely went quiet mid-
            # research rather than having nothing to say: force one tools-disabled
            # synthesis pass before giving up. (With no tools run yet there's
            # nothing to synthesize from, so fall straight through to the error —
            # which also still covers a gateway that truncated mid-thought.)
            no_content_error = {
                "kind": "error",
                "message": (
                    "The model returned no content for this turn. "
                    "This usually means a server-side max_tokens cap "
                    "or a quirky gateway response. Try rephrasing "
                    "or asking a narrower question."
                ),
                "recoverable": True,
            }
            if any_tools_used:
                async for ev in _run_synthesis_fallback(
                    provider, messages, config, no_content_error
                ):
                    yield ev
                return
            # No tools ran yet — nothing to synthesize from, so surface the
            # empty-completion error directly (also covers a gateway that
            # truncated mid-thought on the very first round).
            yield no_content_error
            yield {"kind": "end"}
            return

        # Append the assistant tool-call request + tool results to the
        # conversation, then re-enter the loop for the next turn.
        any_tools_used = True
        assistant_tool_calls: list[dict[str, Any]] = []
        tool_result_messages: list[dict[str, Any]] = []
        for call in pending_tools:
            name = call.get("name") or ""
            args = call.get("args") or {}
            call_id = call.get("id") or f"tu_{rounds}_{name}"
            # Exact-duplicate suppression: a (tool, args) pair already served
            # this turn returns a tiny stub instead of re-running the tool and
            # re-sending its full payload (the model only needs it once).
            dup_key = (name, json.dumps(args, sort_keys=True, ensure_ascii=False))
            if dup_key in seen_calls:
                result = {
                    "ok": True,
                    "duplicate": True,
                    "note": (
                        f"Already called {name} with these arguments this turn; "
                        "reuse the earlier result instead of repeating it."
                    ),
                }
                summary = "duplicate call (reused earlier result)"
            else:
                seen_calls.add(dup_key)
                dup_key_by_call_id[call_id] = dup_key
                result = _dispatch_tool(config, name, args)
                summary = _summarize_tool_result(name, result)
            if trace_hook is not None:
                trace_hook({
                    "round": rounds,
                    "tool_name": name,
                    "args": args,
                    "result_summary": summary,
                })
            yield {"kind": "tool_result", "tool": name, "summary": summary}
            assistant_tool_calls.append({
                "id": call_id,
                "name": name,
                "args": args,
            })
            tool_result_messages.append({
                "role": "tool",
                "tool_call_id": call_id,
                "content": json.dumps(result, ensure_ascii=False),
            })
        messages.append({
            "role": "assistant",
            "content": "".join(round_text),
            "tool_calls": assistant_tool_calls,
        })
        messages.extend(tool_result_messages)

    # Round budget exhausted. Rather than throw the turn away after N rounds of
    # gathering, make one tools-disabled synthesis pass so the user still gets an
    # answer (or an explicit "not in the brain") from everything found so far.
    round_budget_error = {
        "kind": "error",
        "message": (
            f"Hit the max {config.max_agent_rounds}-round tool budget. "
            "Try a narrower question or click 'New conversation'."
        ),
        "recoverable": True,
    }
    async for ev in _run_synthesis_fallback(
        provider, messages, config, round_budget_error
    ):
        yield ev
