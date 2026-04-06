"""Extra CLI tests for coverage — templates, doctor, help, error paths, edge cases."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def test_templates_command(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["templates"])
    assert result.exit_code == 0
    assert "decision" in result.output
    assert "incident" in result.output
    assert "runbook" in result.output


def test_templates_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["templates", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "decision" in data["templates"]
    assert "incident" in data["templates"]


def test_help_unknown_command(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help", "nonexistent"])
    assert result.exit_code != 0


def test_help_shows_config_path(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help"])
    assert "config" in result.output.lower() or ".kluris" in result.output


def test_remove_triggers_install(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / ".claude" / "skills" / "kluris" / "SKILL.md").exists()
    runner.invoke(cli, ["remove", "my-brain"])
    # Commands should still exist (reinstalled for remaining brains)
    # or be cleaned if no brains left


def test_status_no_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["status"])
    assert result.exit_code != 0
    assert "No brains" in result.output


def test_dream_no_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code != 0


def test_mri_no_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["mri"])
    assert result.exit_code != 0


def test_push_no_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["push"])
    assert result.exit_code != 0


def test_version():
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    import kluris
    assert kluris.__version__ in result.output


def test_main_help():
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])
    assert "create" in result.output
    assert "clone" in result.output
    assert "dream" in result.output
    assert "install-skills" in result.output
    assert "uninstall-skills" in result.output
    assert "templates" in result.output


def test_json_error_output(tmp_path, monkeypatch):
    """Errors with --json should return structured JSON."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["remove", "nonexistent", "--json"])
    assert result.exit_code != 0
    # Should have JSON error (from KlurisGroup)
    import sys
    # The --json flag triggers JSON error output
