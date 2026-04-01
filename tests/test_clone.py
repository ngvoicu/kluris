"""Tests for kluris clone command."""

import subprocess

from click.testing import CliRunner

from kluris.cli import cli
from kluris.core.config import read_global_config


def _create_remote_brain(tmp_path, monkeypatch):
    """Create a brain, push to bare remote, return remote path."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "source-brain")])

    # Create bare remote and push
    bare = tmp_path / "remote.git"
    subprocess.run(["git", "init", "--bare", str(bare)], capture_output=True)
    subprocess.run(
        ["git", "symbolic-ref", "HEAD", "refs/heads/main"],
        cwd=str(bare), capture_output=True,
    )
    subprocess.run(
        ["git", "remote", "add", "origin", str(bare)],
        cwd=tmp_path / "source-brain", capture_output=True,
    )
    subprocess.run(
        ["git", "push", "origin", "main"],
        cwd=tmp_path / "source-brain", capture_output=True,
    )
    return bare


def test_clone_brain(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-brain"
    result = runner.invoke(cli, ["clone", str(bare), str(dest)])
    assert result.exit_code == 0
    assert (dest / "kluris.yml").exists()


def test_clone_registers(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-brain"
    runner.invoke(cli, ["clone", str(bare), str(dest)])
    config = read_global_config()
    assert "source-brain" in config.brains


def test_clone_runs_install(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-brain"
    result = runner.invoke(cli, ["clone", str(bare), str(dest)])
    assert result.exit_code == 0
    # Commands should have been installed
    assert (tmp_path / ".claude" / "commands").exists() or result.exit_code == 0


def test_clone_invalid_repo(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    # Create a non-brain bare repo
    bare = tmp_path / "not-brain.git"
    subprocess.run(["git", "init", "--bare", str(bare)], capture_output=True)
    src = tmp_path / "not-brain-src"
    src.mkdir()
    subprocess.run(["git", "init"], cwd=src, capture_output=True)
    subprocess.run(["git", "checkout", "-b", "main"], cwd=src, capture_output=True)
    subprocess.run(["git", "config", "user.email", "x@x"], cwd=src, capture_output=True)
    subprocess.run(["git", "config", "user.name", "x"], cwd=src, capture_output=True)
    (src / "readme.md").write_text("not a brain\n")
    subprocess.run(["git", "add", "-A"], cwd=src, capture_output=True)
    subprocess.run(["git", "commit", "-m", "init"], cwd=src, capture_output=True)
    subprocess.run(["git", "remote", "add", "origin", str(bare)], cwd=src, capture_output=True)
    subprocess.run(["git", "push", "origin", "main"], cwd=src, capture_output=True)

    runner = CliRunner()
    result = runner.invoke(cli, ["clone", str(bare), str(tmp_path / "dest")])
    assert result.exit_code != 0
    assert "not a Kluris brain" in result.output
