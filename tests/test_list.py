"""Tests for kluris list command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_brain_config, write_brain_config


def test_list_shows_brains(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
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
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.stdout)
    assert data["ok"] is True
    assert len(data["brains"]) == 1
    assert data["brains"][0]["companions"] == []
    # `type` and `repo` were dropped from BrainEntry in 2.16.0.
    assert "type" not in data["brains"][0]
    assert "repo" not in data["brains"][0]


def test_list_human_no_type_column(tmp_path, monkeypatch):
    """The human-readable table must not show a 'Type' column."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["list"])
    assert result.exit_code == 0
    assert "Type" not in result.output
    # Other columns still show
    for column in ("Name", "Path", "Description", "Companions"):
        assert column in result.output


def test_list_shows_companions(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    cfg = read_brain_config(brain)
    cfg.companions = ["specmint-core"]
    write_brain_config(cfg, brain)

    result = runner.invoke(cli, ["list"])
    assert result.exit_code == 0
    assert "Companions" in result.output
    assert "specmint-core" in result.output

    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.stdout)
    assert data["brains"][0]["companions"] == ["specmint-core"]

