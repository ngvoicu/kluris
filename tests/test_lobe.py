"""Tests for kluris lobe command."""

from click.testing import CliRunner
from kluris.cli import cli


def test_create_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["lobe", "experiments"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "experiments").is_dir()


def test_nested_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["lobe", "patterns", "--parent", "architecture"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "architecture" / "patterns").is_dir()


def test_lobe_triggers_dream(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    runner.invoke(cli, ["lobe", "experiments"])
    brain_md = (tmp_path / "my-brain" / "brain.md").read_text()
    assert "experiments" in brain_md
