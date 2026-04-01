"""Tests for brain scaffolding."""

import re
from pathlib import Path

import yaml

from kluris.core.brain import get_type_defaults, scaffold_brain, validate_brain_name


# --- [TEST-KLU-09] Brain scaffolding ---


def test_scaffold_team(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    brain = tmp_path / "brain"
    expected_lobes = [
        "architecture", "decisions", "product", "standards",
        "services", "infrastructure", "cortex", "wisdom",
    ]
    for lobe in expected_lobes:
        assert (brain / lobe).is_dir(), f"Missing lobe: {lobe}"
        assert (brain / lobe / "map.md").exists(), f"Missing map.md in {lobe}"


def test_scaffold_personal(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Personal", "personal")
    brain = tmp_path / "brain"
    for lobe in ["projects", "tasks", "releases", "notes"]:
        assert (brain / lobe).is_dir()
    assert not (brain / "architecture").exists()


def test_scaffold_product(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Product", "product")
    brain = tmp_path / "brain"
    for lobe in ["prd", "features", "ux", "analytics", "competitors", "decisions"]:
        assert (brain / lobe).is_dir()


def test_scaffold_research(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Research", "research")
    brain = tmp_path / "brain"
    for lobe in ["literature", "experiments", "findings", "datasets", "tools", "questions"]:
        assert (brain / lobe).is_dir()


def test_scaffold_blank(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Blank", "blank")
    brain = tmp_path / "brain"
    # Should have no lobe directories (only files)
    dirs = [d for d in brain.iterdir() if d.is_dir() and d.name != ".git"]
    assert len(dirs) == 0


def test_creates_kluris_yml(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    brain = tmp_path / "brain"
    assert (brain / "kluris.yml").exists()
    data = yaml.safe_load((brain / "kluris.yml").read_text())
    assert data["name"] == "brain"
    assert data["type"] == "team"
    assert "structure" not in data


def test_creates_brain_md(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    assert (tmp_path / "brain" / "brain.md").exists()


def test_creates_index_md(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    assert (tmp_path / "brain" / "index.md").exists()


def test_creates_glossary(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    assert (tmp_path / "brain" / "glossary.md").exists()


def test_creates_readme(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    readme = (tmp_path / "brain" / "README.md").read_text()
    assert "/kluris.learn" in readme
    assert "/kluris.think" in readme


def test_team_has_neuron_templates(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    data = yaml.safe_load((tmp_path / "brain" / "kluris.yml").read_text())
    templates = data.get("neuron_templates", {})
    assert "decision" in templates
    assert "incident" in templates
    assert "runbook" in templates
    assert len(templates["decision"]["sections"]) == 5
    assert len(templates["incident"]["sections"]) == 6
    assert len(templates["runbook"]["sections"]) == 5


def test_from_custom_config(tmp_path):
    custom = {
        "structure": {"docs": "Documentation", "notes": "Notes"},
    }
    scaffold_brain(tmp_path / "brain", "brain", "Custom", "team", custom_config=custom)
    brain = tmp_path / "brain"
    assert (brain / "docs").is_dir()
    assert (brain / "notes").is_dir()
    # Should NOT have default team lobes since custom overrides
    assert not (brain / "architecture").exists()


def test_creates_gitignore(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    gitignore = (tmp_path / "brain" / ".gitignore").read_text()
    assert ".env" in gitignore
    assert "*.key" in gitignore
    assert "*.pem" in gitignore
    assert "brain-mri.html" in gitignore


def test_brain_name_sanitization():
    assert validate_brain_name("my-brain") is True
    assert validate_brain_name("test-123") is True
    assert validate_brain_name("brain") is True

    assert validate_brain_name("../evil") is False
    assert validate_brain_name(".hidden") is False
    assert validate_brain_name("has spaces") is False
    assert validate_brain_name("special!chars") is False
    assert validate_brain_name("") is False
    assert validate_brain_name("UPPERCASE") is False


def test_paths_use_pathlib(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "team")
    # Verify the function accepted a Path object and created dirs correctly
    assert isinstance(tmp_path / "brain", Path)
    assert (tmp_path / "brain" / "kluris.yml").is_file()
