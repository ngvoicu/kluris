"""Git operations wrapper using subprocess."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


def _run(args: list[str], cwd: Path) -> subprocess.CompletedProcess:
    """Run a git command and return the result."""
    return subprocess.run(
        args, cwd=cwd, capture_output=True, text=True, check=True,
    )


def _read_git_config(path: Path, key: str) -> str | None:
    """Return a git config value, or None when it is unset."""
    result = subprocess.run(
        ["git", "config", "--get", key],
        cwd=path,
        capture_output=True,
        text=True,
        check=False,
    )
    value = result.stdout.strip()
    return value if result.returncode == 0 and value else None


def _commit_env(path: Path) -> dict[str, str]:
    """Provide transient identity only when git config does not already define one."""
    env = os.environ.copy()
    if _read_git_config(path, "user.name") and _read_git_config(path, "user.email"):
        return env

    env.setdefault("GIT_AUTHOR_NAME", "kluris")
    env.setdefault("GIT_AUTHOR_EMAIL", "kluris@local")
    env.setdefault("GIT_COMMITTER_NAME", env["GIT_AUTHOR_NAME"])
    env.setdefault("GIT_COMMITTER_EMAIL", env["GIT_AUTHOR_EMAIL"])
    return env


def is_git_repo(path: Path) -> bool:
    """Return True when path is inside a git work tree."""
    try:
        result = _run(["git", "rev-parse", "--is-inside-work-tree"], cwd=path)
    except (FileNotFoundError, subprocess.CalledProcessError):
        return False
    return result.stdout.strip() == "true"


def git_init(path: Path) -> None:
    """Initialize a git repo at path."""
    _run(["git", "init"], cwd=path)
    # Set default branch to main
    _run(["git", "checkout", "-b", "main"], cwd=path)


def git_add(path: Path, files: str = "-A") -> None:
    """Stage files. Defaults to staging all."""
    _run(["git", "add", files], cwd=path)


def git_commit(path: Path, message: str) -> None:
    """Create a commit with the given message."""
    subprocess.run(
        ["git", "commit", "-m", message],
        cwd=path,
        capture_output=True,
        text=True,
        check=True,
        env=_commit_env(path),
    )


def git_log(path: Path, limit: int = 10) -> list[dict]:
    """Return recent commits as list of {hash, message, date} dicts."""
    result = _run(
        ["git", "log", f"-{limit}", "--format=%H|%s|%aI"],
        cwd=path,
    )
    entries = []
    for line in result.stdout.strip().splitlines():
        if "|" in line:
            parts = line.split("|", 2)
            entries.append({
                "hash": parts[0],
                "message": parts[1],
                "date": parts[2] if len(parts) > 2 else "",
            })
    return entries


def git_status(path: Path) -> str:
    """Return short git status output. Empty string if clean."""
    result = _run(["git", "status", "--short"], cwd=path)
    return result.stdout.strip()


def git_push(path: Path, remote: str = "origin", branch: str = "main") -> None:
    """Push to remote."""
    _run(["git", "push", remote, branch], cwd=path)


def git_clone(url: str, dest: Path) -> None:
    """Clone a repo to dest path."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    _run(["git", "clone", url, str(dest)], cwd=dest.parent)


def checkout_or_create_branch(dest: Path, branch: str) -> None:
    """Checkout ``branch`` inside ``dest``.

    If the branch exists on ``origin``, check it out (tracks the remote).
    Otherwise create a new local branch from the current HEAD -- this lets
    a user clone from main and name a new branch to push to in the future
    without the clone failing on a non-existent branch.
    """
    result = subprocess.run(
        ["git", "ls-remote", "--heads", "origin", branch],
        cwd=dest, capture_output=True, text=True, check=False,
    )
    if result.returncode == 0 and result.stdout.strip():
        _run(["git", "checkout", branch], cwd=dest)
    else:
        _run(["git", "checkout", "-b", branch], cwd=dest)


def git_log_file_dates(path: Path) -> tuple[dict[str, str], dict[str, str]]:
    """Return ``(latest_by_path, created_by_path)`` from a single git log walk.

    Single subprocess invocation:

        git log --format="COMMIT %aI" --name-only HEAD

    The output walks commits newest-first. For each file path:

    - ``latest`` is set on the FIRST occurrence (= newest commit that touched it)
    - ``created`` is overwritten on EVERY occurrence (= oldest commit, since we
      keep overwriting until the last/oldest occurrence wins)

    Uses ``%aI`` (author ISO date). Returns ``({}, {})`` if not a git repo
    or if ``git log`` fails. Caller should typically guard with
    ``is_git_repo()`` first to short-circuit cleanly.
    """
    try:
        result = _run(
            ["git", "log", "--format=COMMIT %aI", "--name-only", "HEAD"],
            cwd=path,
        )
    except Exception:
        return ({}, {})

    if result.returncode != 0:
        return ({}, {})

    latest: dict[str, str] = {}
    created: dict[str, str] = {}
    current_date: str | None = None

    for raw_line in result.stdout.splitlines():
        line = raw_line.strip()
        if line.startswith("COMMIT "):
            current_date = line[len("COMMIT "):].strip()
            continue
        if not line:
            continue
        # File path line. Defensive: if no current_date, skip.
        if current_date is None:
            continue
        # Newest-first walk: first occurrence is most recent.
        if line not in latest:
            latest[line] = current_date
        # Always overwrite: last occurrence (= oldest commit) wins.
        created[line] = current_date

    return (latest, created)
