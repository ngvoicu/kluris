"""Tests for kluris companion add/remove."""

import json

from click.testing import CliRunner

from conftest import create_test_brain
from kluris import cli as cli_module
from kluris.cli import cli
from kluris.core.config import read_brain_config


def test_companion_add_specmint_core(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add", "specmint-core"])

    assert result.exit_code == 0, result.output
    brain = tmp_path / "my-brain"
    assert read_brain_config(brain).companions == ["specmint-core"]
    assert (tmp_path / ".kluris" / "companions" / "specmint-core" / "SKILL.md").exists()
    content = (tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md").read_text(encoding="utf-8")
    assert "## Spec-worthy work first" in content


def test_companion_remove_specmint_core(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])

    result = runner.invoke(cli, ["companion", "remove", "specmint-core"])

    assert result.exit_code == 0, result.output
    brain = tmp_path / "my-brain"
    assert read_brain_config(brain).companions == []
    content = (tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md").read_text(encoding="utf-8")
    assert "## Spec-worthy work first" not in content


def test_companion_add_all(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)

    result = runner.invoke(cli, ["companion", "add", "specmint-tdd", "--brain", "all"])

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "brain-a").companions == ["specmint-tdd"]
    assert read_brain_config(tmp_path / "brain-b").companions == ["specmint-tdd"]


def test_companion_add_idempotent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    runner.invoke(cli, ["companion", "add", "specmint-core"])
    result = runner.invoke(cli, ["companion", "add", "specmint-core"])

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == ["specmint-core"]


def test_companion_add_noninteractive_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add", "specmint-core", "--json"])

    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["name"] == "specmint-core"
    assert data["brains"] == ["my-brain"]
    assert data["opted_in"] is True
    assert data["files_copied"] is True


def test_companion_remove_leaves_global_dir(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])

    result = runner.invoke(cli, ["companion", "remove", "specmint-core"])

    assert result.exit_code == 0, result.output
    assert (tmp_path / ".kluris" / "companions" / "specmint-core" / "SKILL.md").exists()


def test_companion_add_invalid_name(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add", "foo"])

    assert result.exit_code != 0
    assert "Invalid value" in result.output


def test_companion_add_wizard_selects_both(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add"], input="3\n")

    assert result.exit_code == 0, result.output
    brain = tmp_path / "my-brain"
    assert read_brain_config(brain).companions == ["specmint-core", "specmint-tdd"]


def test_companion_add_wizard_skip(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add"], input="4\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == []
    assert "Nothing to do" in result.output


def test_companion_add_json_without_name_errors(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "add", "--json"])

    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False
    assert "non-interactive" in data["error"].lower()


def test_companion_remove_wizard_autoconfirms_single(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])

    result = runner.invoke(cli, ["companion", "remove"], input="y\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == []


def test_companion_remove_wizard_picks_from_multiple(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])
    runner.invoke(cli, ["companion", "add", "specmint-tdd"])

    # Options: [1] specmint-core [2] specmint-tdd [3] all [4] cancel
    result = runner.invoke(cli, ["companion", "remove"], input="2\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == ["specmint-core"]


def test_companion_remove_wizard_all(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])
    runner.invoke(cli, ["companion", "add", "specmint-tdd"])

    result = runner.invoke(cli, ["companion", "remove"], input="3\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == []


def test_companion_remove_wizard_cancel(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["companion", "add", "specmint-core"])
    runner.invoke(cli, ["companion", "add", "specmint-tdd"])

    # Option [4] cancel
    result = runner.invoke(cli, ["companion", "remove"], input="4\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(tmp_path / "my-brain").companions == [
        "specmint-core", "specmint-tdd",
    ]


def test_companion_remove_wizard_errors_when_nothing_enabled(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["companion", "remove"])

    assert result.exit_code != 0
    assert "No companions are enabled" in result.output
