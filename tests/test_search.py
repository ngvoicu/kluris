"""Tests for kluris search command and core.search module."""

import json

import pytest
from click.testing import CliRunner

from kluris.core.search import _collect_searchable, search_brain


def _make_brain_with_yaml_neurons_search(tmp_path):
    """Yaml-neurons fixture for search tests."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text(
        "---\n---\n# Glossary\n", encoding="utf-8"
    )
    (brain / "kluris.yml").write_text(
        "name: brain\ntype: product\n", encoding="utf-8"
    )
    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nparent: ../brain.md\n---\n# Projects\n", encoding="utf-8"
    )
    (lobe / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\n"
        "updated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    (lobe / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# tags: [api, payments]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n  title: Payments API\n  version: 1.0.0\n"
        "paths:\n  /charge: {}\n",
        encoding="utf-8",
    )
    (lobe / "ci-config.yml").write_text(
        "name: ci\non: [push]\n", encoding="utf-8"
    )
    return brain


def _write(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _make_search_brain(tmp_path):
    """Brain with 2 neurons, 2 glossary terms, brain.md.

    - knowledge/use-raw-sql.md (active, tags include `auth`)
    - knowledge/old-decision.md (deprecated)
    - glossary.md with 2 terms in markdown-table format
    - brain.md with an H1 title and a body line
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "brain.md",
        "---\nauto_generated: true\nupdated: 2026-04-01\n---\n"
        "# My Brain\n\nA brain about authentication and OAuth.\n",
    )
    _write(
        brain / "glossary.md",
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n"
        "# Glossary\n\n"
        "| Term | Meaning |\n"
        "|------|---------|\n"
        "| OAuth | Open Authorization protocol for token-based access. |\n"
        "| JWT | JSON Web Token used for stateless auth. |\n",
    )
    _write(
        brain / "knowledge" / "map.md",
        "---\nauto_generated: true\nparent: ../brain.md\n"
        "updated: 2026-04-01\n---\n# Knowledge\n",
    )
    _write(
        brain / "knowledge" / "use-raw-sql.md",
        "---\nparent: ./map.md\n"
        "tags: [auth, sql]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Use Raw SQL\n\nWe chose raw SQL over JPA for query complexity.\n",
    )
    _write(
        brain / "knowledge" / "old-decision.md",
        "---\nparent: ./map.md\n"
        "status: deprecated\n"
        "deprecated_at: 2026-03-01\n"
        "tags: [legacy]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Old Decision\n\nThis is the old way we did things.\n",
    )
    return brain


def test_collect_searchable_returns_neurons_glossary_brain_md(tmp_path):
    """_collect_searchable returns 5 items: 2 neurons + 2 glossary terms + 1 brain.md."""
    from kluris.core.search import _collect_searchable

    brain = _make_search_brain(tmp_path)
    items = _collect_searchable(brain)

    # 2 neurons + 2 glossary terms + 1 brain.md = 5 items
    assert len(items) == 5

    kinds = [item["kind"] for item in items]
    assert kinds.count("neuron") == 2
    assert kinds.count("glossary") == 2
    assert kinds.count("brain_md") == 1

    # Each item must have the documented shape
    for item in items:
        assert "kind" in item
        assert "file" in item
        assert "title" in item
        assert "tags" in item
        assert isinstance(item["tags"], list)
        assert "body" in item
        assert "is_deprecated" in item
        assert isinstance(item["is_deprecated"], bool)

    # Find each item by file
    by_file = {item["file"]: item for item in items}

    # Active neuron
    raw_sql = by_file["knowledge/use-raw-sql.md"]
    assert raw_sql["kind"] == "neuron"
    assert raw_sql["title"] == "Use Raw SQL"
    assert "auth" in raw_sql["tags"]
    assert "raw SQL" in raw_sql["body"]
    assert raw_sql["is_deprecated"] is False

    # Deprecated neuron
    old = by_file["knowledge/old-decision.md"]
    assert old["kind"] == "neuron"
    assert old["title"] == "Old Decision"
    assert old["is_deprecated"] is True

    # Glossary terms (one item per term)
    oauth = by_file.get("glossary.md#oauth") or by_file.get("glossary.md")
    # The collector may use either a single composite key or two separate items;
    # we expect two separate items keyed by file+term. Let's discover both.
    glossary_items = [item for item in items if item["kind"] == "glossary"]
    glossary_terms = {item["title"]: item for item in glossary_items}
    assert "OAuth" in glossary_terms
    assert "JWT" in glossary_terms
    assert "Open Authorization" in glossary_terms["OAuth"]["body"]
    assert "JSON Web Token" in glossary_terms["JWT"]["body"]
    for g in glossary_items:
        assert g["tags"] == []
        assert g["is_deprecated"] is False
        assert g["file"] == "glossary.md"

    # brain.md item
    brain_md_items = [item for item in items if item["kind"] == "brain_md"]
    assert len(brain_md_items) == 1
    bm = brain_md_items[0]
    assert bm["title"] == "My Brain"
    assert "authentication and OAuth" in bm["body"]
    assert bm["tags"] == []
    assert bm["is_deprecated"] is False
    assert bm["file"] == "brain.md"


# --- Scoring + matched_fields ---


def _item(file="x.md", title="", tags=None, body=""):
    """Build a minimal searchable item dict for scoring tests."""
    return {
        "kind": "neuron",
        "file": file,
        "title": title,
        "tags": tags or [],
        "body": body,
        "is_deprecated": False,
    }


def test_score_hit_uses_occurrence_counts():
    """Scoring is occurrence-count based, weighted per field.

    Formula: title*10 + tag*5 + path*3 + body*1 (where each * is the
    number of times the query appears in that field).
    """
    from kluris.core.search import _score_hit

    # Title hit twice -> 2 * 10 = 20
    item = _item(title="oauth oauth flow")
    assert _score_hit(item, "oauth") == 20

    # Body hit three times -> 3 * 1 = 3
    item = _item(body="oauth and oauth then oauth")
    assert _score_hit(item, "oauth") == 3

    # Single tag hit -> 1 * 5 = 5
    item = _item(tags=["oauth", "auth"])
    assert _score_hit(item, "oauth") == 5

    # Path hit once -> 1 * 3 = 3
    item = _item(file="projects/oauth/notes.md")
    assert _score_hit(item, "oauth") == 3

    # Combined: title 1x + body 2x = 10 + 2 = 12
    item = _item(title="OAuth flow", body="oauth here, oauth there")
    assert _score_hit(item, "oauth") == 12

    # Zero matches -> 0
    item = _item(title="something else", body="nothing here")
    assert _score_hit(item, "oauth") == 0

    # Case-insensitive: query "oauth" matches "OAuth" in body
    item = _item(body="OAuth IS great")
    assert _score_hit(item, "oauth") == 1


def test_matched_fields_lists_every_hit_field_in_order():
    """_matched_fields returns the list of field names with non-zero hits,
    in canonical order: title, tag, path, body."""
    from kluris.core.search import _matched_fields

    # Title + body
    item = _item(title="oauth flow", body="oauth here")
    assert _matched_fields(item, "oauth") == ["title", "body"]

    # Tag only
    item = _item(tags=["oauth"])
    assert _matched_fields(item, "oauth") == ["tag"]

    # All four fields
    item = _item(
        file="projects/oauth/x.md",
        title="oauth title",
        tags=["oauth"],
        body="oauth body",
    )
    assert _matched_fields(item, "oauth") == ["title", "tag", "path", "body"]

    # No matches -> empty list
    item = _item(title="other")
    assert _matched_fields(item, "oauth") == []


def test_score_does_not_use_regex():
    """Scoring must treat regex special characters as literal substrings."""
    from kluris.core.search import _score_hit

    # The query "a.b*?" should be treated literally; if regex were used,
    # `.` would match any char and `*?` would be a quantifier, producing
    # different results. Test the literal interpretation.
    item = _item(body="contains a.b*? somewhere")
    assert _score_hit(item, "a.b*?") == 1

    # If regex were used, "a.b" would also match "axb" — make sure it doesn't.
    item = _item(body="contains axb somewhere")
    assert _score_hit(item, "a.b") == 0


# --- Snippet extraction ---


def test_extract_snippet_centers_on_first_match_in_long_body():
    """A 1000-char body with the query at position 500 returns a slice
    centered on the match, with leading/trailing ellipses."""
    from kluris.core.search import _extract_snippet

    # Build a body that's exactly 1000 chars; put "QUERY" at position 500.
    body = ("a" * 500) + "QUERY" + ("b" * 495)
    snippet = _extract_snippet(body, "query", width=200)

    # Match found, so snippet must contain it (case-insensitive: original case preserved)
    assert "QUERY" in snippet
    # Slice is approximately 200 chars wide plus possible ellipsis markers
    assert len(snippet) <= 200 + 6  # 6 = "..." prefix + "..." suffix
    # Both ends are truncated, so both ellipsis markers should appear
    assert snippet.startswith("...")
    assert snippet.endswith("...")


def test_extract_snippet_at_start_no_left_ellipsis():
    """Match at position 0 means the slice already starts at 0; no '...' prefix."""
    from kluris.core.search import _extract_snippet

    body = "QUERY" + ("z" * 500)
    snippet = _extract_snippet(body, "query", width=200)
    assert snippet.startswith("QUERY")
    assert not snippet.startswith("...")
    # Right side is truncated
    assert snippet.endswith("...")


def test_extract_snippet_at_end_no_right_ellipsis():
    """Match near the end means the slice reaches the end; no '...' suffix."""
    from kluris.core.search import _extract_snippet

    body = ("z" * 500) + "QUERY"
    snippet = _extract_snippet(body, "query", width=200)
    assert snippet.endswith("QUERY")
    assert not snippet.endswith("...")
    assert snippet.startswith("...")


def test_extract_snippet_no_match_returns_empty():
    """If the query is not in the body, the snippet is an empty string."""
    from kluris.core.search import _extract_snippet

    snippet = _extract_snippet("nothing relevant here", "oauth", width=200)
    assert snippet == ""


def test_extract_snippet_short_body_no_truncation():
    """If the body is shorter than the snippet width, return the whole thing."""
    from kluris.core.search import _extract_snippet

    body = "We chose oauth for token-based access."
    snippet = _extract_snippet(body, "oauth", width=200)
    assert snippet == body
    assert "..." not in snippet


def test_extract_snippet_preserves_utf8_characters():
    """Snippet slicing must operate on Unicode code points (str), not bytes,
    so multi-byte characters are not split."""
    from kluris.core.search import _extract_snippet

    # Build a body with non-ASCII characters around the query
    body = ("café résumé naïve " * 30) + "QUERY" + (" über schön" * 30)
    snippet = _extract_snippet(body, "query", width=200)
    # The snippet must encode round-trip without errors
    assert snippet.encode("utf-8").decode("utf-8") == snippet
    # And the query must still be in there
    assert "QUERY" in snippet
    # And at least one non-ASCII char should still be present (proves we didn't strip them)
    assert any(ord(c) > 127 for c in snippet)


# --- search_brain core (no filters, no deprecation flag) ---


def _make_ranking_brain(tmp_path):
    """Brain with 5 neurons matching `oauth` in different fields:

    - A: title hit (1x in title) → score 10
    - B: tag hit (1x in tags) → score 5
    - C: path hit (1x in path) → score 3
    - D: body hit (2x in body) → score 2
    - E: no match → excluded
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "knowledge" / "map.md",
        "---\nauto_generated: true\n---\n# K\n",
    )

    # A — title hit
    _write(
        brain / "knowledge" / "a.md",
        "---\nparent: ./map.md\ntags: [auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# OAuth flow\n\nA flow doc.\n",
    )
    # B — tag hit
    _write(
        brain / "knowledge" / "b.md",
        "---\nparent: ./map.md\ntags: [oauth, auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Token storage\n\nWhere we keep tokens.\n",
    )
    # C — path hit (file lives under a path that contains the query)
    _write(
        brain / "knowledge" / "oauth-notes.md",
        "---\nparent: ./map.md\ntags: [auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Notes\n\nNotes about tokens.\n",
    )
    # D — body hit twice
    _write(
        brain / "knowledge" / "d.md",
        "---\nparent: ./map.md\ntags: [auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Auth doc\n\nWe use oauth here. Then more oauth.\n",
    )
    # E — no match
    _write(
        brain / "knowledge" / "e.md",
        "---\nparent: ./map.md\ntags: [other]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Unrelated\n\nNothing about auth here.\n",
    )
    return brain


def test_search_brain_basic_ranking_and_limit(tmp_path):
    """search_brain returns ranked results, excludes zero-score, applies limit.

    Note: `read_frontmatter` returns the body INCLUDING the H1 title line,
    so a neuron with the query in its title also gets a body hit. The
    scoring formula handles this naturally — title hits dominate (10x)
    and the body hit adds a small bonus.
    """
    from kluris.core.search import search_brain

    brain = _make_ranking_brain(tmp_path)
    results = search_brain(brain, "oauth", limit=10)

    # 4 results (E excluded), score order: A(11) > B(5) > C(3) > D(2)
    # A: title 1x = 10, body 1x (from H1 line) = 1, total 11
    # B: tag 1x = 5
    # C: path 1x = 3
    # D: body 2x = 2
    assert len(results) == 4
    files = [r["file"] for r in results]
    assert files == [
        "knowledge/a.md",
        "knowledge/b.md",
        "knowledge/oauth-notes.md",
        "knowledge/d.md",
    ]

    # Each result has the documented shape (no `deprecated` field yet — that comes in IMPL-SCH-12)
    for r in results:
        assert "file" in r
        assert "title" in r
        assert "matched_fields" in r
        assert "snippet" in r
        assert "score" in r
        assert isinstance(r["matched_fields"], list)
        assert len(r["matched_fields"]) >= 1

    # Scores
    by_file = {r["file"]: r for r in results}
    assert by_file["knowledge/a.md"]["score"] == 11  # title 10 + body 1 (H1 line)
    assert by_file["knowledge/b.md"]["score"] == 5
    assert by_file["knowledge/oauth-notes.md"]["score"] == 3
    assert by_file["knowledge/d.md"]["score"] == 2

    # Snippets: A and D both have body matches, so both have non-empty snippets.
    # A's body match is from the H1 line; D's is from the prose.
    assert by_file["knowledge/d.md"]["snippet"] != ""
    assert "oauth" in by_file["knowledge/d.md"]["snippet"].lower()
    assert by_file["knowledge/a.md"]["snippet"] != ""
    # Tag-only / path-only matches have empty snippets
    assert by_file["knowledge/b.md"]["snippet"] == ""
    assert by_file["knowledge/oauth-notes.md"]["snippet"] == ""

    # matched_fields reflects the field of the hit
    assert "title" in by_file["knowledge/a.md"]["matched_fields"]
    assert "body" in by_file["knowledge/a.md"]["matched_fields"]  # H1 also matches body
    assert by_file["knowledge/b.md"]["matched_fields"] == ["tag"]
    assert by_file["knowledge/oauth-notes.md"]["matched_fields"] == ["path"]
    assert by_file["knowledge/d.md"]["matched_fields"] == ["body"]


def test_search_brain_limit_truncates_results(tmp_path):
    """`limit=2` returns only the top 2 results."""
    from kluris.core.search import search_brain

    brain = _make_ranking_brain(tmp_path)
    results = search_brain(brain, "oauth", limit=2)
    assert len(results) == 2
    assert [r["file"] for r in results] == ["knowledge/a.md", "knowledge/b.md"]


def test_search_brain_limit_zero_returns_empty(tmp_path):
    """`limit=0` returns an empty list (still no errors)."""
    from kluris.core.search import search_brain

    brain = _make_ranking_brain(tmp_path)
    assert search_brain(brain, "oauth", limit=0) == []


# --- search_brain filters (lobe, tag) ---


def _make_filter_brain(tmp_path):
    """4 neurons split across two lobes with overlapping tags. All match `x`."""
    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "projects" / "map.md",
        "---\nauto_generated: true\n---\n# P\n",
    )
    _write(
        brain / "knowledge" / "map.md",
        "---\nauto_generated: true\n---\n# K\n",
    )

    _write(
        brain / "projects" / "p1.md",
        "---\nparent: ./map.md\ntags: [oauth, auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# x project 1\n\nx body\n",
    )
    _write(
        brain / "projects" / "p2.md",
        "---\nparent: ./map.md\ntags: [auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# x project 2\n\nx body\n",
    )
    _write(
        brain / "knowledge" / "k1.md",
        "---\nparent: ./map.md\ntags: [oauth, sql]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# x knowledge 1\n\nx body\n",
    )
    _write(
        brain / "knowledge" / "k2.md",
        "---\nparent: ./map.md\ntags: [legacy]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# x knowledge 2\n\nx body\n",
    )
    return brain


def test_search_brain_filter_by_lobe(tmp_path):
    """`lobe_filter='projects'` keeps only neurons under `projects/`."""
    from kluris.core.search import search_brain

    brain = _make_filter_brain(tmp_path)
    results = search_brain(brain, "x", lobe_filter="projects", limit=10)
    files = sorted(r["file"] for r in results)
    assert files == ["projects/p1.md", "projects/p2.md"]


def test_search_brain_filter_by_tag(tmp_path):
    """`tag_filter='oauth'` keeps only neurons whose `tags:` includes oauth."""
    from kluris.core.search import search_brain

    brain = _make_filter_brain(tmp_path)
    results = search_brain(brain, "x", tag_filter="oauth", limit=10)
    files = sorted(r["file"] for r in results)
    # p1 and k1 have oauth in their tags; p2 and k2 do not
    assert files == ["knowledge/k1.md", "projects/p1.md"]


def test_search_brain_filters_combined_and(tmp_path):
    """Both filters together AND — only neurons that pass both are kept."""
    from kluris.core.search import search_brain

    brain = _make_filter_brain(tmp_path)
    results = search_brain(
        brain, "x", lobe_filter="projects", tag_filter="oauth", limit=10
    )
    files = sorted(r["file"] for r in results)
    # Only p1 is in projects/ AND tagged oauth
    assert files == ["projects/p1.md"]


def test_search_brain_lobe_filter_excludes_glossary_and_brain_md(tmp_path):
    """Glossary and brain.md items are at brain root; a lobe_filter excludes them."""
    from kluris.core.search import search_brain

    # Build a brain with one neuron under projects/, one glossary term, and brain.md
    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "brain.md",
        "---\nauto_generated: true\n---\n# Brain\n\nbrain.md mentions x.\n",
    )
    _write(
        brain / "glossary.md",
        "---\nauto_generated: false\n---\n# G\n\n"
        "| Term | Meaning |\n|------|---------|\n"
        "| X | something about x |\n",
    )
    _write(
        brain / "projects" / "map.md",
        "---\nauto_generated: true\n---\n# P\n",
    )
    _write(
        brain / "projects" / "n.md",
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# x neuron\n\nx body\n",
    )

    # No filter: all 3 results (neuron, glossary, brain.md)
    all_results = search_brain(brain, "x", limit=10)
    files = {r["file"] for r in all_results}
    assert "projects/n.md" in files
    assert "glossary.md" in files
    assert "brain.md" in files

    # With lobe_filter: only the neuron survives
    filtered = search_brain(brain, "x", lobe_filter="projects", limit=10)
    files = {r["file"] for r in filtered}
    assert files == {"projects/n.md"}

    # With tag_filter: glossary/brain.md have no tags so they're filtered out;
    # the neuron has empty tags too, so nothing matches
    tag_filtered = search_brain(brain, "x", tag_filter="oauth", limit=10)
    assert tag_filtered == []


# --- search_brain deprecated flag ---


def test_search_brain_marks_deprecated_neurons(tmp_path):
    """Each result has a `deprecated` boolean derived from frontmatter `status`."""
    from kluris.core.search import search_brain

    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "knowledge" / "map.md",
        "---\nauto_generated: true\n---\n# K\n",
    )
    # Active neuron
    _write(
        brain / "knowledge" / "active.md",
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Active\n\nactive query content\n",
    )
    # Deprecated neuron with valid replaced_by and NO incoming links —
    # detect_deprecation_issues would NOT report this as an issue, so a
    # naive deprecation check via that helper would miss it. The spec
    # explicitly requires reading frontmatter directly during collection.
    _write(
        brain / "knowledge" / "replacement.md",
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Replacement\n\nreplacement content\n",
    )
    _write(
        brain / "knowledge" / "old.md",
        "---\nparent: ./map.md\nstatus: deprecated\n"
        "deprecated_at: 2026-03-01\n"
        "replaced_by: ./replacement.md\n"
        "tags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Old\n\nold query content\n",
    )

    results = search_brain(brain, "query", limit=10)

    by_file = {r["file"]: r for r in results}

    # Both active and deprecated neurons appear
    assert "knowledge/active.md" in by_file
    assert "knowledge/old.md" in by_file

    # Each has a `deprecated` field
    assert by_file["knowledge/active.md"]["deprecated"] is False
    assert by_file["knowledge/old.md"]["deprecated"] is True


def test_search_brain_glossary_and_brain_md_are_never_deprecated(tmp_path):
    """Glossary and brain.md results have `deprecated: false`."""
    from kluris.core.search import search_brain

    brain = tmp_path / "brain"
    brain.mkdir()
    _write(
        brain / "brain.md",
        "---\nauto_generated: true\n---\n# Brain\n\ntoken content here\n",
    )
    _write(
        brain / "glossary.md",
        "---\nauto_generated: false\n---\n# G\n\n"
        "| Term | Meaning |\n|------|---------|\n"
        "| Token | a token thing |\n",
    )

    results = search_brain(brain, "token", limit=10)
    assert len(results) >= 2  # at least glossary + brain.md
    for r in results:
        assert "deprecated" in r
        assert r["deprecated"] is False


# --- CLI command (JSON, picker, --brain all rejection) ---


def _make_cli_brain(runner, name, tmp_path):
    """Use the existing create_test_brain helper to set up a brain via the CLI."""
    from conftest import create_test_brain
    create_test_brain(runner, name, tmp_path)


def test_search_cli_single_brain_json(tmp_path, monkeypatch):
    """`kluris search query --json` returns the documented JSON envelope shape."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    # Plant a neuron with a known body
    neuron = tmp_path / "my-brain" / "projects" / "auth.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: [auth]\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Auth\n\nWe use oauth here.\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["search", "oauth", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)

    # Top-level envelope
    assert data["ok"] is True
    assert data["brain"] == "my-brain"
    assert data["query"] == "oauth"
    assert isinstance(data["total"], int)
    assert isinstance(data["results"], list)

    # Result entries (at least 1: the auth neuron)
    assert data["total"] >= 1
    found = next((r for r in data["results"] if r["file"] == "projects/auth.md"), None)
    assert found is not None
    assert "title" in found
    assert "matched_fields" in found
    assert "snippet" in found
    assert "score" in found
    assert "deprecated" in found


def test_search_cli_multi_brain_picker_via_is_interactive(tmp_path, monkeypatch):
    """When 2+ brains are registered and the user is interactive,
    the picker prompts. Tests must monkeypatch `kluris.cli._is_interactive`,
    NOT `sys.stdin.isatty` (CliRunner replaces stdin during invoke)."""
    import kluris.cli as cli_module
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    _make_cli_brain(runner, "brain-a", tmp_path)
    _make_cli_brain(runner, "brain-b", tmp_path)

    # Note: do NOT pass --json. The resolver forces non-interactive mode
    # when --json is set (to keep the JSON envelope clean), so the picker
    # would never fire under --json + multi-brain.
    result = runner.invoke(cli, ["search", "oauth"], input="1\n")
    assert "[1] brain-a" in result.output
    assert "[2] brain-b" in result.output


def test_search_cli_brain_all_rejected(tmp_path, monkeypatch):
    """`--brain all` is rejected because search is single-brain (allow_all=False)."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "brain-a", tmp_path)
    _make_cli_brain(runner, "brain-b", tmp_path)

    result = runner.invoke(cli, ["search", "oauth", "--brain", "all"])
    assert result.exit_code != 0
    assert "all" in result.output
    # The error should mention which commands DO accept --brain all
    assert "dream" in result.output or "fan" in result.output.lower()


# --- CLI edge cases (empty, no match, special chars, non-ASCII) ---


def test_search_empty_query_errors(tmp_path, monkeypatch):
    """An empty-string query is rejected with a clear error."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["search", ""])
    assert result.exit_code != 0
    assert "empty" in result.output.lower()


def test_search_no_matches_returns_empty_results(tmp_path, monkeypatch):
    """A query with zero matches returns total: 0, results: [], exit 0."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["search", "absolutelynothingmatchesthis", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["total"] == 0
    assert data["results"] == []


def test_search_special_characters_treated_literally(tmp_path, monkeypatch):
    """Regex special chars (`.`, `*`, `?`) in the query are treated literally,
    not as regex metacharacters. This is the regression test for Codex finding #8."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    # Plant a neuron with the literal string `a.b*?` in its body
    neuron = tmp_path / "my-brain" / "knowledge" / "weird.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Weird syntax\n\nThe pattern a.b*? is sometimes used.\n",
        encoding="utf-8",
    )
    # And a decoy neuron with `axbcd` (would match `a.b` if regex were used)
    decoy = tmp_path / "my-brain" / "knowledge" / "decoy.md"
    decoy.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Decoy\n\nThis has axbcd in the body.\n",
        encoding="utf-8",
    )

    # Searching for the literal `a.b*?` finds weird.md ONLY
    result = runner.invoke(cli, ["search", "a.b*?", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    files = [r["file"] for r in data["results"]]
    assert "knowledge/weird.md" in files
    assert "knowledge/decoy.md" not in files

    # Searching for the literal `a.b` should NOT match `axb` (the decoy)
    result2 = runner.invoke(cli, ["search", "a.b", "--json"])
    assert result2.exit_code == 0
    data2 = json.loads(result2.output)
    files2 = [r["file"] for r in data2["results"]]
    # weird.md contains "a.b" literally, so it should match
    assert "knowledge/weird.md" in files2
    # decoy.md only has "axb", which would match `a.b` as a regex but NOT as a literal
    assert "knowledge/decoy.md" not in files2


def test_search_non_ascii_query(tmp_path, monkeypatch):
    """UTF-8 queries match UTF-8 content correctly."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    # Plant a neuron with non-ASCII characters
    neuron = tmp_path / "my-brain" / "knowledge" / "french.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Naïve approach\n\n"
        "Le café au bureau est résumé en une phrase.\n",
        encoding="utf-8",
    )

    # Search for an accented query
    result = runner.invoke(cli, ["search", "café", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    files = [r["file"] for r in data["results"]]
    assert "knowledge/french.md" in files

    # The snippet should preserve the non-ASCII characters
    found = next(r for r in data["results"] if r["file"] == "knowledge/french.md")
    assert "café" in found["snippet"]


# --- Text output + help registration ---


def test_search_text_output_includes_score_title_file_snippet(tmp_path, monkeypatch):
    """Non-JSON invocation renders a compact table with score, title, file, snippet."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    # Plant 2 matching neurons
    n1 = tmp_path / "my-brain" / "knowledge" / "first.md"
    n1.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# OAuth design\n\nWe chose oauth flow A.\n",
        encoding="utf-8",
    )
    n2 = tmp_path / "my-brain" / "knowledge" / "second.md"
    n2.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Other doc\n\noauth comes up here too\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["search", "oauth"])
    assert result.exit_code == 0
    # The output should mention each result's title and file
    assert "OAuth design" in result.output
    assert "knowledge/first.md" in result.output
    assert "Other doc" in result.output
    assert "knowledge/second.md" in result.output
    # And at least one snippet
    assert "oauth" in result.output.lower()


def test_search_text_output_no_results_message(tmp_path, monkeypatch):
    """Zero results in text mode prints a friendly 'no results' line, not nothing."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _make_cli_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["search", "absolutelynothingmatches"])
    assert result.exit_code == 0
    assert "no results" in result.output.lower() or "0 results" in result.output.lower()


def test_search_command_listed_in_help(tmp_path, monkeypatch):
    """`kluris help --json` should include 'search' as a registered command."""
    from kluris.cli import cli

    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()

    result = runner.invoke(cli, ["help", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    names = {c["name"] for c in data["commands"]}
    assert "search" in names
    assert len(data["commands"]) == 17  # was 16 before search was added


# --- yaml-neurons search tests ---


def test_collect_searchable_includes_yaml_neurons(tmp_path):
    """_collect_searchable must emit an entry for each opted-in yaml neuron,
    with `file_type: 'yaml'` and title resolved from the frontmatter `title`
    field. Raw yaml files without a block must not appear.
    """
    brain = _make_brain_with_yaml_neurons_search(tmp_path)
    items = _collect_searchable(brain)
    by_file = {item["file"]: item for item in items}

    assert "projects/openapi.yml" in by_file
    yaml_item = by_file["projects/openapi.yml"]
    assert yaml_item.get("file_type") == "yaml"
    assert yaml_item["title"] == "Payments API"

    assert "projects/auth.md" in by_file
    md_item = by_file["projects/auth.md"]
    assert md_item.get("file_type") == "markdown"

    # Opt-out yaml must not appear
    assert "projects/ci-config.yml" not in by_file
    # Brain-root config must not appear
    assert "kluris.yml" not in by_file


def test_search_brain_returns_yaml_hits_with_file_type(tmp_path):
    """search_brain must return yaml neurons in results, each with a
    `file_type` field set to 'yaml'.
    """
    brain = _make_brain_with_yaml_neurons_search(tmp_path)
    results = search_brain(brain, "payments", limit=10)
    files = {r["file"]: r for r in results}
    assert "projects/openapi.yml" in files
    yaml_hit = files["projects/openapi.yml"]
    assert yaml_hit.get("file_type") == "yaml"


def test_search_excludes_kluris_yml_even_with_matching_content(tmp_path):
    """Adversarial: a `kluris.yml` with matching body content must never
    appear in search results.
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    # kluris.yml with a block AND body text that matches the query
    (brain / "kluris.yml").write_text(
        "#---\n# updated: 2026-04-09\n#---\n"
        "name: brain\ndescription: payments platform config\n",
        encoding="utf-8",
    )
    results = search_brain(brain, "payments", limit=10)
    files = [r["file"] for r in results]
    assert "kluris.yml" not in files
