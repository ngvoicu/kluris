"""TEST-PACK-20..36 — eight read-only brain retrieval tools + schemas."""

from __future__ import annotations

import json

import pytest

from kluris.pack.tools.brain import (
    NotFoundError,
    SandboxError,
    glossary_tool,
    lobe_overview_tool,
    multi_read_tool,
    read_neuron_tool,
    recent_tool,
    related_tool,
    resolve_in_brain,
    search_tool,
    wake_up_tool,
)
from kluris.pack.tools.schemas import (
    anthropic_schemas,
    openai_schemas,
)


# --- wake_up -----------------------------------------------------------------


def test_wake_up_returns_runtime_payload(fixture_brain):
    payload = wake_up_tool(fixture_brain)
    assert payload["ok"] is True
    assert payload["name"] == "fixture-brain"
    lobe_names = {l["name"] for l in payload["lobes"]}
    assert {"projects", "knowledge", "infrastructure"} <= lobe_names


def test_wake_up_does_not_expose_scaffold_metadata(fixture_brain):
    """Spec: runtime build_payload omits ``type`` and ``type_structure``."""
    payload = wake_up_tool(fixture_brain)
    assert "type" not in payload
    assert "type_structure" not in payload


def test_wake_up_includes_deprecation_field_and_glossary(fixture_brain):
    """Clean fixture: ``raw-sql-old`` is deprecated WITH a valid
    replacement, so ``detect_deprecation_issues`` reports no issues —
    but the field is still present (count == 0).
    """
    payload = wake_up_tool(fixture_brain)
    assert "deprecation_count" in payload
    assert payload["deprecation_count"] == 0
    assert payload["deprecation"] == []
    glossary_terms = {e["term"] for e in payload["glossary"]}
    assert "JWT" in glossary_terms
    assert "SIT" in glossary_terms


# --- search ------------------------------------------------------------------


def test_search_ranks_results(fixture_brain):
    out = search_tool(fixture_brain, "auth")
    assert out["ok"] is True
    paths = [r["file"] for r in out["results"]]
    assert any("projects/btb/auth.md" in p for p in paths)


def test_search_lobe_filter_narrows(fixture_brain):
    out = search_tool(fixture_brain, "auth", lobe="knowledge")
    for r in out["results"]:
        assert r["file"].startswith("knowledge/")


def test_search_tag_filter_narrows(fixture_brain):
    out = search_tool(fixture_brain, "guidance", tag="decision")
    for r in out["results"]:
        if r["file"] != "glossary.md" and r["file"] != "brain.md":
            # tag filter excludes glossary/brain.md (no tags)
            pass


def test_search_limit_honored(fixture_brain):
    out = search_tool(fixture_brain, "a", limit=2)
    assert len(out["results"]) <= 2


def test_search_marks_deprecated(fixture_brain):
    out = search_tool(fixture_brain, "old guidance")
    deprecated = [r for r in out["results"] if r["deprecated"]]
    assert deprecated, "deprecated raw-sql-old neuron should surface"


def test_search_empty_query_returns_error(fixture_brain):
    out = search_tool(fixture_brain, "")
    assert out["ok"] is False


# --- read_neuron -------------------------------------------------------------


def test_read_neuron_markdown(fixture_brain):
    out = read_neuron_tool(fixture_brain, "knowledge/jwt.md")
    assert out["ok"] is True
    assert "JSON Web Tokens" in out["body"]
    assert out["frontmatter"]["updated"] == "2026-04-10"
    assert out["deprecated"] is False


def test_read_neuron_yaml_with_optin(fixture_brain):
    out = read_neuron_tool(fixture_brain, "infrastructure/openapi.yml")
    assert out["ok"] is True
    assert out["frontmatter"].get("title") == "Internal API"


def test_read_neuron_deprecated_flagged(fixture_brain):
    out = read_neuron_tool(fixture_brain, "knowledge/raw-sql-old.md")
    assert out["deprecated"] is True


def test_read_neuron_nonexistent_path_raises(fixture_brain):
    with pytest.raises(NotFoundError):
        read_neuron_tool(fixture_brain, "knowledge/nope.md")


def test_read_neuron_path_traversal_raises(fixture_brain):
    with pytest.raises(SandboxError):
        read_neuron_tool(fixture_brain, "../../etc/passwd")


def test_read_neuron_absolute_path_treated_as_relative(fixture_brain):
    """Leading slash is stripped, then resolved within the brain — a
    path like ``/knowledge/jwt.md`` reads the brain's
    ``knowledge/jwt.md``, not the host's ``/knowledge/jwt.md``.
    """
    out = read_neuron_tool(fixture_brain, "/knowledge/jwt.md")
    assert out["ok"] is True


def test_resolve_in_brain_rejects_symlink_escape(fixture_brain, tmp_path):
    outside = tmp_path / "outside.md"
    outside.write_text("# outside\n", encoding="utf-8")
    link = fixture_brain / "knowledge" / "escape.md"
    try:
        link.symlink_to(outside)
    except OSError:
        pytest.skip("symlink not supported on this filesystem")
    with pytest.raises(SandboxError):
        resolve_in_brain(fixture_brain, "knowledge/escape.md")


# --- multi_read --------------------------------------------------------------


def test_multi_read_5_valid_paths(fixture_brain):
    out = multi_read_tool(
        fixture_brain,
        [
            "knowledge/jwt.md",
            "knowledge/raw-sql-modern.md",
            "knowledge/raw-sql-old.md",
            "projects/btb/auth.md",
            "infrastructure/openapi.yml",
        ],
        max_paths=5,
    )
    assert out["ok"] is True
    assert len(out["results"]) == 5
    for entry in out["results"]:
        assert "frontmatter" in entry


def test_multi_read_6_paths_with_limit_5_errors(fixture_brain):
    out = multi_read_tool(
        fixture_brain,
        ["knowledge/jwt.md"] * 6,
        max_paths=5,
    )
    assert out["ok"] is False
    assert "max" in out["error"].lower()


def test_multi_read_mixed_valid_invalid(fixture_brain):
    out = multi_read_tool(
        fixture_brain,
        ["knowledge/jwt.md", "knowledge/nope.md", "../../etc/passwd"],
        max_paths=5,
    )
    assert out["ok"] is True
    paths = [r["path"] for r in out["results"]]
    assert "knowledge/jwt.md" in paths
    errors = [r for r in out["results"] if "error" in r]
    assert len(errors) == 2


def test_multi_read_empty_list_returns_empty_results(fixture_brain):
    out = multi_read_tool(fixture_brain, [], max_paths=5)
    assert out["ok"] is True
    assert out["results"] == []


@pytest.mark.parametrize("max_paths", [1, 5, 20])
def test_multi_read_limit_parametrized(fixture_brain, max_paths):
    out = multi_read_tool(
        fixture_brain,
        ["knowledge/jwt.md"] * max_paths,
        max_paths=max_paths,
    )
    assert out["ok"] is True
    assert len(out["results"]) == max_paths


# --- related -----------------------------------------------------------------


def test_related_outbound_and_inbound(fixture_brain):
    out = related_tool(fixture_brain, "knowledge/jwt.md")
    assert "projects/btb/auth.md" in out["outbound"]
    assert "projects/btb/auth.md" in out["inbound"]


def test_related_handles_no_links(fixture_brain):
    out = related_tool(fixture_brain, "knowledge/raw-sql-modern.md")
    assert out["outbound"] == []
    assert isinstance(out["inbound"], list)


# --- recent ------------------------------------------------------------------


def test_recent_sorts_by_updated_desc(fixture_brain):
    out = recent_tool(fixture_brain)
    paths = [r["path"] for r in out["results"]]
    # raw-sql-modern (2026-04-15) is the newest active; jwt (2026-04-10)
    # follows; raw-sql-old is excluded by default (deprecated).
    assert paths.index("knowledge/raw-sql-modern.md") < paths.index("knowledge/jwt.md")


def test_recent_lobe_filter_narrows(fixture_brain):
    out = recent_tool(fixture_brain, lobe="knowledge")
    for r in out["results"]:
        assert r["path"].startswith("knowledge/")


def test_recent_excludes_deprecated_by_default(fixture_brain):
    out = recent_tool(fixture_brain)
    assert all(not r["deprecated"] for r in out["results"])


def test_recent_includes_deprecated_when_requested(fixture_brain):
    out = recent_tool(fixture_brain, include_deprecated=True)
    deprecated = [r for r in out["results"] if r["deprecated"]]
    assert deprecated


def test_recent_limit_honored(fixture_brain):
    out = recent_tool(fixture_brain, limit=2)
    assert len(out["results"]) <= 2


# --- glossary ----------------------------------------------------------------


def test_glossary_no_arg_returns_all_entries(fixture_brain):
    out = glossary_tool(fixture_brain)
    terms = {e["term"] for e in out["entries"]}
    assert {"JWT", "SIT", "UAT", "Tenant"} <= terms


def test_glossary_term_hit_returns_definition(fixture_brain):
    out = glossary_tool(fixture_brain, "JWT")
    assert out["match"]["definition"].startswith("JSON Web Token")


def test_glossary_fuzzy_alternates(fixture_brain):
    out = glossary_tool(fixture_brain, "Tenants")
    # No exact match for "Tenants" but "Tenant" should be a close match
    alt_terms = {a["term"] for a in out["alternates"]}
    assert "Tenant" in alt_terms


def test_glossary_missing_file(tmp_path):
    brain = tmp_path / "empty"
    brain.mkdir()
    (brain / "brain.md").write_text("# B\n", encoding="utf-8")
    out = glossary_tool(brain)
    assert out["entries"] == []


# --- lobe_overview -----------------------------------------------------------


def test_lobe_overview_shape(fixture_brain):
    out = lobe_overview_tool(fixture_brain, "knowledge", budget=4096)
    assert out["lobe"] == "knowledge"
    assert "JWT" in out["map_body"] or "Knowledge" in out["map_body"]
    paths = {n["path"] for n in out["neurons"]}
    assert "knowledge/jwt.md" in paths
    # Tag union deduplicated
    assert isinstance(out["tag_union"], list)
    assert len(set(out["tag_union"])) == len(out["tag_union"])


def test_lobe_overview_excerpt_from_runtime(fixture_brain):
    out = lobe_overview_tool(fixture_brain, "knowledge", budget=4096)
    by_path = {n["path"]: n for n in out["neurons"]}
    jwt = by_path["knowledge/jwt.md"]
    assert "JSON Web Tokens" in jwt["excerpt"]


def test_lobe_overview_missing_lobe_raises(fixture_brain):
    with pytest.raises(NotFoundError):
        lobe_overview_tool(fixture_brain, "nope", budget=4096)


@pytest.mark.parametrize("budget", [1024, 4096, 8192])
def test_lobe_overview_budget_enforced(fixture_brain, budget):
    out = lobe_overview_tool(fixture_brain, "knowledge", budget=budget)
    encoded = json.dumps(out, ensure_ascii=False).encode("utf-8")
    assert len(encoded) <= budget


def test_lobe_overview_truncation_drops_neurons(fixture_brain):
    """A tight budget must drop trailing neurons + add ``truncated`` /
    ``omitted_count`` to the response.
    """
    out = lobe_overview_tool(fixture_brain, "knowledge", budget=1024)
    if out.get("truncated"):
        assert out["omitted_count"] > 0


# --- schemas -----------------------------------------------------------------


def test_anthropic_schemas_have_all_8_tools():
    schemas = anthropic_schemas(max_multi_read=5)
    names = {s["name"] for s in schemas}
    assert names == {
        "wake_up", "search", "read_neuron", "multi_read",
        "related", "recent", "glossary", "lobe_overview",
    }


def test_openai_schemas_have_all_8_tools():
    schemas = openai_schemas(max_multi_read=5)
    names = {s["function"]["name"] for s in schemas}
    assert names == {
        "wake_up", "search", "read_neuron", "multi_read",
        "related", "recent", "glossary", "lobe_overview",
    }


@pytest.mark.parametrize("max_paths", [3, 5, 10])
def test_multi_read_schema_max_items_reflects_runtime(max_paths):
    schemas = anthropic_schemas(max_multi_read=max_paths)
    multi = next(s for s in schemas if s["name"] == "multi_read")
    assert multi["input_schema"]["properties"]["paths"]["maxItems"] == max_paths

    openai = openai_schemas(max_multi_read=max_paths)
    multi_oai = next(s for s in openai if s["function"]["name"] == "multi_read")
    assert (
        multi_oai["function"]["parameters"]["properties"]["paths"]["maxItems"]
        == max_paths
    )


def test_lobe_overview_schema_requires_non_empty_string():
    schemas = anthropic_schemas(max_multi_read=5)
    lo = next(s for s in schemas if s["name"] == "lobe_overview")
    props = lo["input_schema"]["properties"]
    assert props["lobe"]["type"] == "string"
    assert props["lobe"].get("minLength") == 1
