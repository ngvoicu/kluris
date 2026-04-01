"""Git operations wrapper using subprocess."""

from __future__ import annotations

import subprocess
from pathlib import Path


def _run(args: list[str], cwd: Path) -> subprocess.CompletedProcess:
    """Run a git command and return the result."""
    return subprocess.run(
        args, cwd=cwd, capture_output=True, text=True, check=True,
    )


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
    # Ensure git has user config for commits (needed in temp/CI environments)
    _run(["git", "config", "user.email", "kluris@local"], cwd=path)
    _run(["git", "config", "user.name", "kluris"], cwd=path)


def git_add(path: Path, files: str = "-A") -> None:
    """Stage files. Defaults to staging all."""
    _run(["git", "add", files], cwd=path)


def git_commit(path: Path, message: str) -> None:
    """Create a commit with the given message."""
    _run(["git", "commit", "-m", message], cwd=path)


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


def git_file_last_modified(path: Path, filename: str) -> str | None:
    """Get the last modified date of a file from git log."""
    result = _run(
        ["git", "log", "-1", "--format=%aI", "--", filename],
        cwd=path,
    )
    date = result.stdout.strip()
    return date if date else None


def git_file_created_date(path: Path, filename: str) -> str | None:
    """Get the creation date of a file from git log (first commit that added it)."""
    result = _run(
        ["git", "log", "--diff-filter=A", "--format=%aI", "--", filename],
        cwd=path,
    )
    date = result.stdout.strip()
    return date if date else None
