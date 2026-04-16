"""Tests for kluris dream command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain, create_test_brain_with_neurons
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


def test_dream_noop_does_not_bump_dates(tmp_path, monkeypatch):
    """Running dream twice with no changes must not update map.md or brain.md
    dates, avoiding git noise from meaningless timestamp churn."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    bp = tmp_path / "my-brain"

    runner.invoke(cli, ["dream"])
    meta1_brain, _ = read_frontmatter(bp / "brain.md")
    meta1_map, body1_map = read_frontmatter(bp / "knowledge" / "map.md")
    raw1_brain = (bp / "brain.md").read_text(encoding="utf-8")
    raw1_map = (bp / "knowledge" / "map.md").read_text(encoding="utf-8")

    runner.invoke(cli, ["dream"])
    raw2_brain = (bp / "brain.md").read_text(encoding="utf-8")
    raw2_map = (bp / "knowledge" / "map.md").read_text(encoding="utf-8")

    assert raw1_brain == raw2_brain, "brain.md changed on no-op dream"
    assert raw1_map == raw2_map, "knowledge/map.md changed on no-op dream"


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


def test_dream_mixed_structural_and_deprecation_issues(tmp_path, monkeypatch):
    """A brain with both structural issues (broken link) and deprecation
    warnings must still set healthy=false for the structural issue while
    also surfacing the deprecation list. Deprecation warnings alone don't
    break healthy, but they also must not suppress real breakage."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # Structural issue: broken inline markdown link
    (tmp_path / "my-brain" / "projects" / "broken.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Broken\n\nSee [missing](./missing.md)\n",
        encoding="utf-8",
    )
    # Deprecation issue: deprecated neuron without replacement
    (tmp_path / "my-brain" / "knowledge" / "old.md").write_text(
        "---\nparent: ./map.md\nstatus: deprecated\ndeprecated_at: 2026-03-01\n"
        "tags: []\ncreated: 2026-01-01\nupdated: 2026-04-01\n---\n# Old\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)

    # Structural issue dominates exit/healthy
    assert result.exit_code == 1
    assert data["healthy"] is False
    assert data["broken_synapses"] >= 1
    # Deprecation warnings are still reported, not swallowed
    assert data["deprecation_issues"] >= 1
    assert any(
        item.get("kind") == "deprecated_without_replacement"
        for item in data["deprecation"]
    )


# --- Phase 2: batch git subprocess count ---


def test_sync_brain_state_uses_batch_git_with_exact_subprocess_count(
    tmp_path, monkeypatch, counting_git_run
):
    """Dream on a 100-neuron git brain must call core.git._run exactly 2 times:
    1. is_git_repo() to check the brain has git history
    2. git_log_file_dates() to fetch all date info in one batch

    Was ~200 before the refactor (one git log per neuron).
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain_with_neurons(runner, "big-brain", tmp_path, count=100)

    # Reset the counter (create_test_brain_with_neurons may have triggered git calls)
    counting_git_run.count = 0
    counting_git_run.calls = []

    result = runner.invoke(cli, ["dream", "--json"])
    assert result.exit_code == 0

    # Exactly 2 calls: is_git_repo + git_log_file_dates
    assert counting_git_run.count == 2, (
        f"Expected exactly 2 git subprocess calls (is_git_repo + git_log_file_dates), "
        f"got {counting_git_run.count}: {counting_git_run.calls}"
    )


def test_sync_brain_state_handles_uncommitted_neurons(tmp_path, monkeypatch):
    """A neuron that exists on disk but isn't committed yet must NOT crash dream
    and must retain its scaffolded `updated:` field (no batch hit available)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # Add a neuron that is NOT committed
    new = tmp_path / "my-brain" / "projects" / "uncommitted.md"
    new.write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2025-01-01\nupdated: 2025-06-15\n---\n"
        "# Uncommitted\n\nbody\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    assert result.exit_code == 0

    # The uncommitted neuron's scaffolded dates must be preserved
    meta, _ = read_frontmatter(new)
    assert meta["updated"] == "2025-06-15"
    assert meta["created"] == "2025-01-01"


def test_sync_brain_state_no_git_brain_skips_batch_call(
    tmp_path, monkeypatch, counting_git_run
):
    """A brain without a git repo must short-circuit the batch call.

    Subprocess count: exactly 1 (just is_git_repo, which returns False).
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    # Create with --no-git
    runner.invoke(cli, [
        "create", "no-git-brain",
        "--path", str(tmp_path),
        "--description", "test",
        "--no-git",
        "--json",
    ])

    counting_git_run.count = 0
    counting_git_run.calls = []

    result = runner.invoke(cli, ["dream", "--json"])
    assert result.exit_code == 0
    # Exactly 1 call: is_git_repo returns False, batch is skipped
    assert counting_git_run.count == 1, (
        f"Expected exactly 1 git subprocess call (is_git_repo only), "
        f"got {counting_git_run.count}: {counting_git_run.calls}"
    )


# --- yaml-neurons dream tests ---


def test_dream_discovers_opted_in_yaml_neuron_in_map(tmp_path, monkeypatch):
    """After `kluris dream`, map.md for the lobe must list the opted-in
    yaml neuron (with its frontmatter title).
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    lobe = brain / "projects"
    (lobe / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# tags: [api]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-09\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n  title: Payments API\n  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0, result.output
    map_content = (lobe / "map.md").read_text(encoding="utf-8")
    assert "openapi.yml" in map_content
    assert "Payments API" in map_content


def test_dream_excludes_kluris_yml_from_sync(tmp_path, monkeypatch):
    """Adversarial: a `kluris.yml` with a `#---` block must NOT be touched
    by dream's date-sync path.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    # Replace the auto-generated kluris.yml with an adversarial one.
    adversarial = (
        "#---\n# updated: 2026-01-01\n#---\n"
        "name: my-brain\ntype: product\n"
    )
    (brain / "kluris.yml").write_text(adversarial, encoding="utf-8")

    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0, result.output
    # The kluris.yml content must still have the original block + body.
    after = (brain / "kluris.yml").read_text(encoding="utf-8")
    assert after == adversarial


def test_dream_ignores_raw_yaml_without_block(tmp_path, monkeypatch):
    """A raw yaml file (no #--- block) in a lobe must not be indexed as a
    neuron. Dream should leave it completely untouched.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    raw = "name: ci\non: [push]\njobs:\n  build: {}\n"
    (brain / "projects" / "ci.yml").write_text(raw, encoding="utf-8")

    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0, result.output
    assert (brain / "projects" / "ci.yml").read_text(encoding="utf-8") == raw
    # Map.md must not list it.
    map_content = (brain / "projects" / "map.md").read_text(encoding="utf-8")
    assert "ci.yml" not in map_content
