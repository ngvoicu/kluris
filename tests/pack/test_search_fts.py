"""BM25/FTS5 search — ranking, prefix recall, filters, fallback, and the
canary that proves the pack actually runs FTS5 (not a silent substring
fallback that would mask a broken index path)."""

from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

import kluris_runtime.search_fts as sf
from kluris.pack.tools.brain import search_tool
from kluris_runtime.search import search_brain
from kluris_runtime.search_fts import fts5_available, search_brain_fts

requires_fts = pytest.mark.skipif(
    not fts5_available(), reason="sqlite3 built without FTS5 here"
)

_RESULT_KEYS = {
    "file", "file_type", "title", "matched_fields", "snippet", "score", "deprecated",
}


def _brain(tmp_path: Path) -> Path:
    b = tmp_path / "brain"
    (b / "knowledge").mkdir(parents=True)
    (b / "projects").mkdir()
    (b / "brain.md").write_text("# Demo\n", encoding="utf-8")
    (b / "glossary.md").write_text("# Glossary\n", encoding="utf-8")
    # 'auth' and 'flow' far apart — substring "auth flow" scores 0.
    (b / "projects" / "login.md").write_text(
        "# Login\n\nThe service starts auth at the gateway. Much later, after "
        "several hops, the response flow returns to the client.\n",
        encoding="utf-8",
    )
    # Prefix recall: says 'authentication', not the bare word 'auth'.
    (b / "knowledge" / "sso.md").write_text(
        "# SSO\n\nSingle sign-on relies on authentication tokens.\n",
        encoding="utf-8",
    )
    # Deprecated neuron carrying a tag, for filter + flag tests.
    (b / "knowledge" / "legacy.md").write_text(
        "---\nstatus: deprecated\ntags: [decision]\n---\n"
        "# Legacy\n\nold guidance lives here.\n",
        encoding="utf-8",
    )
    return b


@requires_fts
def test_fts_finds_nonadjacent_multiword_that_substring_misses(tmp_path):
    """CANARY: 'auth flow' has its two words far apart. The substring engine
    (the fallback) scores it 0; FTS5 must find it — and the pack tool must too,
    which only happens if FTS5 is the live path, not a silent fallback."""
    brain = _brain(tmp_path)
    # Substring engine: contiguous-phrase requirement → no hit.
    assert not any(
        r["file"] == "projects/login.md"
        for r in search_brain(brain, "auth flow", limit=10)
    )
    # FTS5 engine directly: tokenized OR → hit.
    assert "projects/login.md" in {
        r["file"] for r in search_brain_fts(brain, "auth flow", limit=10)
    }
    # Through the real pack tool: FTS5 must be live here.
    assert "projects/login.md" in {
        r["file"] for r in search_tool(brain, "auth flow")["results"]
    }


@requires_fts
def test_fts_prefix_recall_reaches_authentication(tmp_path):
    """Query 'auth' must reach a doc that only says 'authentication' via
    prefix matching."""
    brain = _brain(tmp_path)
    files = {r["file"] for r in search_brain_fts(brain, "auth", limit=10)}
    assert "knowledge/sso.md" in files


@requires_fts
def test_fts_output_shape_matches_substring_contract(tmp_path):
    brain = _brain(tmp_path)
    hits = search_brain_fts(brain, "authentication", limit=10)
    assert hits
    for r in hits:
        assert _RESULT_KEYS <= set(r.keys())
        assert isinstance(r["score"], float) and r["score"] >= 0  # higher = better


@requires_fts
def test_fts_lobe_and_tag_filters(tmp_path):
    brain = _brain(tmp_path)
    lobe = search_brain_fts(
        brain, "authentication", limit=10, lobe_filter="knowledge",
    )
    assert lobe and all(r["file"].startswith("knowledge/") for r in lobe)
    tagged = search_brain_fts(brain, "guidance", limit=10, tag_filter="decision")
    assert tagged and all(r["file"] == "knowledge/legacy.md" for r in tagged)


@requires_fts
def test_fts_preserves_deprecated_flag(tmp_path):
    brain = _brain(tmp_path)
    hits = search_brain_fts(brain, "old guidance", limit=10)
    assert any(
        r["deprecated"] for r in hits if r["file"] == "knowledge/legacy.md"
    )


def _boom(*a, **k):
    raise sqlite3.Error("simulated FTS5 failure")


def test_falls_back_to_substring_on_fts_error(tmp_path, monkeypatch):
    """If the FTS5 path raises (e.g. a build that lacks it at runtime), the
    function must delegate to the substring engine, not crash or return []."""
    brain = _brain(tmp_path)
    monkeypatch.setattr(sf, "fts5_available", lambda: True)  # bypass the skip guard
    monkeypatch.setattr(sf.sqlite3, "connect", _boom)        # force the FTS5 path to error
    out = search_brain_fts(brain, "authentication", limit=10)
    assert any(r["file"] == "knowledge/sso.md" for r in out)


def test_no_token_query_falls_back_without_error(tmp_path):
    """A query with no word characters yields no FTS5 expr → substring
    fallback. Must match the substring engine and not raise."""
    brain = _brain(tmp_path)
    assert search_brain_fts(brain, "!!!", limit=10) == search_brain(
        brain, "!!!", limit=10
    )


# --- build-once-at-boot index (build_index / registry) -----------------------


@requires_fts
def test_prebuilt_index_matches_per_query_no_filter(tmp_path):
    """The prebuilt full-corpus index returns results byte-identical to the
    per-query path for an unfiltered query (today's path also indexes ALL
    items, so BM25 IDF is unchanged)."""
    brain = _brain(tmp_path)
    baseline = search_brain_fts(brain, "authentication", limit=10)  # registry miss
    sf.build_index(brain)
    assert search_brain_fts(brain, "authentication", limit=10) == baseline  # hit


@requires_fts
def test_prebuilt_index_matches_per_query_filtered(tmp_path):
    """Filtered queries must STILL rebuild a subset table even when an index
    is registered, so IDF stays scoped to the eligible rows (identical to
    today). Guards against 'optimizing' filtered queries onto the full index."""
    brain = _brain(tmp_path)
    base_lobe = search_brain_fts(brain, "authentication", lobe_filter="knowledge")
    base_tag = search_brain_fts(brain, "guidance", tag_filter="decision")
    sf.build_index(brain)
    assert search_brain_fts(brain, "authentication", lobe_filter="knowledge") == base_lobe
    assert search_brain_fts(brain, "guidance", tag_filter="decision") == base_tag


@requires_fts
def test_prebuilt_index_keeps_canary_live(tmp_path):
    """FTS5 must remain the LIVE path through the prebuilt index: the
    non-adjacent 'auth flow' hit the substring engine misses is still found by
    both search_brain_fts and the pack search_tool after build_index."""
    brain = _brain(tmp_path)
    sf.build_index(brain)
    assert "projects/login.md" in {
        r["file"] for r in search_brain_fts(brain, "auth flow")
    }
    assert "projects/login.md" in {
        r["file"] for r in search_tool(brain, "auth flow")["results"]
    }


@requires_fts
def test_registry_miss_falls_back_to_per_query(tmp_path):
    """With no build_index call, search_brain_fts behaves exactly as before —
    the standalone per-query path is unchanged."""
    brain = _brain(tmp_path)
    assert brain.resolve() not in sf._INDEX_REGISTRY
    assert "projects/login.md" in {
        r["file"] for r in search_brain_fts(brain, "auth flow")
    }


@requires_fts
def test_stale_cache_guarded_by_drop_index(tmp_path):
    """A registered index is a frozen snapshot of an immutable brain. Editing
    the brain in place is invisible until drop_index + build_index — the
    documented invalidation contract."""
    brain = _brain(tmp_path)
    sf.build_index(brain)
    (brain / "knowledge" / "fresh.md").write_text(
        "# Fresh\n\nbrand new keycloak material.\n", encoding="utf-8"
    )
    # Stale snapshot: the new neuron is not yet visible.
    assert "knowledge/fresh.md" not in {
        r["file"] for r in search_brain_fts(brain, "keycloak")
    }
    sf.drop_index(brain)
    sf.build_index(brain)
    assert "knowledge/fresh.md" in {
        r["file"] for r in search_brain_fts(brain, "keycloak")
    }


def test_boot_builds_search_index(api_key_config, stub_provider):
    """create_app caches the brain walk at boot and flips the
    app.state.search_index health flag."""
    from kluris.pack.main import create_app

    app = create_app(
        config=api_key_config,
        provider=stub_provider,
        allow_writable_brain=True,
        skip_smoke_test=True,
    )
    assert app.state.search_index is True
    assert api_key_config.brain_dir.resolve() in sf._INDEX_REGISTRY


@requires_fts
def test_route_search_uses_live_fts_across_threads(tmp_path, stub_provider):
    """REGRESSION GUARD: the brain-explorer route must run LIVE FTS5, not a
    silent substring fallback, even though create_app builds the cache on a
    different thread than the one serving the request (TestClient / anyio
    portal). 'auth flow' is the non-adjacent multiword the substring engine
    scores 0 — if it comes back, FTS5 is genuinely live on the request thread.
    """
    from fastapi.testclient import TestClient
    from kluris.pack.config import Config
    from kluris.pack.main import create_app

    (tmp_path / "data").mkdir()
    cfg = Config(brain_dir=_brain(tmp_path), data_dir=tmp_path / "data")
    app = create_app(
        config=cfg, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )
    client = TestClient(app)
    resp = client.get("/api/brain/search", params={"q": "auth flow"})
    assert resp.status_code == 200
    files = {r["file"] for r in resp.json()["results"]}
    assert "projects/login.md" in files


def test_boot_index_failure_degrades_silently(tmp_path, stub_provider, monkeypatch):
    """A build_index failure at boot must not kill create_app — it flips
    app.state.search_index False and search still works via the per-query
    walk (registry miss)."""
    import kluris_runtime.search_fts as sfmod
    from kluris.pack.config import Config
    from kluris.pack.main import create_app

    monkeypatch.setattr(sfmod, "build_index", _boom)
    (tmp_path / "data").mkdir()
    brain = _brain(tmp_path)
    cfg = Config(brain_dir=brain, data_dir=tmp_path / "data")
    app = create_app(
        config=cfg, provider=stub_provider,
        allow_writable_brain=True, skip_smoke_test=True,
    )
    assert app.state.search_index is False
    assert brain.resolve() not in sfmod._INDEX_REGISTRY
    # Per-query fallback still answers.
    assert any(
        r["file"] == "knowledge/sso.md"
        for r in search_brain_fts(brain, "authentication")
    )
