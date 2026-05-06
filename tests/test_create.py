"""Tests for kluris create command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_brain_config, read_global_config


def test_create_team(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = create_test_brain(runner, "my-brain", tmp_path)
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "kluris.yml").exists()
    assert (tmp_path / "my-brain" / "brain.md").exists()
    assert (tmp_path / "my-brain" / "projects").is_dir()


def test_create_git_repo(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / "my-brain" / ".git").is_dir()


def test_create_registers(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    config = read_global_config()
    assert "my-brain" in config.brains


def test_create_prints_learn_hint(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = create_test_brain(runner, "my-brain", tmp_path)
    data = json.loads(result.output)
    assert data["ok"] is True


def test_create_fails_existing(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    brain_path = tmp_path / "my-brain"
    brain_path.mkdir()
    (brain_path / "kluris.yml").write_text("name: test\n", encoding="utf-8")
    result = create_test_brain(runner, "my-brain", tmp_path)
    assert result.exit_code != 0
    assert "already" in result.output


def test_create_personal(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path),
                                  "--description", "test", "--type", "personal", "--json"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects").is_dir()
    assert not (tmp_path / "my-brain" / "infrastructure").exists()


def test_create_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["name"] == "my-brain"


def test_create_error_invalid_name(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = create_test_brain(runner, "BAD NAME", tmp_path)
    assert result.exit_code != 0


def test_create_with_name_prompts_for_remaining(tmp_path, monkeypatch):
    """kluris create <name> prompts for description, location, type, git."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    wizard_input = f"Test brain\n{tmp_path}\nproduct-group\n1\nmain\n"
    result = runner.invoke(cli, ["create", "my-brain"], input=wizard_input)
    assert result.exit_code == 0
    assert "What does this brain cover" in result.output
    assert (tmp_path / "my-brain" / "brain.md").exists()


def test_create_with_all_flags_no_prompts(tmp_path, monkeypatch):
    """kluris create <name> with all flags skips prompts entirely."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path),
                                  "--description", "test", "--type", "product-group",
                                  "--no-git"])
    assert result.exit_code == 0
    assert "What does this brain cover" not in result.output
    assert "Install specmint companions" not in result.output
    assert (tmp_path / "my-brain" / "brain.md").exists()


def test_create_json_never_prompts(tmp_path, monkeypatch):
    """--json always skips prompts and uses defaults."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = create_test_brain(runner, "my-brain", tmp_path)
    assert result.exit_code == 0
    assert "What does this brain cover" not in result.output


def test_create_no_args_runs_wizard(tmp_path, monkeypatch):
    """kluris create with no args must prompt for name, description, etc."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    wizard_input = f"my-brain\nTest brain\n{tmp_path}\nproduct-group\n1\nmain\n"
    result = runner.invoke(cli, ["create"], input=wizard_input)
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "brain.md").exists()


def test_create_wizard_prompts_for_missing(tmp_path, monkeypatch):
    """kluris create with no args prompts for description, location, etc."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    wizard_input = f"my-brain\nTest brain\n{tmp_path}\nproduct-group\n1\nmain\n"
    result = runner.invoke(cli, ["create"], input=wizard_input)
    assert result.exit_code == 0
    assert "What does this brain cover" in result.output
    assert (tmp_path / "my-brain" / "brain.md").exists()


def test_create_prompt_specmint(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("kluris.cli._is_interactive", lambda: True)
    runner = CliRunner()
    wizard_input = f"my-brain\nTest brain\n{tmp_path}\nproduct-group\n3\n3\n"

    result = runner.invoke(cli, ["create"], input=wizard_input)

    assert result.exit_code == 0, result.output
    brain = tmp_path / "my-brain"
    assert read_brain_config(brain).companions == ["specmint-core", "specmint-tdd"]
    assert (tmp_path / ".kluris" / "companions" / "specmint-core" / "SKILL.md").exists()
    assert (tmp_path / ".kluris" / "companions" / "specmint-tdd" / "SKILL.md").exists()
    content = (tmp_path / ".claude" / "skills" / "kluris-my-brain" / "SKILL.md").read_text(encoding="utf-8")
    assert "## Spec-worthy work first" in content


def test_create_noninteractive_skips_prompt(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()

    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--json"])

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == []
    assert "Install specmint companions" not in result.output
