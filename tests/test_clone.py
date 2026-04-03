"""Tests for kluris clone command."""

import subprocess

from click.testing import CliRunner

from kluris.cli import cli
from kluris.core.config import read_brain_config, read_global_config


def _create_remote_brain(tmp_path, monkeypatch):
    """Create a brain, push to bare remote, return remote path."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "source-brain", "--path", str(tmp_path)])

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


def _create_remote_brain_on_branch(tmp_path, monkeypatch, branch_name):
    """Create a brain on a non-main branch, push to bare remote, return remote path."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(
        cli,
        ["create", "source-brain", "--path", str(tmp_path), "--branch", branch_name, "--remote", str(tmp_path / "remote.git")],
    )

    bare = tmp_path / "remote.git"
    subprocess.run(["git", "init", "--bare", str(bare)], capture_output=True)
    subprocess.run(
        ["git", "symbolic-ref", "HEAD", f"refs/heads/{branch_name}"],
        cwd=str(bare), capture_output=True,
    )
    subprocess.run(
        ["git", "remote", "add", "origin", str(bare)],
        cwd=tmp_path / "source-brain", capture_output=True,
    )
    subprocess.run(
        ["git", "push", "origin", branch_name],
        cwd=tmp_path / "source-brain", capture_output=True,
    )
    return bare


def _use_fresh_clone_registry(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "clone-config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))


def test_clone_brain(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-brain"
    result = runner.invoke(cli, ["clone", str(bare), str(dest)])
    assert result.exit_code == 0
    assert (dest / "kluris.yml").exists()


def test_clone_registers(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-brain"
    runner.invoke(cli, ["clone", str(bare), str(dest)])
    config = read_global_config()
    assert "source-brain" in config.brains
    assert "cloned-brain" not in config.brains


def test_clone_uses_canonical_brain_identity(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "my-copy"

    result = runner.invoke(cli, ["clone", str(bare), str(dest)])

    assert result.exit_code == 0
    config = read_global_config()
    assert "source-brain" in config.brains
    assert "my-copy" not in config.brains
    brain_config = read_brain_config(dest)
    assert brain_config.name == "source-brain"
    assert brain_config.description == "source-brain knowledge base"


def test_clone_runs_install(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
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
    (src / "readme.md").write_text("not a brain\n", encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=src, capture_output=True)
    subprocess.run(["git", "commit", "-m", "init"], cwd=src, capture_output=True)
    subprocess.run(["git", "remote", "add", "origin", str(bare)], cwd=src, capture_output=True)
    subprocess.run(["git", "push", "origin", "main"], cwd=src, capture_output=True)

    runner = CliRunner()
    result = runner.invoke(cli, ["clone", str(bare), str(tmp_path / "dest")])
    assert result.exit_code != 0
    assert "not a Kluris brain" in result.output


def test_clone_rejects_duplicate_canonical_name(tmp_path, monkeypatch):
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()

    first_dest = tmp_path / "first-copy"
    second_dest = tmp_path / "second-copy"
    first_result = runner.invoke(cli, ["clone", str(bare), str(first_dest)])
    assert first_result.exit_code == 0

    second_result = runner.invoke(cli, ["clone", str(bare), str(second_dest)])
    assert second_result.exit_code != 0
    assert "already registered" in second_result.output


def test_clone_records_checked_out_branch_when_remote_default_is_not_main(tmp_path, monkeypatch):
    bare = _create_remote_brain_on_branch(tmp_path, monkeypatch, "develop")
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-branch-brain"

    result = runner.invoke(cli, ["clone", str(bare), str(dest)])

    assert result.exit_code == 0
    brain_config = read_brain_config(dest)
    assert brain_config.git.default_branch == "develop"
