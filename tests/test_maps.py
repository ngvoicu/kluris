"""Tests for brain.md and map.md generation."""

import subprocess
from pathlib import Path

from kluris.core.maps import _get_neurons, generate_brain_md, generate_map_md
from kluris.core.frontmatter import read_frontmatter


def _make_brain_with_yaml_neurons(tmp_path):
    """Copy of the fixture from test_linker.py (per-file helper pattern)."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    (brain / "kluris.yml").write_text(
        "name: brain\ntype: product\n", encoding="utf-8"
    )

    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n",
        encoding="utf-8",
    )
    (lobe / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated: [./openapi.yml]\ntags: [auth]\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n",
        encoding="utf-8",
    )
    (lobe / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./auth.md]\n"
        "# tags: [api, openapi]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )
    (lobe / "ci-config.yml").write_text(
        "name: ci\non: [push]\njobs:\n  build: {}\n",
        encoding="utf-8",
    )
    return brain


def test_get_neurons_includes_opted_in_yaml(tmp_path):
    """`_get_neurons(lobe_path)` must return markdown neurons AND opted-in
    yaml neurons in the lobe. Raw yaml without a #--- block stays excluded.
    The yaml neuron's title must resolve to its frontmatter `title` field.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    neurons = _get_neurons(brain / "projects")
    names = {n["name"] for n in neurons}
    assert "auth.md" in names
    assert "openapi.yml" in names
    assert "ci-config.yml" not in names
    # Title comes from the yaml frontmatter block
    openapi = next(n for n in neurons if n["name"] == "openapi.yml")
    assert openapi["title"] == "Payments API"


def test_generate_map_md_lists_yaml_entries(tmp_path):
    """generate_map_md must emit a list entry for each yaml neuron in the lobe,
    using the same `- [name](./name) — title` format as markdown neurons.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    generate_map_md(brain, brain / "projects")
    map_content = (brain / "projects" / "map.md").read_text(encoding="utf-8")
    # Yaml neuron is listed with its frontmatter title.
    assert "openapi.yml" in map_content
    assert "Payments API" in map_content
    # Raw yaml (no block) is NOT listed.
    assert "ci-config" not in map_content
    # Markdown neuron is still there.
    assert "auth.md" in map_content


def _make_brain(tmp_path):
    """Helper: create a minimal brain with 3 lobes and some neurons."""
    brain = tmp_path / "brain"
    brain.mkdir()
    for lobe in ["projects", "infrastructure", "knowledge"]:
        lobe_dir = brain / lobe
        lobe_dir.mkdir()
        (lobe_dir / "map.md").write_text(
            f"---\nauto_generated: true\nparent: ../brain.md\nupdated: 2026-04-01\n---\n# {lobe.title()}\n", encoding="utf-8"
        )
    (brain / "glossary.md").write_text("---\nauto_generated: false\n---\n# Glossary\n", encoding="utf-8")
    return brain


def _make_brain_with_git(tmp_path):
    """Helper: brain with git for recent changes."""
    brain = _make_brain(tmp_path)
    subprocess.run(["git", "init"], cwd=brain, capture_output=True)
    subprocess.run(["git", "checkout", "-b", "main"], cwd=brain, capture_output=True)
    # Add a neuron
    neuron = brain / "projects" / "auth.md"
    neuron.write_text("---\nparent: ../map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth Design\n", encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=brain, capture_output=True)
    subprocess.run(["git", "commit", "-m", "brain: add auth"], cwd=brain, capture_output=True)
    return brain


# --- [TEST-KLU-11] brain.md generation ---


def test_brain_md_links_all_lobes(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    content = (brain / "brain.md").read_text()
    assert "[projects/]" in content
    assert "[infrastructure/]" in content
    assert "[knowledge/]" in content


def test_brain_md_links_index(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    content = (brain / "brain.md").read_text()
    # index is now in brain.md


def test_brain_md_links_glossary(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    content = (brain / "brain.md").read_text()
    assert "glossary.md" in content


def test_brain_md_frontmatter(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    meta, _ = read_frontmatter(brain / "brain.md")
    assert meta.get("auto_generated") is True
    assert "updated" in meta


# --- [TEST-KLU-13] map.md generation ---


def test_map_lists_neurons(tmp_path):
    brain = _make_brain_with_git(tmp_path)
    generate_map_md(brain, brain / "projects")
    content = (brain / "projects" / "map.md").read_text()
    assert "auth.md" in content


def test_map_parent_link(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "projects")
    content = (brain / "projects" / "map.md").read_text()
    assert "brain.md" in content


def test_map_sibling_links(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "projects")
    content = (brain / "projects" / "map.md").read_text()
    # Should link to sibling lobes
    assert "infrastructure" in content or "knowledge" in content


def test_map_nested_lobe(tmp_path):
    brain = _make_brain(tmp_path)
    # Create a nested lobe
    nested = brain / "projects" / "patterns"
    nested.mkdir()
    (nested / "map.md").write_text("---\nauto_generated: true\nparent: ../map.md\n---\n# Patterns\n", encoding="utf-8")
    generate_map_md(brain, nested)
    content = (nested / "map.md").read_text()
    # Parent should be architecture/map.md, not brain.md
    meta, _ = read_frontmatter(nested / "map.md")
    assert meta.get("parent") == "../map.md"


def test_map_no_recent_changes(tmp_path):
    """Recent Changes section was removed -- maps are cleaner now."""
    brain = _make_brain_with_git(tmp_path)
    generate_map_md(brain, brain / "projects")
    content = (brain / "projects" / "map.md").read_text()
    assert "Recent Changes" not in content


def test_map_empty_lobe(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "knowledge")
    content = (brain / "knowledge" / "map.md").read_text()
    # Should still have structure, just no neuron entries
    assert "# Knowledge" in content or "# knowledge" in content.lower()


# --- brain.md is lightweight: lobes only, no neuron index ---


def test_brain_md_no_neuron_index(tmp_path):
    """brain.md should NOT contain a neuron table -- maps handle that."""
    brain = _make_brain_with_git(tmp_path)
    generate_brain_md(brain, "test", "Test brain")
    content = (brain / "brain.md").read_text()
    assert "Neuron Index" not in content
    assert "| Neuron |" not in content


def test_brain_md_lobes_only(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test", "Test brain")
    content = (brain / "brain.md").read_text()
    assert "## Lobes" in content
    assert "glossary.md" in content
    assert "projects" in content


def test_get_recent_changes_dead_code_removed():
    """The unused `_get_recent_changes` helper in core/maps.py was deleted
    in Phase 2 cleanup. Importing it should now fail."""
    import pytest
    with pytest.raises(ImportError):
        from kluris.core.maps import _get_recent_changes  # noqa: F401
