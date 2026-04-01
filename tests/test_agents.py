"""Tests for agent registry and template rendering."""

from pathlib import Path

from kluris.core.agents import AGENT_REGISTRY, install_for_agent, render_commands


def test_registry_8_agents():
    assert len(AGENT_REGISTRY) == 8


def test_claude_path():
    r = AGENT_REGISTRY["claude"]
    assert r["subdir"] == "commands"
    assert r["format"] == "md"


def test_codex_skill_format():
    r = AGENT_REGISTRY["codex"]
    assert r["format"] == "skill.md"
    assert r["subdir"] == "skills"


def test_gemini_toml():
    r = AGENT_REGISTRY["gemini"]
    assert r["format"] == "toml"
    assert r["args"] == "{{args}}"


def test_copilot_agent_md():
    r = AGENT_REGISTRY["copilot"]
    assert r["format"] == "agent.md"


def test_junie_path():
    r = AGENT_REGISTRY["junie"]
    assert r["subdir"] == "commands"
    assert r["format"] == "md"


def test_kilocode_path():
    r = AGENT_REGISTRY["kilocode"]
    assert "kilo" in r["dir"]


def test_render_md(tmp_path):
    files = render_commands("claude", tmp_path)
    assert len(files) >= 1
    # Check one file has $ARGUMENTS
    content = files[0].read_text()
    assert "$ARGUMENTS" in content or "ARGUMENTS" in content


def test_render_toml(tmp_path):
    files = render_commands("gemini", tmp_path)
    assert len(files) >= 1
    content = files[0].read_text()
    assert "{{args}}" in content or "args" in content


def test_render_skill_md(tmp_path):
    files = render_commands("codex", tmp_path)
    assert len(files) == 1
    assert files[0].name == "SKILL.md"


def test_render_agent_md(tmp_path):
    files = render_commands("copilot", tmp_path)
    assert len(files) >= 1
    assert all(f.suffix == ".md" for f in files)
    assert any("agent" in f.name for f in files)


def test_all_9_commands(tmp_path):
    files = render_commands("claude", tmp_path)
    assert len(files) == 9


def test_all_reference_config(tmp_path):
    files = render_commands("claude", tmp_path)
    for f in files:
        content = f.read_text()
        assert "config" in content.lower() or "KLURIS_CONFIG" in content


def test_context_budget(tmp_path):
    files = render_commands("claude", tmp_path)
    # think, recall, learn commands should mention max neurons or context budget
    think_file = [f for f in files if "think" in f.name]
    assert len(think_file) == 1
    content = think_file[0].read_text()
    assert "max" in content.lower() or "10" in content or "budget" in content.lower()
