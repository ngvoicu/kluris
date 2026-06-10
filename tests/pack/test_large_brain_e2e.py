"""End-to-end behaviour on a LARGE brain (~3,200 neurons / 9 lobes).

Reproduces the production profile that motivated v2.28.0/2.28.1 — a 9-country
MasterCard-docs-shaped brain and the broad "rate for X across all countries"
fan-out — and asserts the cost-control machinery holds:

- every snapshot-served tool stays fast and never re-walks the brain,
- group_by_lobe covers every country in ONE call,
- the in-turn request stays bounded across a long fan-out,
- wake_up is never elided (no re-orient churn) and snippet-only re-searches
  are suppressed,
- the round cap still yields a synthesized answer.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from kluris.pack.agent import (
    _SEEN_TOOL_RESULT,
    _estimate_messages_tokens,
    run_agent,
)
from kluris.pack.config import Config
from kluris.pack.providers.base import LLMProvider
from kluris.pack.search_index import build_search_db
from kluris.pack.tools.brain import (
    _clear_wake_up_cache,
    build_wake_up_cache,
    lobe_overview_tool,
    recent_tool,
    related_tool,
    search_tool,
)
from kluris_runtime.search_fts import build_index, register_db
from kluris_runtime.snapshot import build_snapshot, register_snapshot

COUNTRIES = [
    "bulgaria", "germany", "greece", "hungary", "lithuania",
    "poland", "romania", "spain", "uk",
]
PER_COUNTRY = 360  # 9 * 360 = 3,240 neurons


@pytest.fixture(scope="module")
def big_brain(tmp_path_factory) -> Path:
    """A ~3,240-neuron, 9-country brain shaped like the real mastercard-docs
    brain: each country lobe has a scheme/ sub-lobe of card-product neurons,
    a few of which carry the 'Card Payment Promotion Fund' rate the broad
    query is after."""
    brain = tmp_path_factory.mktemp("big-brain")
    (brain / "brain.md").write_text(
        "# MasterCard Docs\n\nIssuer + acquirer scheme documentation.\n",
        encoding="utf-8",
    )
    (brain / "glossary.md").write_text(
        "# Glossary\n\n**CPF** -- Card Payment Promotion Fund\n"
        "**Intra-European** -- transactions within the European region\n",
        encoding="utf-8",
    )
    for ci, country in enumerate(COUNTRIES):
        scheme = brain / country / "scheme"
        scheme.mkdir(parents=True)
        (brain / country / "map.md").write_text(
            f"# {country.title()}\n\n{country} scheme docs.\n", encoding="utf-8"
        )
        (scheme / "map.md").write_text("# Scheme\n", encoding="utf-8")
        # The acquirer-assessment neuron carries the CPF rate for this country.
        (scheme / "ae-acquirer-assessment-part-1.md").write_text(
            f"---\nupdated: 2026-06-0{(ci % 9) + 1}\ntags: [acquirer, rates, cpf]\n---\n"
            f"# Acquirer Assessment — {country.title()}\n\n"
            "Card Payment Promotion Fund — Intra European — MasterCard. "
            f"The acquirer is billed quarterly. Rate: EUR 0.000{ci + 1}.\n",
            encoding="utf-8",
        )
        # Bulk of the lobe: many card-product neurons (noise for the ranker).
        for ni in range(PER_COUNTRY - 1):
            (scheme / f"product-{ni:03d}.md").write_text(
                f"---\ntags: [product, fee-{ni % 7}]\nrelated:\n"
                f"  - ./product-{(ni + 1) % (PER_COUNTRY - 1):03d}.md\n---\n"
                f"# Product {ni} — {country.title()}\n\n"
                "Standard MasterCard product fee schedule and event codes.\n",
                encoding="utf-8",
            )
    return brain


@pytest.fixture(scope="module")
def big_brain_assets(big_brain):
    """Build the (expensive) snapshot + on-disk FTS db ONCE for the module."""
    snap = build_snapshot(big_brain)
    db_path = big_brain.parent / "cache" / "fts.sqlite"
    build_search_db(big_brain, snap["rows"], db_path)
    return big_brain, snap, db_path


@pytest.fixture
def booted_big_brain(big_brain_assets):
    """Register the prebuilt snapshot + indexes for THIS test, exactly as
    create_app wires them. Function-scoped because the autouse cache-reset in
    conftest clears the registries after every test — so each test re-registers
    the (cheap to register, already-built) assets."""
    brain, snap, db_path = big_brain_assets
    register_snapshot(brain, snap)
    build_index(brain, rows=snap["rows"])
    register_db(brain, db_path)
    build_wake_up_cache(brain, snap)
    return brain, snap


def _config(brain: Path, tmp_path: Path, **overrides) -> Config:
    env = dict(
        {
            "KLURIS_PROVIDER_SHAPE": "openai",
            "KLURIS_BASE_URL": "http://api.test",
            "KLURIS_API_KEY": "sk-test",
            "KLURIS_MODEL": "gpt-5.4-mini",
            "KLURIS_BRAIN_DIR": str(brain),
            "KLURIS_DATA_DIR": str(tmp_path / "data"),
        },
        **overrides,
    )
    (tmp_path / "data").mkdir(exist_ok=True)
    return Config.load_from_env(env)


class _RecordingProvider(LLMProvider):
    model = "rec"

    def __init__(self, scripts):
        self._scripts = list(scripts)
        self.seen_messages = []
        self.calls = 0

    async def smoke_test(self):  # pragma: no cover
        return None

    async def complete_stream(self, messages, tools):
        self.seen_messages.append([dict(m) for m in messages])
        self.calls += 1
        if self._scripts:
            for ev in self._scripts.pop(0):
                yield ev


# --- the brain itself scales -----------------------------------------------------


def test_big_brain_has_expected_scale(booted_big_brain):
    _brain, snap = booted_big_brain
    assert len(snap["entries"]) == len(COUNTRIES) * PER_COUNTRY  # 3,240
    assert len(snap["lobe_tags"]) == len(COUNTRIES)


def test_group_by_lobe_answers_across_all_countries_in_one_call(booted_big_brain):
    brain, _snap = booted_big_brain
    out = search_tool(
        brain, "Card Payment Promotion Fund intra European",
        limit=2, group_by_lobe=True,
    )
    assert out["grouped_by_lobe"] is True
    # Every country surfaces its own best hits in ONE call — the shortcut.
    # ('(root)' may also appear from the matching glossary entry; the contract
    # is that no country is missing.)
    assert set(COUNTRIES) <= set(out["lobes"].keys())
    # And the acquirer-assessment (rate-bearing) neuron is a top hit per country.
    for country in COUNTRIES:
        hits = out["lobes"][country]
        assert any("ae-acquirer-assessment" in h["file"] for h in hits)


def test_full_bodies_returns_rate_content_in_one_call(booted_big_brain):
    brain, _snap = booted_big_brain
    out = search_tool(
        brain, "Card Payment Promotion Fund acquirer", limit=10, full_bodies=5,
    )
    bodies = [r for r in out["results"] if "body" in r]
    assert bodies
    assert any("Rate: EUR" in r["body"] for r in bodies)


def test_snapshot_served_tools_do_not_rewalk_on_big_brain(
    booted_big_brain, monkeypatch
):
    brain, _snap = booted_big_brain
    import kluris.pack.tools.brain as brain_mod

    def _no_walk(*_a, **_k):
        raise AssertionError("tool re-walked the 3,240-neuron brain")

    monkeypatch.setattr(brain_mod, "neuron_files", _no_walk)
    assert related_tool(brain, "germany/scheme/product-005.md")["ok"] is True
    assert recent_tool(brain, limit=10)["ok"] is True
    monkeypatch.setattr(brain_mod, "read_frontmatter", lambda p: ({}, "# Map\n"))
    assert lobe_overview_tool(brain, "germany", budget=4096)["ok"] is True


# --- the full agent turn on the broad query --------------------------------------


def _broad_query_fan_out():
    """A scripted fan-out that mirrors the production screenshots: wake_up,
    then many searches (some varying ONLY snippet_chars — the wasteful
    re-search), a re-issued wake_up, group_by_lobe, full_bodies, a wide
    multi_read, then a final answer."""
    q = "Card Payment Promotion Fund intra European MasterCard"
    return [
        [{"kind": "tool_use", "name": "wake_up", "id": "w1", "args": {}},
         {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "s1",
          "args": {"query": q, "limit": 5, "group_by_lobe": True,
                   "snippet_chars": 180}}, {"kind": "end"}],
        # snippet-only re-search (should be suppressed):
        [{"kind": "tool_use", "name": "search", "id": "s2",
          "args": {"query": q, "limit": 5, "group_by_lobe": True,
                   "snippet_chars": 400}}, {"kind": "end"}],
        # model re-orients (wake_up again — should be a cheap pointer dup):
        [{"kind": "tool_use", "name": "wake_up", "id": "w2", "args": {}},
         {"kind": "end"}],
        [{"kind": "tool_use", "name": "search", "id": "s3",
          "args": {"query": q, "limit": 10, "full_bodies": 5}},
         {"kind": "end"}],
        [{"kind": "tool_use", "name": "multi_read", "id": "m1",
          "args": {"paths": [
              f"{c}/scheme/ae-acquirer-assessment-part-1.md"
              for c in COUNTRIES[:5]
          ]}}, {"kind": "end"}],
        [{"kind": "token", "text": "Here are the CPF rates per country."},
         {"kind": "end"}],
    ]


@pytest.mark.asyncio
async def test_broad_query_turn_stays_bounded_and_answers(
    booted_big_brain, tmp_path
):
    brain, _snap = booted_big_brain
    cfg = _config(
        brain, tmp_path,
        MAX_AGENT_ROUNDS="12",
        KLURIS_KEEP_RESULT_ROUNDS="2",
    )
    provider = _RecordingProvider(_broad_query_fan_out())
    traces = []
    events = await _drain_collect(run_agent(
        config=cfg, provider=provider, history=[],
        user_message="Provide the rate for Card Payment Promotion Fund — "
                     "Intra European — MasterCard for all countries",
        trace_hook=traces.append,
    ))

    # Answered.
    tokens = [e for e in events if e["kind"] == "token"]
    assert any("CPF rates per country" in t["text"] for t in tokens)
    assert not any(e["kind"] == "error" for e in events)

    # The snippet-only re-search was suppressed (not re-dispatched).
    summaries = [t["result_summary"] for t in traces]
    assert any("duplicate" in s.lower() for s in summaries)

    # Every per-round request stayed comfortably under the turn budget — no
    # quadratic blow-up even though the fan-out gathered full bodies + a
    # 5-path multi_read on a 3,240-neuron brain.
    peak = max(_estimate_messages_tokens(m) for m in provider.seen_messages)
    assert peak < cfg.max_turn_tokens, f"peak per-round estimate {peak} >= budget"

    # wake_up stayed resident in the FINAL request (no re-orient churn).
    final = provider.seen_messages[-1]
    w1 = next(m for m in final if m.get("tool_call_id") == "w1")
    assert w1["content"] != _SEEN_TOOL_RESULT


@pytest.mark.asyncio
async def test_round_cap_still_answers_on_relentless_fan_out(
    booted_big_brain, tmp_path
):
    """A model that never stops searching hits the round cap — and the
    synthesis fallback still returns an answer built from the evidence."""
    brain, _snap = booted_big_brain
    cfg = _config(brain, tmp_path, MAX_AGENT_ROUNDS="6")
    looper = [
        {"kind": "tool_use", "name": "search", "id": "tu",
         "args": {"query": "promotion fund", "full_bodies": 3}},
        {"kind": "end"},
    ]
    synthesis = [{"kind": "token", "text": "Synthesized rate summary."},
                 {"kind": "end"}]
    # 6 rounds of looping (each a distinct call_id), then the synthesis pass.
    scripts = [
        [{"kind": "tool_use", "name": "search", "id": f"tu{i}",
          "args": {"query": f"promotion fund {i}", "full_bodies": 3}},
         {"kind": "end"}]
        for i in range(6)
    ] + [synthesis]
    provider = _RecordingProvider(scripts)
    events = await _drain_collect(run_agent(
        config=cfg, provider=provider, history=[], user_message="rates?",
    ))
    tokens = [e for e in events if e["kind"] == "token"]
    assert any("Synthesized rate summary" in t["text"] for t in tokens)


async def _drain_collect(agent_iter):
    return [ev async for ev in agent_iter]
