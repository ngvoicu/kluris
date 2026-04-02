"""Tests for kluris install-skills command."""

import json

from click.testing import CliRunner

from kluris.cli import cli


def test_install_creates_claude_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    skill_file = tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md"
    assert skill_file.exists()
    content = skill_file.read_text()
    assert "name: kluris" in content


def test_install_creates_codex_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / ".codex" / "skills" / "kluris" / "SKILL.md").exists()


def test_install_creates_gemini_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / ".gemini" / "skills" / "kluris" / "SKILL.md").exists()


def test_install_creates_cursor_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / ".cursor" / "skills" / "kluris" / "SKILL.md").exists()


def test_install_creates_windsurf_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / ".codeium" / "windsurf" / "skills" / "kluris" / "SKILL.md").exists()


def test_install_skill_has_brain_info(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    content = (tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md").read_text()
    assert "my-brain" in content


def test_install_idempotent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result1 = runner.invoke(cli, ["install-skills"])
    result2 = runner.invoke(cli, ["install-skills"])
    assert result1.exit_code == 0
    assert result2.exit_code == 0


def test_install_cleans_old_commands(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    # Create stale old command file
    old_cmd_dir = tmp_path / ".claude" / "commands"
    old_cmd_dir.mkdir(parents=True, exist_ok=True)
    stale = old_cmd_dir / "kluris.md"
    stale.write_text("stale", encoding="utf-8")
    # Install skills should clean it
    runner.invoke(cli, ["install-skills"])
    assert not stale.exists()


def test_install_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["install-skills", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["agents"] == 8
    assert data["total_files"] > 0


def test_uninstall_skills(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md").exists()
    result = runner.invoke(cli, ["uninstall-skills"])
    assert result.exit_code == 0
    assert not (tmp_path / ".claude" / "skills" / "kluris").exists()


def test_uninstall_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["uninstall-skills", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["removed"] > 0
