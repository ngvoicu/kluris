"""Tests for kluris status command."""

from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain


def test_status_shows_counts(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["status"])
    assert "Lobes" in result.output or "lobes" in result.output.lower()


def test_status_shows_git_log(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["status"])
    assert "initialize" in result.output.lower() or "brain" in result.output.lower()


def test_status_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["status", "--json"])
    import json
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "brains" in data


def test_status_no_git_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path),
                        "--description", "test", "--no-git", "--json"])
    result = runner.invoke(cli, ["status", "--json"])
    import json
    data = json.loads(result.output)
    assert result.exit_code == 0
    assert data["brains"][0]["git_enabled"] is False
    assert data["brains"][0]["recent_commits"] == []


def test_status_ignores_hidden_and_tooling_dirs(tmp_path, monkeypatch):
    """`status` must not count markdown under .git/, node_modules/, etc.

    Before we centralized on the shared neuron filter, `status` used a raw
    rglob that counted every `.md` file anywhere in the brain tree -- so a
    random markdown file committed by a tool into node_modules/ would
    inflate the neuron count.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"

    # Plant stray markdown files under tooling/hidden dirs
    (brain / "node_modules" / "pkg").mkdir(parents=True)
    (brain / "node_modules" / "pkg" / "README.md").write_text("# pkg\n", encoding="utf-8")
    (brain / ".github" / "workflows").mkdir(parents=True)
    (brain / ".github" / "workflows" / "ci.md").write_text("# ci\n", encoding="utf-8")

    result = runner.invoke(cli, ["status", "--json"])
    import json
    data = json.loads(result.output)
    # Baseline brain has no neurons under test fixture so count must be 0
    assert data["brains"][0]["neurons"] == 0
