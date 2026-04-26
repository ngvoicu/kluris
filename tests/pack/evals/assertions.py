"""Deterministic answer-shape assertion helpers."""

from __future__ import annotations

import re
from typing import Iterable


def assert_starts_with_direct_answer(text: str) -> None:
    """Reject obvious "let me think" / "I'll use the tools" preludes."""
    head = (text.strip().split("\n", 1)[0]).lower()
    forbidden = [
        "let me",
        "i'll start",
        "i'll use",
        "i need to",
        "first, i",
    ]
    assert not any(head.startswith(p) for p in forbidden), (
        f"answer should not start with a tool-use prelude: {head!r}"
    )


def assert_cites_paths_inline(text: str, paths: Iterable[str]) -> None:
    """Each path must appear at least once in the answer body."""
    for p in paths:
        assert p in text, f"answer must cite source path {p!r}"


def assert_has_sources_block(text: str, paths: Iterable[str]) -> None:
    """Answers based on 2+ neurons must end with a Sources: block."""
    pattern = re.compile(r"(?im)^sources:\s*$")
    assert pattern.search(text), "answer must end with a 'Sources:' block"
    for p in paths:
        assert p in text, f"Sources block must list {p!r}"


def assert_no_invention(text: str, forbidden: Iterable[str]) -> None:
    """The answer must not contain made-up terms."""
    for word in forbidden:
        assert word.lower() not in text.lower(), (
            f"answer must not invent {word!r}"
        )


def assert_says_no_answer(text: str) -> None:
    """No-answer cases must surface that explicitly."""
    lower = text.lower()
    markers = [
        "nothing in the brain",
        "no answer",
        "the brain does not",
        "could not find",
    ]
    assert any(m in lower for m in markers), (
        "no-answer case must say so plainly"
    )


def assert_calls_out_conflict(text: str) -> None:
    lower = text.lower()
    markers = ["conflict", "disagree", "differs", "contradict"]
    assert any(m in lower for m in markers), (
        "conflict case must surface the conflict explicitly"
    )


def assert_names_replacement(text: str, replacement_path: str) -> None:
    assert replacement_path in text, (
        f"deprecated case must direct the reader to {replacement_path!r}"
    )


def assert_trace_starts_with(trace: list[dict], expected_prefix: list[str]) -> None:
    actual = [t["tool_name"] for t in trace[: len(expected_prefix)]]
    assert actual == expected_prefix, (
        f"expected trace prefix {expected_prefix}, got {actual}"
    )


def assert_no_duplicate_reads(trace: list[dict]) -> None:
    """``read_neuron`` should not visit the same path twice."""
    seen: set[str] = set()
    for entry in trace:
        if entry["tool_name"] != "read_neuron":
            continue
        path = (entry.get("args") or {}).get("path")
        if not path:
            continue
        assert path not in seen, (
            f"agent re-read {path!r} — should remember from earlier turn"
        )
        seen.add(path)
