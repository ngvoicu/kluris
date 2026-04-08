"""Tests for git operations wrapper."""

import subprocess
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


def test_git_init_does_not_override_local_identity(tmp_path):
    git_init(tmp_path)
    user_name = subprocess.run(
        ["git", "config", "--local", "--get", "user.name"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    user_email = subprocess.run(
        ["git", "config", "--local", "--get", "user.email"],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    assert user_name.returncode != 0
    assert user_email.returncode != 0


def test_git_add_commit(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "initial commit")
    entries = git_log(tmp_path)
    assert len(entries) >= 1
    assert "initial commit" in entries[0]["message"]


def test_git_log(tmp_path):
    git_init(tmp_path)
    for i in range(3):
        (tmp_path / f"file{i}.md").write_text(f"content {i}", encoding="utf-8")
        git_add(tmp_path)
        git_commit(tmp_path, f"commit {i}")
    entries = git_log(tmp_path)
    assert len(entries) == 3


def test_git_status_clean(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
    status = git_status(tmp_path)
    assert status == ""


def test_git_status_dirty(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
    (tmp_path / "test.md").write_text("modified", encoding="utf-8")
    status = git_status(tmp_path)
    assert "test.md" in status


def test_git_push(tmp_path, bare_remote):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("hello", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "initial")
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
    (source / "test.md").write_text("hello", encoding="utf-8")
    git_add(source)
    git_commit(source, "initial")
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
    (tmp_path / "test.md").write_text("v1", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "first")
    date = git_file_last_modified(tmp_path, "test.md")
    assert date is not None
    # Should be an ISO-ish date string
    assert "-" in date


def test_git_file_created_date(tmp_path):
    git_init(tmp_path)
    (tmp_path / "test.md").write_text("v1", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "first")
    (tmp_path / "test.md").write_text("v2", encoding="utf-8")
    git_add(tmp_path)
    git_commit(tmp_path, "second")
    date = git_file_created_date(tmp_path, "test.md")
    assert date is not None
    assert "-" in date


# --- Batch git_log_file_dates ---


def _commit_with_date(repo, message, iso_date):
    """Helper: stage and commit at a specific GIT_AUTHOR_DATE / GIT_COMMITTER_DATE."""
    import os
    env = os.environ.copy()
    env["GIT_AUTHOR_DATE"] = iso_date
    env["GIT_COMMITTER_DATE"] = iso_date
    env["GIT_AUTHOR_NAME"] = "test"
    env["GIT_AUTHOR_EMAIL"] = "test@test"
    env["GIT_COMMITTER_NAME"] = "test"
    env["GIT_COMMITTER_EMAIL"] = "test@test"
    subprocess.run(["git", "add", "-A"], cwd=repo, env=env, capture_output=True, check=True)
    subprocess.run(["git", "commit", "-m", message], cwd=repo, env=env, capture_output=True, check=True)


def test_git_log_file_dates_returns_two_maps(tmp_path):
    """Returns (latest_by_path, created_by_path) populated from one git log walk.

    latest = newest commit that touched each file.
    created = oldest commit that touched each file (for files added once,
              this is the file's add commit).
    """
    from kluris.core.git import git_log_file_dates

    git_init(tmp_path)

    # Commit 1 (oldest): adds a.md
    (tmp_path / "a.md").write_text("v1", encoding="utf-8")
    _commit_with_date(tmp_path, "add a", "2026-01-01T00:00:00Z")

    # Commit 2: modifies a.md, adds b.md
    (tmp_path / "a.md").write_text("v2", encoding="utf-8")
    (tmp_path / "b.md").write_text("v1", encoding="utf-8")
    _commit_with_date(tmp_path, "modify a, add b", "2026-02-01T00:00:00Z")

    # Commit 3: modifies b.md
    (tmp_path / "b.md").write_text("v2", encoding="utf-8")
    _commit_with_date(tmp_path, "modify b", "2026-03-01T00:00:00Z")

    # Commit 4 (newest): adds c.md
    (tmp_path / "c.md").write_text("v1", encoding="utf-8")
    _commit_with_date(tmp_path, "add c", "2026-04-01T00:00:00Z")

    latest, created = git_log_file_dates(tmp_path)

    # latest_by_path: most recent commit per file
    assert latest["a.md"].startswith("2026-02-01")  # last touched in commit 2
    assert latest["b.md"].startswith("2026-03-01")  # last touched in commit 3
    assert latest["c.md"].startswith("2026-04-01")  # only touched in commit 4

    # created_by_path: oldest commit per file
    assert created["a.md"].startswith("2026-01-01")  # added in commit 1
    assert created["b.md"].startswith("2026-02-01")  # added in commit 2
    assert created["c.md"].startswith("2026-04-01")  # added in commit 4


def test_git_log_file_dates_empty_repo_returns_empty_maps(tmp_path):
    """An initialized repo with no commits returns ({}, {}) without crashing."""
    from kluris.core.git import git_log_file_dates

    git_init(tmp_path)
    latest, created = git_log_file_dates(tmp_path)
    assert latest == {}
    assert created == {}


def test_git_log_file_dates_non_git_dir_returns_empty_maps(tmp_path):
    """A non-git directory returns ({}, {}) without crashing."""
    from kluris.core.git import git_log_file_dates

    latest, created = git_log_file_dates(tmp_path)
    assert latest == {}
    assert created == {}


def test_git_log_file_dates_uses_author_date_aI_not_committer_date(tmp_path):
    """The batch helper must use %aI (author date) to match git_file_last_modified
    and git_file_created_date, NOT %cI (committer date)."""
    import os
    from kluris.core.git import git_log_file_dates, git_file_last_modified

    git_init(tmp_path)
    (tmp_path / "test.md").write_text("v1", encoding="utf-8")

    # Commit with DIFFERENT author and committer dates so we can tell them apart
    env = os.environ.copy()
    env["GIT_AUTHOR_DATE"] = "2025-01-01T00:00:00Z"
    env["GIT_COMMITTER_DATE"] = "2026-12-31T00:00:00Z"
    env["GIT_AUTHOR_NAME"] = "test"
    env["GIT_AUTHOR_EMAIL"] = "test@test"
    env["GIT_COMMITTER_NAME"] = "test"
    env["GIT_COMMITTER_EMAIL"] = "test@test"
    subprocess.run(["git", "add", "-A"], cwd=tmp_path, env=env, capture_output=True, check=True)
    subprocess.run(["git", "commit", "-m", "x"], cwd=tmp_path, env=env, capture_output=True, check=True)

    latest, created = git_log_file_dates(tmp_path)
    # Both should match the AUTHOR date, not the committer date
    assert latest["test.md"].startswith("2025-01-01")
    assert created["test.md"].startswith("2025-01-01")

    # Cross-check: git_file_last_modified should also report the author date
    # (this proves the batch matches the per-file helper byte-for-byte)
    per_file = git_file_last_modified(tmp_path, "test.md")
    assert per_file.startswith("2025-01-01")


def test_git_log_file_dates_rename_history_parity(tmp_path):
    """Renamed files: the batch helper must produce dates that match the
    per-file `git_file_last_modified` / `git_file_created_date` helpers.

    Without `-M` (rename detection), `git log -- newpath` returns the rename
    commit's date, not the original creation date. The batch behaves the same
    way because it walks `git log --name-only HEAD` which lists the new path
    starting from the rename commit. This test pins that parity.
    """
    import os
    from kluris.core.git import (
        git_log_file_dates,
        git_file_last_modified,
        git_file_created_date,
    )

    git_init(tmp_path)

    # Commit 1: add old-name.md
    (tmp_path / "old-name.md").write_text("v1", encoding="utf-8")
    _commit_with_date(tmp_path, "add old", "2026-01-15T00:00:00Z")

    # Commit 2: rename old-name.md → new-name.md (via git mv)
    env = os.environ.copy()
    env["GIT_AUTHOR_DATE"] = "2026-03-15T00:00:00Z"
    env["GIT_COMMITTER_DATE"] = "2026-03-15T00:00:00Z"
    env["GIT_AUTHOR_NAME"] = "test"
    env["GIT_AUTHOR_EMAIL"] = "test@test"
    env["GIT_COMMITTER_NAME"] = "test"
    env["GIT_COMMITTER_EMAIL"] = "test@test"
    subprocess.run(
        ["git", "mv", "old-name.md", "new-name.md"],
        cwd=tmp_path, env=env, capture_output=True, check=True,
    )
    subprocess.run(
        ["git", "commit", "-m", "rename"],
        cwd=tmp_path, env=env, capture_output=True, check=True,
    )

    latest, created = git_log_file_dates(tmp_path)

    # The batch's view of new-name.md
    batch_latest = latest.get("new-name.md")
    batch_created = created.get("new-name.md")

    # Per-file helpers' view of new-name.md
    per_file_latest = git_file_last_modified(tmp_path, "new-name.md")
    per_file_created = git_file_created_date(tmp_path, "new-name.md")

    # The batch must match the per-file helpers byte-for-byte (within the
    # date prefix; both return ISO8601 strings)
    assert batch_latest is not None
    assert per_file_latest is not None
    assert batch_latest.startswith(per_file_latest[:10])

    assert batch_created is not None
    assert per_file_created is not None
    # Note: per_file_created may return multi-line stdout if there were
    # multiple add commits; compare just the first 10 chars (the date)
    assert batch_created.startswith(per_file_created[:10])

    # Both should report the rename commit date for new-name.md, NOT the
    # original creation commit
    assert batch_latest.startswith("2026-03-15")
    assert batch_created.startswith("2026-03-15")
