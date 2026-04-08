"""Tests for kluris push command."""

from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain


def test_push_clean_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["push"])
    assert "nothing to push" in result.output.lower()


def test_push_clean_brain_json_reports_configured_branch(tmp_path, monkeypatch):
    """The clean-branch JSON envelope used to hardcode `"main"`, which was a
    lie for users on other default branches. It must now read the configured
    branch from kluris.yml."""
    import json
    import yaml
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # Reconfigure the brain's default_branch so the hardcoded "main" would be wrong
    kluris_yml = tmp_path / "my-brain" / "kluris.yml"
    cfg = yaml.safe_load(kluris_yml.read_text())
    cfg["git"]["default_branch"] = "develop"
    kluris_yml.write_text(yaml.dump(cfg), encoding="utf-8")

    result = runner.invoke(cli, ["push", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["brains"][0]["branch"] == "develop"


def test_push_commits_changes(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    result = runner.invoke(cli, ["push", "-m", "add auth"])
    assert "committed" in result.output.lower() or result.exit_code == 0


def test_push_with_message(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    runner.invoke(cli, ["push", "-m", "brain: add auth neuron"])
    import subprocess
    log = subprocess.run(["git", "log", "--oneline", "-1"], cwd=tmp_path / "my-brain", capture_output=True, text=True)
    assert "add auth" in log.stdout


def test_push_no_remote_warning(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    result = runner.invoke(cli, ["push", "-m", "test"])
    # Should succeed (local commit) even without remote
    assert result.exit_code == 0


def test_push_prompts_for_message(tmp_path, monkeypatch):
    """kluris push without -m shows changed files and prompts for a message."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    (tmp_path / "my-brain" / "projects" / "auth.md").write_text("# Auth\n", encoding="utf-8")
    result = runner.invoke(cli, ["push"], input="add auth neuron\n")
    assert result.exit_code == 0
    assert "files changed" in result.output
    assert "Commit message" in result.output


def test_push_no_git_brain(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path),
                        "--description", "test", "--no-git", "--json"])
    result = runner.invoke(cli, ["push", "--json"])
    import json
    data = json.loads(result.output)
    assert result.exit_code == 0
    assert data["brains"][0]["git_enabled"] is False
    assert data["brains"][0]["pushed"] is False
