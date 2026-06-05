"""Build-once wake_up snapshot cache + recency-sort hardening.

The brain is immutable inside the container, so the wake_up payload is built
once at boot and reused by the agent's first-call-of-session wake_up and every
``/api/brain/tree`` UI load, skipping the brain re-walk. The cache holds plain
JSON (no handle), so unlike a SQLite connection it is safe to read from the
request thread — these tests prove that end-to-end.
"""

from __future__ import annotations

from fastapi.testclient import TestClient

import kluris.pack.tools.brain as bm
from kluris.pack.main import create_app
from kluris_runtime.wake_up import build_payload


def test_cached_wake_up_matches_fresh_and_skips_rebuild(fixture_brain, monkeypatch):
    """A cache hit returns a payload equal to a fresh build and does NOT
    recompute. Proven by sabotaging build_payload after caching — a hit must
    not call it. (We assert equality, not object identity, so by-reference
    return stays an implementation detail rather than a pinned contract.)"""
    fresh = bm.wake_up_tool(fixture_brain)  # miss → build_payload
    bm.build_wake_up_cache(fixture_brain)

    def _no_recompute(*a, **k):
        raise RuntimeError("recomputed on cache hit")

    monkeypatch.setattr(bm, "build_payload", _no_recompute)
    assert bm.wake_up_tool(fixture_brain) == fresh  # hit, no recompute


def test_wake_up_miss_builds_fresh(fixture_brain):
    """With no cache entry, wake_up_tool behaves exactly as before (fresh)."""
    assert fixture_brain.resolve() not in bm._WAKE_UP_CACHE
    assert bm.wake_up_tool(fixture_brain) == build_payload(fixture_brain)


def test_boot_builds_wake_up_cache(api_key_config, stub_provider):
    """create_app precomputes + registers the snapshot and flips the health flag."""
    app = create_app(
        config=api_key_config, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )
    assert app.state.wake_up_cached is True
    assert api_key_config.brain_dir.resolve() in bm._WAKE_UP_CACHE


def test_route_tree_serves_cache_across_threads(
    api_key_config, stub_provider, monkeypatch,
):
    """REGRESSION GUARD: GET /api/brain/tree must serve the boot-built snapshot
    from the request thread (TestClient/anyio portal) — not silently recompute.
    Sabotage fresh builds after boot so only a live cache hit can answer 200.
    """
    app = create_app(
        config=api_key_config, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )
    assert app.state.wake_up_cached is True

    def _no_recompute(*a, **k):
        raise RuntimeError("wake_up recomputed on the request thread — cache missed")

    monkeypatch.setattr(bm, "build_payload", _no_recompute)
    client = TestClient(app)
    resp = client.get("/api/brain/tree")
    assert resp.status_code == 200
    data = resp.json()
    assert data["ok"] is True
    assert isinstance(data["lobes"], list)


def test_wake_up_cache_staleness_and_drop(fixture_brain):
    """A cached snapshot is a frozen view of an immutable brain — an in-place
    edit is invisible until drop_wake_up_cache, the documented contract."""
    bm.build_wake_up_cache(fixture_brain)
    before = bm.wake_up_tool(fixture_brain)["total_neurons"]
    (fixture_brain / "knowledge" / "extra.md").write_text(
        "---\nupdated: 2026-05-01\n---\n# Extra\n\nbody.\n", encoding="utf-8"
    )
    assert bm.wake_up_tool(fixture_brain)["total_neurons"] == before  # stale
    bm.drop_wake_up_cache(fixture_brain)
    assert bm.wake_up_tool(fixture_brain)["total_neurons"] == before + 1  # fresh


def test_boot_wake_up_failure_degrades_silently(
    api_key_config, stub_provider, monkeypatch,
):
    """A build_wake_up_cache failure at boot must not kill create_app or affect
    the (independent) search-index build — it flips app.state.wake_up_cached
    False, and wake_up still answers via the per-call fallback."""
    def _boom(*a, **k):
        raise RuntimeError("simulated wake_up cache build failure")

    monkeypatch.setattr(bm, "build_wake_up_cache", _boom)
    app = create_app(
        config=api_key_config, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )
    assert app.state.wake_up_cached is False
    assert app.state.search_index is True  # search build is independent
    assert api_key_config.brain_dir.resolve() not in bm._WAKE_UP_CACHE
    # per-call fallback still answers
    assert bm.wake_up_tool(api_key_config.brain_dir)["ok"] is True


def test_agent_dispatch_uses_wake_up_cache(fixture_brain, tmp_path, monkeypatch):
    """The agent's _dispatch_tool('wake_up') path serves the boot-cached
    snapshot — the same cache the route uses, reached via Config.brain_dir.
    With fresh builds sabotaged, only a cache hit yields ok=True (a miss would
    raise and _dispatch_tool would return ok=False)."""
    from kluris.pack.agent import _dispatch_tool
    from kluris.pack.config import Config

    bm.build_wake_up_cache(fixture_brain)

    def _no_recompute(*a, **k):
        raise RuntimeError("wake_up recomputed — cache missed")

    monkeypatch.setattr(bm, "build_payload", _no_recompute)
    cfg = Config(brain_dir=fixture_brain, data_dir=tmp_path / "data")
    result = _dispatch_tool(cfg, "wake_up", {})
    assert result["ok"] is True
    assert isinstance(result["lobes"], list)


def test_recent_orders_iso_and_demotes_non_iso(tmp_path):
    """Recency sort: ISO dates newest-first (unchanged); a non-ISO `updated:`
    sorts BELOW all valid dates instead of jumping the list lexicographically.
    """
    brain = tmp_path / "brain"
    (brain / "k").mkdir(parents=True)
    (brain / "brain.md").write_text("# B\n", encoding="utf-8")
    (brain / "glossary.md").write_text("# G\n", encoding="utf-8")

    def neuron(name, updated):
        (brain / "k" / name).write_text(
            f"---\nupdated: {updated}\n---\n# {name}\n", encoding="utf-8"
        )

    neuron("newer.md", "2026-04-15")
    neuron("older.md", "2026-01-01")
    neuron("bad.md", "not-a-date")  # old str-sort would float this to the TOP

    paths = [r["path"] for r in build_payload(brain)["recent"]]
    assert paths == ["k/newer.md", "k/older.md", "k/bad.md"]


def test_recent_tool_agrees_with_wake_up_on_non_iso(tmp_path):
    """The `recent` tool must order the same way as wake_up's recent[]: a
    non-ISO `updated:` sorts BELOW valid dates. (Before the shared recency key,
    recent_tool floated 'not-a-date' to the TOP — the two surfaces disagreed.)
    """
    from kluris.pack.tools.brain import recent_tool

    brain = tmp_path / "brain"
    (brain / "k").mkdir(parents=True)
    (brain / "brain.md").write_text("# B\n", encoding="utf-8")
    (brain / "glossary.md").write_text("# G\n", encoding="utf-8")
    for name, updated in (("newer.md", "2026-04-15"),
                          ("older.md", "2026-01-01"),
                          ("bad.md", "not-a-date")):
        (brain / "k" / name).write_text(
            f"---\nupdated: {updated}\n---\n# {name}\n", encoding="utf-8"
        )
    paths = [r["path"] for r in recent_tool(brain)["results"]]
    assert paths == ["k/newer.md", "k/older.md", "k/bad.md"]


def test_recent_ranks_newer_yaml_ahead_of_older_md(tmp_path):
    """A newer yaml neuron must rank ahead of an older md neuron, even though
    the brain walk yields .md before .yml — the recency key overrides that
    walk-order bias. Holds on both the wake_up payload and the recent tool."""
    from kluris.pack.tools.brain import recent_tool

    brain = tmp_path / "brain"
    (brain / "infra").mkdir(parents=True)
    (brain / "k").mkdir()
    (brain / "brain.md").write_text("# B\n", encoding="utf-8")
    (brain / "glossary.md").write_text("# G\n", encoding="utf-8")
    (brain / "k" / "old.md").write_text(
        "---\nupdated: 2026-01-01\n---\n# Old\n", encoding="utf-8"
    )
    (brain / "infra" / "api.yml").write_text(
        "#---\n# updated: 2026-06-01\n#---\nopenapi: 3.1.0\n", encoding="utf-8"
    )
    for paths in (
        [r["path"] for r in build_payload(brain)["recent"]],
        [r["path"] for r in recent_tool(brain)["results"]],
    ):
        assert paths.index("infra/api.yml") < paths.index("k/old.md")
