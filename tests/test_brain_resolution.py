"""Tests for brain resolution logic."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def test_explicit_brain_flag(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status", "--brain", "brain-b", "--json"])
    data = json.loads(result.output)
    assert data["brains"][0]["name"] == "brain-b"


def test_default_brain_used(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    # brain-a is default (first created)
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.output)
    assert data["brains"][0]["name"] == "brain-a"


def test_single_brain_auto(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "only-brain", tmp_path)
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.output)
    assert len(data["brains"]) == 1


def test_multi_brain_neuron_error(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    from kluris.core.config import read_global_config, write_global_config
    cfg = read_global_config()
    cfg.default_brain = None
    write_global_config(cfg)
    # neuron should error when multi-brain and no default
    result = runner.invoke(cli, ["neuron", "test.md", "--lobe", "projects"])
    assert result.exit_code != 0
    assert "Multiple brains" in result.output or "Specify" in result.output
