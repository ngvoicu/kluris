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
