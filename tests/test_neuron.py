"""Tests for kluris neuron command."""

from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain


def test_create_neuron(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects" / "auth.md").exists()


def test_neuron_with_template(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["neuron", "auth-migration.md", "--lobe", "knowledge", "--template", "decision"])
    assert result.exit_code == 0
    content = (tmp_path / "my-brain" / "knowledge" / "auth-migration.md").read_text()
    assert "## Context" in content
    assert "## Decision" in content
    assert "template: decision" in content


def test_neuron_frontmatter_parent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    content = (tmp_path / "my-brain" / "projects" / "auth.md").read_text()
    assert "parent: ./map.md" in content


def test_neuron_triggers_dream(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    map_content = (tmp_path / "my-brain" / "projects" / "map.md").read_text()
    assert "auth.md" in map_content


def test_neuron_nested_path_creates_nested_map(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["neuron", "projects/api/auth.md"])

    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "projects" / "api" / "auth.md").exists()
    nested_map = tmp_path / "my-brain" / "projects" / "api" / "map.md"
    assert nested_map.exists()
    map_content = nested_map.read_text(encoding="utf-8")
    assert "auth.md" in map_content


def test_neuron_template_not_found(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["neuron", "x.md", "--lobe", "projects", "--template", "nonexistent"])
    assert result.exit_code != 0
    assert "not found" in result.output.lower()


def test_neuron_rejects_paths_outside_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["neuron", "pwn.md", "--lobe", "../my-brain-escape"])

    assert result.exit_code != 0
    assert "Path escapes the brain directory" in result.output
    assert not (tmp_path / "my-brain-escape" / "pwn.md").exists()


def test_neuron_rejects_overwriting_existing(tmp_path, monkeypatch):
    """A second `kluris neuron foo.md --lobe projects` must fail to protect user content."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # First create succeeds
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    # Hand-edit it to have unique content
    target = tmp_path / "my-brain" / "projects" / "auth.md"
    target.write_text(target.read_text() + "\nhuman wrote this\n", encoding="utf-8")

    # Second create with the same path must fail with a clear error
    result = runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    assert result.exit_code != 0
    assert "already exists" in result.output
    # And the user's content must be untouched
    assert "human wrote this" in target.read_text()


def test_neuron_force_overwrites(tmp_path, monkeypatch):
    """--force bypasses the overwrite guard (explicit user intent)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects"])
    target = tmp_path / "my-brain" / "projects" / "auth.md"
    target.write_text(target.read_text() + "\nhuman wrote this\n", encoding="utf-8")

    result = runner.invoke(cli, ["neuron", "auth.md", "--lobe", "projects", "--force"])
    assert result.exit_code == 0
    # With --force, the user's content is gone (user explicitly asked)
    assert "human wrote this" not in target.read_text()
