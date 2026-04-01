"""Tests for kluris create command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from kluris.core.config import read_global_config


def test_create_team(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "kluris.yml").exists()
    assert (tmp_path / "my-brain" / "brain.md").exists()
    assert (tmp_path / "my-brain" / "architecture").is_dir()


def test_create_git_repo(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert (tmp_path / "my-brain" / ".git").is_dir()


def test_create_registers(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    config = read_global_config()
    assert "my-brain" in config.brains


def test_create_sets_default(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    config = read_global_config()
    assert config.default_brain == "my-brain"


def test_create_prints_learn_hint(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert "/kluris.learn" in result.output


def test_create_fails_existing(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    brain_path = tmp_path / "my-brain"
    brain_path.mkdir()
    (brain_path / "kluris.yml").write_text("name: test\n", encoding="utf-8")
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert result.exit_code != 0
    assert "already" in result.output


def test_create_personal(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--type", "personal"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects").is_dir()
    assert not (tmp_path / "my-brain" / "architecture").exists()


def test_create_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["name"] == "my-brain"


def test_create_json_reports_actual_default(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "brain-a", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["create", "brain-b", "--path", str(tmp_path), "--json"])
    data = json.loads(result.output)
    assert data["default_brain"] == "brain-a"


def test_create_error_invalid_name(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["create", "BAD NAME", "--path", str(tmp_path)])
    assert result.exit_code != 0
