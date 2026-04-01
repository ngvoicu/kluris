"""Tests for kluris neuron command."""

from click.testing import CliRunner
from kluris.cli import cli


def test_create_neuron(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["neuron", "auth.md", "--lobe", "architecture"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "architecture" / "auth.md").exists()


def test_neuron_with_template(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["neuron", "auth-migration.md", "--lobe", "decisions", "--template", "decision"])
    assert result.exit_code == 0
    content = (tmp_path / "my-brain" / "decisions" / "auth-migration.md").read_text()
    assert "## Context" in content
    assert "## Decision" in content
    assert "template: decision" in content


def test_neuron_frontmatter_parent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "architecture"])
    content = (tmp_path / "my-brain" / "architecture" / "auth.md").read_text()
    assert "parent: ./map.md" in content


def test_neuron_triggers_dream(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "architecture"])
    map_content = (tmp_path / "my-brain" / "architecture" / "map.md").read_text()
    assert "auth.md" in map_content


def test_neuron_template_not_found(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["neuron", "x.md", "--lobe", "architecture", "--template", "nonexistent"])
    assert result.exit_code != 0
    assert "not found" in result.output.lower()
