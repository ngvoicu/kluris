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
    assert "default_brain" not in data


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


def test_neuron_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "path" in data


def test_lobe_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["lobe", "experiments", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "path" in data


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


def test_push_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["push", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "brains" in data


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


def test_install_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    _create_brain(runner, tmp_path)
    result = runner.invoke(cli, ["install-skills", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "agents" in data
    assert data["commands_per_agent"] == 1
    assert "total_files" in data


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
    assert len(data["commands"]) == 18


def test_doctor_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "checks" in data


def test_error_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["remove", "nonexistent", "--json"])
    # Should still produce an error message (Click handles this)
    assert result.exit_code != 0
