"""Tests for brain resolution logic."""

import json

from click.testing import CliRunner

from kluris.cli import cli


def test_explicit_brain_flag(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "brain-a")])
    runner.invoke(cli, ["create", str(tmp_path / "brain-b")])
    result = runner.invoke(cli, ["status", "--brain", "brain-b", "--json"])
    data = json.loads(result.output)
    assert data["brains"][0]["name"] == "brain-b"


def test_default_brain_used(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "brain-a")])
    runner.invoke(cli, ["create", str(tmp_path / "brain-b")])
    # brain-a is default (first created)
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.output)
    assert data["brains"][0]["name"] == "brain-a"


def test_single_brain_auto(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "only-brain")])
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.output)
    assert len(data["brains"]) == 1


def test_multi_brain_recall_all(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "brain-a")])
    runner.invoke(cli, ["create", str(tmp_path / "brain-b")])
    # Remove default so it doesn't short-circuit to one brain
    from kluris.core.config import read_global_config, write_global_config
    cfg = read_global_config()
    cfg.default_brain = None
    write_global_config(cfg)
    # recall operates on all when multi-brain
    (tmp_path / "brain-a" / "architecture" / "auth.md").write_text("# Keycloak Auth\n")
    result = runner.invoke(cli, ["recall", "Keycloak", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True


def test_multi_brain_neuron_error(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "brain-a")])
    runner.invoke(cli, ["create", str(tmp_path / "brain-b")])
    from kluris.core.config import read_global_config, write_global_config
    cfg = read_global_config()
    cfg.default_brain = None
    write_global_config(cfg)
    # neuron should error when multi-brain and no default
    result = runner.invoke(cli, ["neuron", "test.md", "--lobe", "architecture"])
    assert result.exit_code != 0
    assert "Multiple brains" in result.output or "Specify" in result.output
