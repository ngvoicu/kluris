"""Tests for --json output on all kluris commands."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def _create_brain(runner, tmp_path):
    create_test_brain(runner, "my-brain", tmp_path)


def test_create_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "name" in data
    assert "path" in data
    assert "type" in data
    assert "lobes" in data


def test_list_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "brains" in data
    assert data["brains"][0]["companions"] == []


def test_status_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "brains" in data
    assert "lobes" in data["brains"][0]
    assert "neurons" in data["brains"][0]


def test_dream_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "healthy" in data
    assert "broken_synapses" in data
    assert "one_way_synapses" in data
    assert "orphans" in data
    assert "fixes" in data
    assert "total" in data["fixes"]


def test_mri_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["mri", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "brains" in data
    assert "nodes" in data["brains"][0]
    assert "edges" in data["brains"][0]


def test_companion_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["companion", "add", "specmint-core", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["name"] == "specmint-core"
    assert data["opted_in"] is True

    result = runner.invoke(cli, ["companion", "remove", "specmint-core", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["opted_in"] is False
    assert data["files_kept"] is True


def test_remove_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["remove", "my-brain", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "name" in data
    assert "was_default" not in data


def test_help_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["help", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "commands" in data
    assert len(data["commands"]) == 13
    names = {c["name"] for c in data["commands"]}
    assert "pack" in names
    for removed in ("clone", "push", "pull", "branch"):
        assert removed not in names


def test_doctor_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "checks" in data
    assert "companions" in data


def test_error_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["remove", "nonexistent", "--json"])
    # Should still produce an error message (Click handles this)
    assert result.exit_code != 0
