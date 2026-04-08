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
    data = json.loads(result.output)
    assert data["ok"] is True
    assert len(data["commands"]) == 17
    names = {c["name"] for c in data["commands"]}
    assert "wake-up" in names
    assert "search" in names
    assert "use" not in names
