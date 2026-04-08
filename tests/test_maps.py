"""Tests for brain.md and map.md generation."""

import subprocess
from pathlib import Path

from kluris.core.maps import generate_brain_md, generate_index_md, generate_map_md
from kluris.core.frontmatter import read_frontmatter


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
