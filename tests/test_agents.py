"""Tests for agent registry and skill rendering."""

from kluris.core.agents import AGENT_REGISTRY, render_commands, render_skill


def test_registry_8_agents():
    assert len(AGENT_REGISTRY) == 8


def test_all_agents_use_skills():
    """All agents now use skills/ subdirectory with SKILL.md format."""
    for name, reg in AGENT_REGISTRY.items():
        assert reg["subdir"] == "skills", f"{name} should use skills/"


def test_render_skill_has_frontmatter():
    content = render_skill()
    assert "---" in content
    assert "name: kluris" in content
    assert "description:" in content
    assert "allowed-tools:" in content


def test_render_skill_has_brain_info():
    brain_info = "## Your brains\n\n- **test**: `/tmp/test-brain`"
    content = render_skill(brain_info)
    assert "test-brain" in content


def test_render_creates_skill_md(tmp_path):
    files = render_commands("claude", tmp_path)
    assert len(files) == 1
    assert files[0].name == "SKILL.md"
    assert files[0].parent.name == "kluris"


def test_render_skill_content(tmp_path):
    brain_info = "## Your brains\n\n- **test**: `/tmp/test-brain`"
    files = render_commands("claude", tmp_path, brain_info=brain_info)
    content = files[0].read_text()
    assert "name: kluris" in content
    assert "test-brain" in content
    assert "Reading protocol" in content
    assert "Intent detection" in content


def test_render_same_format_all_agents(tmp_path):
    """All agents get the same SKILL.md format."""
    for agent_name in AGENT_REGISTRY:
        agent_dir = tmp_path / agent_name
        files = render_commands(agent_name, agent_dir)
        assert len(files) == 1
        assert files[0].name == "SKILL.md"
        content = files[0].read_text()
        assert "name: kluris" in content
