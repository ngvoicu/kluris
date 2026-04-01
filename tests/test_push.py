"""Tests for kluris push command."""

from click.testing import CliRunner
from kluris.cli import cli


def test_push_clean_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["push"])
    assert "nothing to push" in result.output.lower()


def test_push_commits_changes(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    result = runner.invoke(cli, ["push", "-m", "add auth"])
    assert "committed" in result.output.lower() or result.exit_code == 0


def test_push_with_message(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    runner.invoke(cli, ["push", "-m", "brain: add auth neuron"])
    import subprocess
    log = subprocess.run(["git", "log", "--oneline", "-1"], cwd=tmp_path / "my-brain", capture_output=True, text=True)
    assert "add auth" in log.stdout


def test_push_no_remote_warning(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    result = runner.invoke(cli, ["push", "-m", "test"])
    # Should succeed (local commit) even without remote
    assert result.exit_code == 0


def test_push_no_git_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path), "--no-git"])
    result = runner.invoke(cli, ["push", "--json"])
    import json
    data = json.loads(result.output)
    assert result.exit_code == 0
    assert data["brains"][0]["git_enabled"] is False
    assert data["brains"][0]["pushed"] is False
