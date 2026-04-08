"""Tests for kluris lobe command."""

from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain


def test_create_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["lobe", "experiments"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "experiments").is_dir()


def test_nested_lobe(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["lobe", "patterns", "--parent", "projects"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects" / "patterns").is_dir()
    assert (tmp_path / "my-brain" / "projects" / "patterns" / "map.md").exists()


def test_lobe_description_is_preserved(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["lobe", "experiments", "--description", "Spike notes and discovery work"])

    assert result.exit_code == 0
    map_content = (tmp_path / "my-brain" / "experiments" / "map.md").read_text(encoding="utf-8")
    assert "description: Spike notes and discovery work" in map_content
    assert "Spike notes and discovery work" in map_content


def test_lobe_triggers_dream(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["lobe", "experiments"])
    brain_md = (tmp_path / "my-brain" / "brain.md").read_text()
    assert "experiments" in brain_md


def test_lobe_rejects_paths_outside_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["lobe", "secret", "--parent", "../my-brain-escape"])

    assert result.exit_code != 0
    assert "Path escapes the brain directory" in result.output
    assert not (tmp_path / "my-brain-escape" / "secret").exists()


def test_lobe_rejects_existing_directory(tmp_path, monkeypatch):
    """Creating a lobe where one already exists must fail without --force."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # The scaffolded product-group brain already has a `projects` lobe
    result = runner.invoke(cli, ["lobe", "projects"])
    assert result.exit_code != 0
    assert "already exists" in result.output


def test_lobe_force_allows_existing_directory(tmp_path, monkeypatch):
    """--force lets the user add a description to an existing lobe."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(
        cli,
        ["lobe", "projects", "--description", "Project knowledge", "--force"],
    )
    assert result.exit_code == 0
