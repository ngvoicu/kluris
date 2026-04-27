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
from .tools.schemas import anthropic_schemas, openai_schemas


# Per-call test-only trace hook. Production code never reads this; the
# scripted-provider eval harness sets a callable that records every
# tool call/result for assertions.
ToolTraceHook = Callable[[dict[str, Any]], None]


def _system_prompt(config: Config, brain_name: str) -> str:
    prompt_path = config.data_dir / "config" / "system_prompt.md"
    return load_prompt(prompt_path, brain_name=brain_name)


def _tool_schemas(config: Config) -> list[dict[str, Any]]:
    if config.provider_shape == "openai" or config.auth_mode == "oauth":
        return openai_schemas(max_multi_read=config.max_multi_read_paths)
    return anthropic_schemas(max_multi_read=config.max_multi_read_paths)


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
            return fn(config.brain_dir, args.get("path", ""))
        if name == "multi_read":
            return fn(
                config.brain_dir,
                args.get("paths", []) or [],
                max_paths=config.max_multi_read_paths,
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
    messages.extend(history)
    messages.append({"role": "user", "content": user_message})

    rounds = 0
    cumulative_input = 0
    cumulative_output = 0
    while rounds < config.max_agent_rounds:
        rounds += 1
        pending_tools: list[dict[str, Any]] = []
        round_text: list[str] = []
        try:
            async for event in provider.complete_stream(messages, tools):
                kind = event.get("kind")
                if kind == "token":
                    round_text.append(str(event.get("text", "")))
                    yield event
                elif kind == "usage":
                    cumulative_input += int(event.get("input", 0))
                    cumulative_output += int(event.get("output", 0))
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
        except ContextLimitError:
            yield {
                "kind": "error",
                "message": (
                    "Conversation has grown beyond the model's context window. "
                    "Click 'New conversation' to start fresh."
                ),
                "recoverable": True,
            }
            yield {"kind": "end"}
            return
        except (AuthError, RequestError) as exc:
            yield {
                "kind": "error",
                "message": f"Provider error: {type(exc).__name__}",
                "recoverable": False,
            }
            yield {"kind": "end"}
            return

        if not pending_tools:
            # No more tools requested. If the round produced ANY text,
            # we have a real answer — emit end and return cleanly.
            #
            # If the round produced ZERO tokens AND ZERO tool_uses, the
            # provider returned an empty completion (some Bedrock-fronted
            # gateways do this; the model may have gone over its server-
            # side max_tokens budget mid-thought, or simply emitted a
            # bare ``stop_reason`` with no content). Don't leave the
            # user staring at a blank assistant block — surface a
            # visible recoverable error so they know to retry.
            if not round_text:
                yield {
                    "kind": "error",
                    "message": (
                        "The model returned no content for this turn. "
                        "This usually means a server-side max_tokens cap "
                        "or a quirky gateway response. Try rephrasing "
                        "or asking a narrower question."
                    ),
                    "recoverable": True,
                }
            yield {"kind": "end"}
            return

        # Append the assistant tool-call request + tool results to the
        # conversation, then re-enter the loop for the next turn.
        assistant_tool_calls: list[dict[str, Any]] = []
        tool_result_messages: list[dict[str, Any]] = []
        for call in pending_tools:
            name = call.get("name") or ""
            args = call.get("args") or {}
            call_id = call.get("id") or f"tu_{rounds}_{name}"
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

    yield {
        "kind": "error",
        "message": (
            f"Hit the max {config.max_agent_rounds}-round tool budget. "
            "Try a narrower question or click 'New conversation'."
        ),
        "recoverable": True,
    }
    yield {"kind": "end"}


def total_usage_event(
    input_tokens: int, output_tokens: int
) -> dict[str, Any]:
    return {"kind": "usage", "input": input_tokens, "output": output_tokens}
