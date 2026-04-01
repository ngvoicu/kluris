"""Tests for kluris list command."""

import json
from click.testing import CliRunner
from kluris.cli import cli


def test_list_shows_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["list"])
    assert "my-brain" in result.output


def test_list_empty(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["list"])
    assert "No brains" in result.output


def test_list_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert len(data["brains"]) == 1


def test_use_sets_default_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "brain-a", "--path", str(tmp_path)])
    runner.invoke(cli, ["create", "brain-b", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["use", "brain-b", "--json"])
    data = json.loads(result.output)
    assert result.exit_code == 0
    assert data["default_brain"] == "brain-b"

    listed = runner.invoke(cli, ["list", "--json"])
    listed_data = json.loads(listed.output)
    assert listed_data["default_brain"] == "brain-b"
