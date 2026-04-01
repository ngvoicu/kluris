"""Tests for brain.md, map.md, and index.md generation."""

import subprocess
from pathlib import Path

from kluris.core.maps import generate_brain_md, generate_index_md, generate_map_md
from kluris.core.frontmatter import read_frontmatter


def _make_brain(tmp_path):
    """Helper: create a minimal brain with 3 lobes and some neurons."""
    brain = tmp_path / "brain"
    brain.mkdir()
    for lobe in ["architecture", "decisions", "product"]:
        lobe_dir = brain / lobe
        lobe_dir.mkdir()
        (lobe_dir / "map.md").write_text(
            f"---\nauto_generated: true\nparent: ../brain.md\nupdated: 2026-04-01\n---\n# {lobe.title()}\n"
        )
    (brain / "glossary.md").write_text("---\nauto_generated: false\n---\n# Glossary\n")
    (brain / "index.md").write_text("---\nauto_generated: true\n---\n# Index\n")
    return brain


def _make_brain_with_git(tmp_path):
    """Helper: brain with git for recent changes."""
    brain = _make_brain(tmp_path)
    subprocess.run(["git", "init"], cwd=brain, capture_output=True)
    subprocess.run(["git", "checkout", "-b", "main"], cwd=brain, capture_output=True)
    # Add a neuron
    neuron = brain / "architecture" / "auth.md"
    neuron.write_text("---\nparent: ../map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth Design\n")
    subprocess.run(["git", "add", "-A"], cwd=brain, capture_output=True)
    subprocess.run(["git", "commit", "-m", "brain: add auth"], cwd=brain, capture_output=True)
    return brain


# --- [TEST-KLU-11] brain.md generation ---


def test_brain_md_links_all_lobes(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    content = (brain / "brain.md").read_text()
    assert "[architecture/]" in content
    assert "[decisions/]" in content
    assert "[product/]" in content


def test_brain_md_links_index(tmp_path):
    brain = _make_brain(tmp_path)
    generate_brain_md(brain, "test-brain", "A test brain")
    content = (brain / "brain.md").read_text()
    assert "index.md" in content


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
    generate_map_md(brain, brain / "architecture")
    content = (brain / "architecture" / "map.md").read_text()
    assert "auth.md" in content


def test_map_parent_link(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "architecture")
    content = (brain / "architecture" / "map.md").read_text()
    assert "brain.md" in content


def test_map_sibling_links(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "architecture")
    content = (brain / "architecture" / "map.md").read_text()
    # Should link to sibling lobes
    assert "decisions" in content or "product" in content


def test_map_nested_lobe(tmp_path):
    brain = _make_brain(tmp_path)
    # Create a nested lobe
    nested = brain / "architecture" / "patterns"
    nested.mkdir()
    (nested / "map.md").write_text("---\nauto_generated: true\nparent: ../map.md\n---\n# Patterns\n")
    generate_map_md(brain, nested)
    content = (nested / "map.md").read_text()
    # Parent should be architecture/map.md, not brain.md
    meta, _ = read_frontmatter(nested / "map.md")
    assert meta.get("parent") == "../map.md"


def test_map_recent_changes(tmp_path):
    brain = _make_brain_with_git(tmp_path)
    generate_map_md(brain, brain / "architecture")
    content = (brain / "architecture" / "map.md").read_text()
    assert "Recent Changes" in content


def test_map_empty_lobe(tmp_path):
    brain = _make_brain(tmp_path)
    generate_map_md(brain, brain / "decisions")
    content = (brain / "decisions" / "map.md").read_text()
    # Should still have structure, just no neuron entries
    assert "# Decisions" in content or "# decisions" in content.lower()


# --- [TEST-KLU-15] index.md generation ---


def test_index_lists_all_neurons(tmp_path):
    brain = _make_brain_with_git(tmp_path)
    # Add more neurons
    (brain / "decisions" / "use-sql.md").write_text(
        "---\nparent: ../map.md\ntags: [sql]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Use Raw SQL\n"
    )
    subprocess.run(["git", "add", "-A"], cwd=brain, capture_output=True)
    subprocess.run(["git", "commit", "-m", "add sql"], cwd=brain, capture_output=True)
    generate_index_md(brain)
    content = (brain / "index.md").read_text()
    assert "auth.md" in content or "Auth" in content
    assert "use-sql.md" in content or "Use Raw SQL" in content


def test_index_columns(tmp_path):
    brain = _make_brain_with_git(tmp_path)
    generate_index_md(brain)
    content = (brain / "index.md").read_text()
    assert "Neuron" in content
    assert "Lobe" in content
    assert "Tags" in content
    assert "Updated" in content


def test_index_empty_brain(tmp_path):
    brain = _make_brain(tmp_path)
    generate_index_md(brain)
    content = (brain / "index.md").read_text()
    assert "0 neurons" in content


def test_index_frontmatter(tmp_path):
    brain = _make_brain(tmp_path)
    generate_index_md(brain)
    meta, _ = read_frontmatter(brain / "index.md")
    assert meta.get("auto_generated") is True
