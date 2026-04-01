"""Tests for kluris install command."""

import json

from click.testing import CliRunner

from kluris.cli import cli


def test_install_creates_claude_commands(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    claude_dir = tmp_path / ".claude" / "commands"
    assert claude_dir.exists()
    md_files = list(claude_dir.glob("kluris*.md"))
    assert len(md_files) == 10


def test_install_creates_gemini_toml(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    gemini_dir = tmp_path / ".gemini" / "commands"
    assert gemini_dir.exists()
    toml_files = list(gemini_dir.glob("kluris*.toml"))
    assert len(toml_files) == 10


def test_install_creates_codex_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    codex_dir = tmp_path / ".codex" / "skills"
    assert (codex_dir / "kluris" / "SKILL.md").exists()
    assert (codex_dir / "kluris-think" / "SKILL.md").exists()
    assert (codex_dir / "kluris-learn" / "SKILL.md").exists()


def test_install_creates_copilot_agent_md(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    copilot_dir = tmp_path / ".copilot" / "agents"
    agent_files = list(copilot_dir.glob("*.agent.md"))
    assert len(agent_files) == 10


def test_install_creates_junie(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    junie_dir = tmp_path / ".junie" / "commands"
    assert junie_dir.exists()
    assert len(list(junie_dir.glob("kluris*.md"))) == 10


def test_install_creates_kilocode(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    kilo_dir = tmp_path / ".config" / "kilo" / "commands"
    assert kilo_dir.exists()


def test_install_content_references_config(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    cmd_file = tmp_path / ".claude" / "commands" / "kluris.md"
    content = cmd_file.read_text()
    assert "config" in content.lower()


def test_install_idempotent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result1 = runner.invoke(cli, ["install-commands"])
    result2 = runner.invoke(cli, ["install-commands"])
    assert result1.exit_code == 0
    assert result2.exit_code == 0


def test_install_clean_slate(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    # Create a stale file
    stale = tmp_path / ".claude" / "commands" / "kluris.old-command.md"
    stale.write_text("stale content", encoding="utf-8")
    runner.invoke(cli, ["install-commands"])
    assert not stale.exists()


def test_install_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["install-commands", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["agents"] == 8
    assert data["total_files"] > 0
