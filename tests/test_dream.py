"""Tests for kluris dream command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.frontmatter import read_frontmatter


def test_dream_regenerates_maps(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    # Add a neuron manually
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream"])
    map_content = (tmp_path / "my-brain" / "projects" / "map.md").read_text().lower()
    assert "auth" in map_content


def test_dream_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert "healthy" in data
    assert "broken_synapses" in data
    assert "fixes" in data
    assert "total" in data["fixes"]


def test_dream_exit_0_healthy(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0


def test_dream_regenerates_brain_md(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    # Add a lobe manually
    (tmp_path / "my-brain" / "experiments").mkdir()
    runner.invoke(cli, ["dream"])
    brain_md = (tmp_path / "my-brain" / "brain.md").read_text()
    assert "experiments" in brain_md


def test_dream_updates_map_with_neuron(tmp_path, monkeypatch):
    """After adding a neuron, dream should update the lobe's map.md (not brain.md)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    runner.invoke(cli, ["dream"])
    map_content = (tmp_path / "my-brain" / "projects" / "map.md").read_text()
    assert "auth" in map_content.lower()


def test_dream_preserves_lobe_descriptions(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    runner.invoke(cli, ["dream"])
    runner.invoke(cli, ["dream"])

    brain_md = (tmp_path / "my-brain" / "brain.md").read_text(encoding="utf-8")
    assert "- [projects/](./projects/map.md)" in brain_md
    assert "— auto_generated: true" not in brain_md


def test_dream_reports_broken_links(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "bad.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Bad\n\n[broken](./nonexistent.md)\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert data["broken_synapses"] >= 1


def test_dream_fixes_one_way_synapse(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "a.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../knowledge/b.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# A\n", encoding="utf-8"
    )
    (tmp_path / "my-brain" / "knowledge" / "b.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# B\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    meta, _ = read_frontmatter(tmp_path / "my-brain" / "knowledge" / "b.md")
    assert result.exit_code == 0
    assert data["one_way_synapses"] == 0
    assert data["fixes"]["reverse_synapses_added"] == 1
    assert "../projects/a.md" in meta["related"]


def test_dream_adds_missing_parent_frontmatter(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    neuron = tmp_path / "my-brain" / "projects" / "no-parent.md"
    neuron.write_text(
        "---\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# No Parent\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    meta, _ = read_frontmatter(neuron)

    assert result.exit_code == 0
    assert data["frontmatter_issues"] == 0
    assert data["fixes"]["parents_inferred"] == 1
    assert meta["parent"] == "./map.md"


def test_dream_fixes_orphans_by_regenerating_parent_map(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    neuron = tmp_path / "my-brain" / "projects" / "orphan.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Orphan\n",
        encoding="utf-8",
    )
    (tmp_path / "my-brain" / "projects" / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\nupdated: 2026-04-01\n---\n# Architecture\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    map_content = (tmp_path / "my-brain" / "projects" / "map.md").read_text(encoding="utf-8")

    assert result.exit_code == 0
    assert data["orphans"] == 0
    assert data["fixes"]["orphan_references_added"] == 1
    assert "orphan.md" in map_content


def test_dream_shows_fix_counts_in_output(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "a.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../knowledge/b.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# A\n",
        encoding="utf-8",
    )
    (tmp_path / "my-brain" / "knowledge" / "b.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# B\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    assert "3 automatic fixes applied" in result.output
    assert "1 missing reverse related links added" in result.output
    assert "2 missing neuron references added to parent map.md files" in result.output


def test_dream_shows_lobes_and_maps(tmp_path, monkeypatch):
    """Dream output must list discovered lobes and regenerated maps."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    assert "Lobes:" in result.output
    assert "projects" in result.output
    assert "projects" in result.output
    assert "Maps regenerated:" in result.output


def test_dream_reports_broken_related_synapse(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "a.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../knowledge/missing.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# A\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)

    assert result.exit_code == 1
    assert data["broken_synapses"] >= 1


def test_dream_exit_1_issues(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "bad.md").write_text(
        "---\nparent: ./map.md\n---\n# Bad\n\n[broken](./nope.md)\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 1


def test_dream_generates_nested_maps(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "api").mkdir(parents=True)

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects" / "api" / "map.md").exists()


def test_dream_sub_lobe_listed_in_parent_map(tmp_path, monkeypatch):
    """After dream, parent lobe's map.md must contain a link to the sub-lobe."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "api").mkdir(parents=True)
    (tmp_path / "my-brain" / "projects" / "api" / "endpoints.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Endpoints\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    parent_map = (tmp_path / "my-brain" / "projects" / "map.md").read_text(encoding="utf-8")
    assert "api/" in parent_map
    assert "api/map.md" in parent_map


def test_dream_sibling_sub_lobes_see_each_other(tmp_path, monkeypatch):
    """Two sibling sub-lobes created together must both appear in each other's sideways links."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "api").mkdir(parents=True)
    (tmp_path / "my-brain" / "projects" / "web").mkdir(parents=True)

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    api_map = (tmp_path / "my-brain" / "projects" / "api" / "map.md").read_text(encoding="utf-8")
    web_map = (tmp_path / "my-brain" / "projects" / "web" / "map.md").read_text(encoding="utf-8")
    assert "web" in api_map, "api/map.md should list web as sibling"
    assert "api" in web_map, "web/map.md should list api as sibling"


def test_dream_reports_deprecation_warnings_json(tmp_path, monkeypatch):
    """Dream's JSON output surfaces deprecation_issues count and a list."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # Active neuron references a deprecated one
    (tmp_path / "my-brain" / "knowledge" / "new.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ./old.md\n"
        "tags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# New\n",
        encoding="utf-8",
    )
    (tmp_path / "my-brain" / "knowledge" / "old.md").write_text(
        "---\nparent: ./map.md\nstatus: deprecated\ndeprecated_at: 2026-03-01\n"
        "replaced_by: ./new.md\nrelated:\n  - ./new.md\n"
        "tags: []\ncreated: 2026-01-01\nupdated: 2026-04-01\n---\n# Old\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)

    assert "deprecation_issues" in data
    assert data["deprecation_issues"] >= 1
    assert "deprecation" in data
    assert any(
        item.get("kind") == "active_links_to_deprecated"
        for item in data["deprecation"]
    )


def test_dream_deprecation_warnings_do_not_fail_healthy(tmp_path, monkeypatch):
    """Deprecation issues are warnings — dream still exits 0 when they are
    the only finding (so CI pipelines don't break on legitimate migrations)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    (tmp_path / "my-brain" / "knowledge" / "old.md").write_text(
        "---\nparent: ./map.md\nstatus: deprecated\ndeprecated_at: 2026-03-01\n"
        "tags: []\ncreated: 2026-01-01\nupdated: 2026-04-01\n---\n# Old\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0
    assert "deprecat" in result.output.lower()


def test_dream_no_deprecation_issues_on_clean_brain(tmp_path, monkeypatch):
    """Clean brain shows deprecation_issues == 0."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert data["deprecation_issues"] == 0
