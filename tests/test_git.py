"""Tests for git operations wrapper."""

from pathlib import Path

from kluris.core.git import (
    git_add,
    git_clone,
    git_commit,
    git_file_created_date,
    git_file_last_modified,
    git_init,
    git_log,
    git_push,
    git_status,
)


# --- [TEST-KLU-05] Git wrapper tests ---


def test_git_init(tmp_path):
    git_init(tmp_path)
    assert (tmp_path / ".git").is_dir()


def test_git_add_commit(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello")
    git_add(tmp_path)
    git_commit(tmp_path, "initial commit")
    entries = git_log(tmp_path)
    assert len(entries) >= 1
    assert "initial commit" in entries[0]["message"]


def test_git_log(tmp_path):
    git_init(tmp_path)
    for i in range(3):
        (tmp_path / f"file{i}.md").write_text(f"content {i}")
        git_add(tmp_path)
        git_commit(tmp_path, f"commit {i}")
    entries = git_log(tmp_path)
    assert len(entries) == 3


def test_git_status_clean(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
    status = git_status(tmp_path)
    assert status == ""


def test_git_status_dirty(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
    (tmp_path / "test.md").write_text("modified")
    status = git_status(tmp_path)
    assert "test.md" in status


def test_git_push(tmp_path, bare_remote):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
    import subprocess
    subprocess.run(
        ["git", "remote", "add", "origin", str(bare_remote)],
        cwd=tmp_path, capture_output=True,
    )
    git_push(tmp_path, "origin", "main")
    # Verify remote has the commit
    result = subprocess.run(
        ["git", "log", "--oneline"],
        cwd=bare_remote, capture_output=True, text=True,
    )
    assert "initial" in result.stdout


def test_git_clone(tmp_path, bare_remote):
    # Create a source repo and push to bare
    source = tmp_path / "source"
    source.mkdir()
    git_init(source)
    (source / "test.md").write_text("hello")
    git_add(source)
    git_commit(source, "initial")
    import subprocess
    subprocess.run(
        ["git", "remote", "add", "origin", str(bare_remote)],
        cwd=source, capture_output=True,
    )
    git_push(source, "origin", "main")

    # Clone
    clone_path = tmp_path / "cloned"
    git_clone(str(bare_remote), clone_path)
    assert (clone_path / "test.md").exists()
    assert (clone_path / "test.md").read_text() == "hello"


def test_git_file_last_modified(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("v1")
    git_add(tmp_path)
    git_commit(tmp_path, "first")
    date = git_file_last_modified(tmp_path, "test.md")
    assert date is not None
    # Should be an ISO-ish date string
    assert "-" in date


def test_git_file_created_date(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("v1")
    git_add(tmp_path)
    git_commit(tmp_path, "first")
    (tmp_path / "test.md").write_text("v2")
    git_add(tmp_path)
    git_commit(tmp_path, "second")
    date = git_file_created_date(tmp_path, "test.md")
    assert date is not None
    assert "-" in date
