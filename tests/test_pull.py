"""Tests for kluris pull command."""

import json
import subprocess

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def _bootstrap_brain_with_remote(tmp_path, monkeypatch, name="test-brain"):
    """Register a kluris brain with origin pointing at a bare remote.

    The brain is already pushed to the remote on main so subsequent pulls
    have a valid upstream to sync with.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, name, tmp_path)
    brain = tmp_path / name

    # Raw subprocess git commits in these tests bypass kluris's _commit_env,
    # so set the brain's local git identity explicitly.
    subprocess.run(["git", "config", "user.email", "t@t"], cwd=brain, capture_output=True, check=True)
    subprocess.run(["git", "config", "user.name", "t"], cwd=brain, capture_output=True, check=True)

    bare = tmp_path / f"{name}.git"
    subprocess.run(["git", "init", "--bare", str(bare)], capture_output=True, check=True)
    subprocess.run(
        ["git", "symbolic-ref", "HEAD", "refs/heads/main"],
        cwd=str(bare), capture_output=True, check=True,
    )
    subprocess.run(
        ["git", "remote", "add", "origin", str(bare)],
        cwd=brain, capture_output=True, check=True,
    )
    subprocess.run(
        ["git", "push", "-u", "origin", "main"],
        cwd=brain, capture_output=True, check=True,
    )
    return brain, bare


def _push_change_via_scratch_clone(bare, tmp_path, filename, content, branch="main", scratch_name=None):
    """Add a commit to `bare` (on `branch`) via a separate clone."""
    scratch = tmp_path / (scratch_name or f"scratch-{filename.replace('/', '-')}")
    subprocess.run(["git", "clone", str(bare), str(scratch)], capture_output=True, check=True)
    subprocess.run(["git", "config", "user.email", "t@t"], cwd=scratch, capture_output=True, check=True)
    subprocess.run(["git", "config", "user.name", "t"], cwd=scratch, capture_output=True, check=True)
    if branch != "main":
        subprocess.run(["git", "checkout", "-b", branch], cwd=scratch, capture_output=True, check=True)
    (scratch / filename).parent.mkdir(parents=True, exist_ok=True)
    (scratch / filename).write_text(content, encoding="utf-8")
    subprocess.run(["git", "add", "-A"], cwd=scratch, capture_output=True, check=True)
    subprocess.run(
        ["git", "commit", "-m", f"add {filename}"],
        cwd=scratch, capture_output=True, check=True,
    )
    subprocess.run(["git", "push", "origin", branch], cwd=scratch, capture_output=True, check=True)
    return scratch


def test_pull_no_git_brain_is_reported(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, [
        "create", "ng-brain", "--path", str(tmp_path),
        "--description", "t", "--no-git", "--json",
    ])
    result = runner.invoke(cli, ["pull", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["brains"][0]["git_enabled"] is False


def test_pull_errors_on_dirty_tree(tmp_path, monkeypatch):
    brain, _ = _bootstrap_brain_with_remote(tmp_path, monkeypatch)
    (brain / "projects" / "dirty.md").write_text("uncommitted\n", encoding="utf-8")
    runner = CliRunner()
    result = runner.invoke(cli, ["pull"])
    assert result.exit_code != 0
    assert "uncommitted changes" in result.output.lower()


def test_pull_clean_fast_forward_on_default_branch(tmp_path, monkeypatch):
    """On main, upstream has a new commit -- kluris pull fast-forwards."""
    brain, bare = _bootstrap_brain_with_remote(tmp_path, monkeypatch)
    _push_change_via_scratch_clone(bare, tmp_path, "projects/new.md", "# New\n")

    result = CliRunner().invoke(cli, ["pull", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["ok"] is True
    assert data["brains"][0]["pulled"] is True
    assert data["brains"][0]["files_changed"] >= 1
    assert (brain / "projects" / "new.md").exists()


def test_pull_on_default_branch_does_not_prompt(tmp_path, monkeypatch):
    """When current == default (main), the merge-default prompt never fires."""
    from kluris import cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    brain, bare = _bootstrap_brain_with_remote(tmp_path, monkeypatch)
    _push_change_via_scratch_clone(bare, tmp_path, "projects/a.md", "# A\n")

    result = CliRunner().invoke(cli, ["pull"], input="")
    assert result.exit_code == 0
    assert "also merge" not in result.output.lower()


def test_pull_detects_conflicts_and_reports_them(tmp_path, monkeypatch):
    """Overlapping edits on the same file -> pull surfaces a conflict block."""
    brain, bare = _bootstrap_brain_with_remote(tmp_path, monkeypatch)

    # Remote changes glossary.md to one thing
    _push_change_via_scratch_clone(
        bare, tmp_path,
        "glossary.md",
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n# Glossary\n\n- REMOTE VALUE\n",
    )

    # Local changes glossary.md to a conflicting value and commits locally
    (brain / "glossary.md").write_text(
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n# Glossary\n\n- LOCAL VALUE\n",
        encoding="utf-8",
    )
    subprocess.run(["git", "add", "-A"], cwd=brain, capture_output=True, check=True)
    subprocess.run(
        ["git", "commit", "-m", "local glossary edit"],
        cwd=brain, capture_output=True, check=True,
    )

    result = CliRunner().invoke(cli, ["pull"])
    assert result.exit_code != 0
    assert "merge conflicts" in result.output.lower()
    assert "glossary.md" in result.output
    assert "merge --abort" in result.output


def test_pull_brain_all_fans_out(tmp_path, monkeypatch):
    """--brain all iterates every registered brain in sequence."""
    brain_a, bare_a = _bootstrap_brain_with_remote(tmp_path, monkeypatch, name="brain-a")
    brain_b, bare_b = _bootstrap_brain_with_remote(tmp_path, monkeypatch, name="brain-b")
    _push_change_via_scratch_clone(bare_a, tmp_path, "projects/x.md", "# X\n", scratch_name="sa")
    _push_change_via_scratch_clone(bare_b, tmp_path, "projects/y.md", "# Y\n", scratch_name="sb")

    result = CliRunner().invoke(cli, ["pull", "--brain", "all", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    names = {b["name"] for b in data["brains"]}
    assert names == {"brain-a", "brain-b"}
    for b in data["brains"]:
        assert b["pulled"] is True
