"""Tests for kluris clone command."""

import subprocess

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_brain_config, read_global_config


def _create_remote_brain(tmp_path, monkeypatch):
    """Create a brain, push to bare remote, return remote path."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "source-brain", tmp_path)

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
        ["create", "source-brain", "--path", str(tmp_path), "--description", "test",
         "--branch", branch_name, "--remote", str(tmp_path / "remote.git"), "--json"],
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
    assert result.exit_code == 0


def test_clone_creates_local_branch_when_name_not_on_remote(tmp_path, monkeypatch):
    """User clones from main but asks for a new local branch to push to later.

    Previously this crashed with a raw subprocess error because
    `git checkout <nonexistent>` failed. The fix creates the branch from
    the current HEAD so the user can start committing on it immediately.
    """
    bare = _create_remote_brain(tmp_path, monkeypatch)
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-new-branch"

    result = runner.invoke(cli, ["clone", str(bare), str(dest), "--branch", "feature-x"])

    assert result.exit_code == 0, result.output
    actual_branch = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=dest, capture_output=True, text=True,
    ).stdout.strip()
    assert actual_branch == "feature-x"


def test_clone_checks_out_existing_remote_branch(tmp_path, monkeypatch):
    """When the user names a branch that exists on origin, check it out (track remote)."""
    bare = _create_remote_brain_on_branch(tmp_path, monkeypatch, "develop")
    _use_fresh_clone_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    dest = tmp_path / "cloned-existing-branch"

    result = runner.invoke(cli, ["clone", str(bare), str(dest), "--branch", "develop"])

    assert result.exit_code == 0, result.output
    actual_branch = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=dest, capture_output=True, text=True,
    ).stdout.strip()
    assert actual_branch == "develop"


def test_clone_surfaces_git_stderr_when_remote_unreachable(tmp_path, monkeypatch):
    """git clone failures must show the actual git error (not a Python traceback).

    Previously the user saw a CalledProcessError stack with the raw `git clone`
    exit code and no hint at WHY (auth? network? repo missing?). Surfacing
    stderr lets the user act: configure a PAT, fix the URL, check connectivity.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    bogus_url = str(tmp_path / "does-not-exist.git")
    dest = tmp_path / "dest"

    result = runner.invoke(cli, ["clone", bogus_url, str(dest)])

    assert result.exit_code != 0
    assert "git clone failed" in result.output
    assert "missing credentials" in result.output or "does not exist" in result.output.lower() or "not a git repository" in result.output.lower()


def test_clone_cleans_up_partial_clone_on_non_brain_repo(tmp_path, monkeypatch):
    """If the cloned repo is not a brain, remove the clone so the user can retry."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
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

    dest = tmp_path / "dest"
    runner = CliRunner()
    result = runner.invoke(cli, ["clone", str(bare), str(dest)])

    assert result.exit_code != 0
    assert "not a Kluris brain" in result.output
    assert not dest.exists(), "partial clone should be removed on failure"
