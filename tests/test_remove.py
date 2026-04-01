"""Tests for kluris remove command."""

from click.testing import CliRunner
from kluris.cli import cli
from kluris.core.config import read_global_config


def test_remove_unregisters(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["remove", "my-brain"])
    assert result.exit_code == 0
    config = read_global_config()
    assert "my-brain" not in config.brains


def test_remove_clears_default(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    runner.invoke(cli, ["remove", "my-brain"])
    config = read_global_config()
    assert config.default_brain is None


def test_remove_preserves_files(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    runner.invoke(cli, ["remove", "my-brain"])
    assert (tmp_path / "my-brain" / "kluris.yml").exists()


def test_remove_unknown(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["remove", "nonexistent"])
    assert result.exit_code != 0
