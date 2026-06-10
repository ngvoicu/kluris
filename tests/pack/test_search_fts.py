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


# --- persistent on-disk index (pack-built, runtime-served) --------------------


def _build_db(brain, db_path):
    """Pack-side index build (the runtime is write-free), from the cached
    rows when registered, else a fresh walk."""
    from kluris.pack.search_index import build_search_db
    from kluris_runtime.search import collect_searchable

    rows = sf._INDEX_REGISTRY.get(brain.resolve())
    if rows is None:
        rows = collect_searchable(brain)
    return build_search_db(brain, rows, db_path)


@requires_fts
def test_on_disk_index_matches_in_memory_unfiltered(tmp_path):
    """The persistent index serves unfiltered queries byte-identical to the
    per-query in-memory build — scores included."""
    brain = _brain(tmp_path)
    baseline = search_brain_fts(brain, "authentication", limit=10)
    multiword = search_brain_fts(brain, "auth flow", limit=10)
    sf.build_index(brain)
    assert _build_db(brain, tmp_path / "cache" / "fts.sqlite")
    assert brain.resolve() in sf._DB_REGISTRY
    assert search_brain_fts(brain, "authentication", limit=10) == baseline
    assert search_brain_fts(brain, "auth flow", limit=10) == multiword


@requires_fts
def test_on_disk_index_filtered_queries_keep_subset_idf(tmp_path):
    """Lobe/tag-filtered queries must bypass the persistent index and keep
    the subset rebuild, so BM25 IDF stays scoped to the eligible rows."""
    brain = _brain(tmp_path)
    base_lobe = search_brain_fts(brain, "authentication", lobe_filter="knowledge")
    base_tag = search_brain_fts(brain, "guidance", tag_filter="decision")
    sf.build_index(brain)
    assert _build_db(brain, tmp_path / "fts.sqlite")
    assert search_brain_fts(brain, "authentication", lobe_filter="knowledge") == base_lobe
    assert search_brain_fts(brain, "guidance", tag_filter="decision") == base_tag


@requires_fts
def test_on_disk_index_skips_per_query_rebuild(tmp_path, monkeypatch):
    """With the persistent index registered, an unfiltered query must NOT
    build an in-memory table — the large-brain hot-path win."""
    brain = _brain(tmp_path)
    sf.build_index(brain)
    assert _build_db(brain, tmp_path / "fts.sqlite")

    def _no_rebuild(items):
        raise AssertionError("per-query FTS table was rebuilt")

    monkeypatch.setattr(sf, "_build_fts_table", _no_rebuild)
    hits = search_brain_fts(brain, "authentication", limit=10)
    assert any(r["file"] == "knowledge/sso.md" for r in hits)


@requires_fts
def test_paged_search_window_and_total(tmp_path):
    """offset pages deterministically through the same ranked order, and
    total reports the full match count regardless of the window — identically
    on the persistent and in-memory paths."""
    brain = _brain(tmp_path)
    sf.build_index(brain)
    assert _build_db(brain, tmp_path / "fts.sqlite")
    full = sf.search_brain_fts_paged(brain, "auth old", limit=10)
    assert full["total"] == len(full["results"]) >= 3
    page = sf.search_brain_fts_paged(brain, "auth old", limit=1, offset=1)
    assert page["results"] == full["results"][1:2]
    assert page["total"] == full["total"]

    # In-memory path (no persistent index) pages identically.
    sf.drop_index(brain)
    assert sf.search_brain_fts_paged(brain, "auth old", limit=1, offset=1) == page


@requires_fts
def test_corrupt_on_disk_index_degrades_to_in_memory(tmp_path):
    """A corrupted index file must degrade to the in-memory build, never
    error out — search can only get slower, not broken."""
    brain = _brain(tmp_path)
    db = tmp_path / "fts.sqlite"
    sf.build_index(brain)
    assert _build_db(brain, db)
    db.write_bytes(b"this is not a sqlite database")
    hits = search_brain_fts(brain, "authentication", limit=10)
    assert any(r["file"] == "knowledge/sso.md" for r in hits)


@requires_fts
def test_build_index_accepts_snapshot_rows(tmp_path):
    """The boot snapshot's rows feed build_index directly (single boot walk):
    results match a walk-fed build exactly."""
    from kluris_runtime.snapshot import build_snapshot

    brain = _brain(tmp_path)
    baseline = search_brain_fts(brain, "authentication", limit=10)
    snap = build_snapshot(brain)
    sf.build_index(brain, rows=snap["rows"])
    assert _build_db(brain, tmp_path / "fts.sqlite")
    assert search_brain_fts(brain, "authentication", limit=10) == baseline


@requires_fts
def test_build_index_overwrites_stale_db_file(tmp_path):
    """Rebuilding over an existing db file serves the NEW corpus — the data
    volume outlives the image, so stale indexes must never survive a boot."""
    brain = _brain(tmp_path)
    db = tmp_path / "fts.sqlite"
    sf.build_index(brain)
    assert _build_db(brain, db)
    (brain / "knowledge" / "fresh.md").write_text(
        "# Fresh\n\nbrand new keycloak material.\n", encoding="utf-8"
    )
    sf.drop_index(brain)
    sf.build_index(brain)
    assert _build_db(brain, db)
    assert "knowledge/fresh.md" in {
        r["file"] for r in search_brain_fts(brain, "keycloak")
    }


# --- grouped per-lobe search (partitioned, not windowed) -----------------------


def _homogeneous_brain(tmp_path, lobes=6, per_lobe=40):
    """Every neuron matches the query — the shape where a flat top-N window
    bunches into one lobe and grouping must still cover all of them."""
    b = tmp_path / "homo-brain"
    for li in range(lobes):
        d = b / f"lobe-{li:02d}"
        d.mkdir(parents=True)
        for ni in range(per_lobe):
            (d / f"doc-{li}-{ni}.md").write_text(
                f"# Doc {li}-{ni}\n\nfee rate details entry {ni}.\n",
                encoding="utf-8",
            )
    return b


@requires_fts
def test_grouped_search_covers_every_lobe_on_homogeneous_corpus(tmp_path):
    brain = _homogeneous_brain(tmp_path)
    out = sf.search_brain_fts_grouped(brain, "fee rate", per_lobe=2)
    assert len(out["lobes"]) == 6
    assert all(len(hits) == 2 for hits in out["lobes"].values())
    assert out["total"] == 240


@requires_fts
def test_grouped_search_identical_coverage_on_persistent_index(tmp_path):
    brain = _homogeneous_brain(tmp_path)
    sf.build_index(brain)
    assert _build_db(brain, tmp_path / "fts.sqlite")
    out = sf.search_brain_fts_grouped(brain, "fee rate", per_lobe=2)
    assert len(out["lobes"]) == 6
    assert all(len(hits) == 2 for hits in out["lobes"].values())
    # Every hit belongs to the lobe it is bucketed under.
    for lobe, hits in out["lobes"].items():
        assert all(h["file"].startswith(lobe + "/") for h in hits)


@requires_fts
def test_grouped_search_through_search_tool(tmp_path):
    brain = _homogeneous_brain(tmp_path)
    out = search_tool(brain, "fee rate", limit=2, group_by_lobe=True)
    assert out["grouped_by_lobe"] is True
    assert len(out["lobes"]) == 6
    assert out["per_lobe_limit"] == 2


@requires_fts
def test_grouped_within_lobe_ranking_identical_on_both_paths(tmp_path):
    """Heterogeneous corpus: in-memory (no db) and on-disk grouped search must
    produce IDENTICAL within-lobe ranking, not just identical coverage — both
    use GLOBAL bm25 IDF."""
    brain = tmp_path / "het-brain"
    (brain / "lobe-a").mkdir(parents=True)
    (brain / "lobe-a" / "A.md").write_text("# A\n\nalpha distinctive\n", encoding="utf-8")
    (brain / "lobe-a" / "f0.md").write_text("# f0\n\nbeta beta common\n", encoding="utf-8")
    (brain / "lobe-a" / "f1.md").write_text("# f1\n\nbeta common\n", encoding="utf-8")
    (brain / "lobe-b").mkdir(parents=True)
    for i in range(40):
        (brain / "lobe-b" / f"x{i}.md").write_text(
            f"# x{i}\n\nalpha alpha entry\n", encoding="utf-8")

    sf.build_index(brain)
    mem = sf.search_brain_fts_grouped(brain, "alpha beta", per_lobe=2)

    assert _build_db(brain, tmp_path / "fts.sqlite")
    disk = sf.search_brain_fts_grouped(brain, "alpha beta", per_lobe=2)

    def files(g):
        return {k: [h["file"] for h in v] for k, v in g["lobes"].items()}

    assert files(mem) == files(disk)        # order included, not just membership
    assert mem["total"] == disk["total"]
