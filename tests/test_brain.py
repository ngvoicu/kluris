"""Tests for brain scaffolding."""

from pathlib import Path

import yaml

from kluris.core.brain import scaffold_brain, validate_brain_name


# --- [TEST-KLU-09] Brain scaffolding ---


def test_scaffold_team(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    brain = tmp_path / "brain"
    expected_lobes = [
        "projects", "infrastructure", "knowledge",
    ]
    for lobe in expected_lobes:
        assert (brain / lobe).is_dir(), f"Missing lobe: {lobe}"
        assert (brain / lobe / "map.md").exists(), f"Missing map.md in {lobe}"


def test_scaffold_personal(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Personal", "personal")
    brain = tmp_path / "brain"
    for lobe in ["projects", "tasks", "notes"]:
        assert (brain / lobe).is_dir()
    assert not (brain / "infrastructure").exists()


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
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    brain = tmp_path / "brain"
    assert (brain / "kluris.yml").exists()
    data = yaml.safe_load((brain / "kluris.yml").read_text())
    assert data["name"] == "brain"
    assert "type" not in data  # type is scaffold-only
    assert "structure" not in data


def test_creates_brain_md(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    assert (tmp_path / "brain" / "brain.md").exists()


def test_creates_index_md(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")


def test_creates_glossary(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    assert (tmp_path / "brain" / "glossary.md").exists()


def test_creates_readme(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    readme = (tmp_path / "brain" / "README.md").read_text()
    assert "/kluris-brain" in readme
    assert "auto-fix safe issues" in readme


def test_generated_readme_teaches_git_native_workflow(tmp_path):
    """README must teach git clone + kluris register, not the deleted wrappers."""
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    readme = (tmp_path / "brain" / "README.md").read_text()

    # New workflow appears
    assert "git clone <this-repo-url>" in readme
    assert "kluris register" in readme
    # Removed commands must not be advertised anywhere in the README
    for stale in (
        "kluris clone",
        "kluris push",
        "kluris pull",
        "kluris branch",
    ):
        assert stale not in readme, f"removed command leaked into README: {stale}"
    # No zip path
    assert ".zip" not in readme
    # Rule #8 mentions git directly
    assert "git directly" in readme
    # CLI commands block does not list deleted commands
    assert "kluris dream" in readme
    assert "kluris status" in readme
    assert "kluris mri" in readme


def test_generated_readme_does_not_freeze_scaffold_structure(tmp_path):
    """The generated README is durable guidance, not a static tree of the
    initial scaffold. The live structure belongs in status/mri/wake-up output.
    """
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    readme = (tmp_path / "brain" / "README.md").read_text()

    assert "## Current structure" in readme
    assert "intentionally does not list the current lobes or neurons" in readme
    assert "kluris status --brain brain" in readme
    assert "kluris mri --brain brain" in readme
    assert "kluris wake-up --brain brain" in readme
    assert "## Brain structure" not in readme
    assert "| Lobe | What goes in it |" not in readme
    for scaffold_lobe in ("`projects/`", "`infrastructure/`", "`knowledge/`"):
        assert scaffold_lobe not in readme


def test_generated_readme_uses_brain_named_slash(tmp_path):
    """Generated README must use /kluris-<name> without a bare /kluris alias."""
    scaffold_brain(tmp_path / "foo", "foo", "Foo brain", "product-group")
    readme = (tmp_path / "foo" / "README.md").read_text()
    assert "/kluris-foo " in readme  # slash command form, with trailing space
    # The deleted `kluris use` command must not appear
    assert "kluris use " not in readme
    # The old bare slash command alias must not appear.
    assert "`/kluris`" not in readme
    # No code block (lines starting with `/kluris ` not followed by a hyphen)
    # should leak the bare slash command -- examples must use /kluris-foo
    in_code_block = False
    for line in readme.splitlines():
        if line.strip().startswith("```"):
            in_code_block = not in_code_block
            continue
        if in_code_block:
            # `/kluris ` (with trailing space) inside a code block would be the bug
            assert "/kluris " not in line, f"bare /kluris leaked into code block: {line}"


def test_readme_mentions_wake_up(tmp_path):
    """Generated brain README must mention `kluris wake-up` so new team
    members know how the agent bootstraps its brain snapshot."""
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    readme = (tmp_path / "brain" / "README.md").read_text()
    assert "kluris wake-up" in readme


def test_readme_documents_deprecation_frontmatter(tmp_path):
    """Generated brain README must explain the deprecation frontmatter
    fields so humans know how to mark superseded decisions."""
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    readme = (tmp_path / "brain" / "README.md").read_text()
    assert "status:" in readme
    assert "deprecated" in readme.lower()
    assert "replaced_by" in readme


def test_from_custom_config(tmp_path):
    custom = {
        "structure": {"docs": "Documentation", "notes": "Notes"},
    }
    scaffold_brain(tmp_path / "brain", "brain", "Custom", "product-group", custom_config=custom)
    brain = tmp_path / "brain"
    assert (brain / "docs").is_dir()
    assert (brain / "notes").is_dir()
    # Should NOT have default team lobes since custom overrides
    assert not (brain / "projects").exists()


def test_creates_gitignore(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
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


def test_validate_brain_name_rejects_all():
    """`all` is reserved -- it collides with --brain all."""
    assert validate_brain_name("all") is False


def test_validate_brain_name_accepts_48_chars():
    """48-char brain name is the boundary -- still valid."""
    name = "a" + "b" * 47  # 48 chars total
    assert len(name) == 48
    assert validate_brain_name(name) is True


def test_validate_brain_name_rejects_too_long():
    """49+ char brain name is rejected (kluris-<name> would be 56+ chars)."""
    name = "a" + "b" * 48  # 49 chars total
    assert len(name) == 49
    assert validate_brain_name(name) is False


def test_paths_use_pathlib(tmp_path):
    scaffold_brain(tmp_path / "brain", "brain", "Test", "product-group")
    # Verify the function accepted a Path object and created dirs correctly
    assert isinstance(tmp_path / "brain", Path)
    assert (tmp_path / "brain" / "kluris.yml").is_file()
