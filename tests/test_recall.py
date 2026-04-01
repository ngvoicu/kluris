"""Tests for kluris recall command."""

from click.testing import CliRunner
from kluris.cli import cli


def test_recall_finds_match(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\n---\n# Keycloak Auth Design\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["recall", "Keycloak"])
    assert "Keycloak" in result.output or "auth" in result.output


def test_recall_no_match(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["recall", "xyznonexistent"])
    assert "No results" in result.output or result.exit_code == 0


def test_recall_prefers_neurons_over_generated_files(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\n---\n# Architecture notes\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["recall", "architecture", "--json"])
    import json
    data = json.loads(result.output)
    assert result.exit_code == 0
    assert data["results"]
    assert all(item["file"] == "architecture/auth.md" for item in data["results"])
