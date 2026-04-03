"""Tests for kluris lobe command."""

from click.testing import CliRunner
from kluris.cli import cli


def test_create_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["lobe", "experiments"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "experiments").is_dir()


def test_nested_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["lobe", "patterns", "--parent", "architecture"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "architecture" / "patterns").is_dir()
    assert (tmp_path / "my-brain" / "architecture" / "patterns" / "map.md").exists()


def test_lobe_description_is_preserved(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])

    result = runner.invoke(cli, ["lobe", "experiments", "--description", "Spike notes and discovery work"])

    assert result.exit_code == 0
    map_content = (tmp_path / "my-brain" / "experiments" / "map.md").read_text(encoding="utf-8")
    assert "description: Spike notes and discovery work" in map_content
    assert "Spike notes and discovery work" in map_content


def test_lobe_triggers_dream(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    runner.invoke(cli, ["lobe", "experiments"])
    brain_md = (tmp_path / "my-brain" / "brain.md").read_text()
    assert "experiments" in brain_md


def test_lobe_rejects_paths_outside_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])

    result = runner.invoke(cli, ["lobe", "secret", "--parent", "../my-brain-escape"])

    assert result.exit_code != 0
    assert "Path escapes the brain directory" in result.output
    assert not (tmp_path / "my-brain-escape" / "secret").exists()
