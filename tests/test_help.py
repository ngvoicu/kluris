"""Tests for kluris help command."""

import json
from click.testing import CliRunner
from kluris.cli import cli


def test_help_lists_commands(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help"])
    assert "create" in result.output
    assert "dream" in result.output
    assert "doctor" in result.output


def test_help_specific_command(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help", "create"])
    assert "create" in result.output


def test_help_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help", "--json"])
    data = json.loads(result.stdout)
    assert data["ok"] is True
    assert len(data["commands"]) == 13
    names = {c["name"] for c in data["commands"]}
    assert "wake-up" in names
    assert "search" in names
    assert "companion" in names
    assert "register" in names
    assert "pack" in names
    # Removed in 2.16.0 — wrappers over `git` / zip path:
    assert "clone" not in names
    assert "push" not in names
    assert "pull" not in names
    assert "branch" not in names
    # Older removals stay gone:
    assert "use" not in names
    assert "install-skills" not in names
    assert "uninstall-skills" not in names
    assert "neuron" not in names
    assert "lobe" not in names


def test_help_text_omits_removed_commands(tmp_path, monkeypatch):
    """The plain-text help output should not list any of the removed commands."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help"])
    assert result.exit_code == 0
    # Each name appears only as its own command line, so this matches the listing.
    for name in ("clone", "push", "pull", "branch"):
        # We use a leading two-space indent + name + space to avoid collisions
        # with the word "branch" in a description line, etc.
        assert f"  {name} " not in result.output
        assert f"  {name:<10}" not in result.output


def test_removed_commands_no_such_command(tmp_path, monkeypatch):
    """Invoking a removed command should fail with Click's no-such-command error."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    for name in ("clone", "push", "pull", "branch"):
        result = runner.invoke(cli, [name, "--help"])
        assert result.exit_code != 0, f"{name} should not be a known command"
