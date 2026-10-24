"""Embedded companion playbooks for spec-worthy work.

Companions are bundled with Kluris and copied into ``~/.kluris`` on opt-in.
Each one is a skill folder: ``SKILL.md`` plus the ``references/`` and
``assets/`` it points at, and nothing else. Kluris does not version-track
companions; refreshing is an unconditional re-copy of the whole folder from the
installed Kluris package.
"""

from __future__ import annotations

import contextlib
import shutil
from pathlib import Path

from kluris.core.config import GlobalConfig, read_brain_config

KNOWN = ("specmint-core", "specmint-tdd", "specmint-core-html", "specmint-tdd-html")
_VENDORED = Path(__file__).parent.parent / "vendored"
_HOME_COMPANIONS_REL = Path(".kluris") / "companions"
# The only top-level entries a companion folder may hold.
_RUNTIME_FILES = ("SKILL.md",)
_RUNTIME_DIRS = ("references", "assets")


def normalize(names: list[str] | tuple[str, ...] | None) -> list[str]:
    """Return known companion names, de-duped in canonical order."""
    selected = set(names or [])
    return [name for name in KNOWN if name in selected]


def _validate_name(name: str) -> None:
    if name not in KNOWN:
        allowed = ", ".join(KNOWN)
        raise ValueError(f"Unknown companion '{name}'. Allowed: {allowed}")


def vendored_dir(name: str) -> Path:
    """Return the vendored directory for a known companion."""
    _validate_name(name)
    return _VENDORED / name


def installed_dir(name: str, home: Path) -> Path:
    """Return the runtime install directory for a known companion."""
    _validate_name(name)
    return home / _HOME_COMPANIONS_REL / name


def is_installed(name: str, home: Path) -> bool:
    """Return True when the companion's runtime SKILL.md exists."""
    return (installed_dir(name, home) / "SKILL.md").is_file()


def _verify_runtime_dir(path: Path) -> None:
    skill = path / "SKILL.md"
    if not skill.is_file():
        raise FileNotFoundError(f"Companion copy is missing {skill}")
    extras = sorted(
        p.name for p in path.iterdir()
        if not (
            (p.name in _RUNTIME_FILES and p.is_file())
            or (p.name in _RUNTIME_DIRS and p.is_dir())
        )
    )
    if extras:
        allowed = ", ".join([*_RUNTIME_FILES, *(f"{d}/" for d in _RUNTIME_DIRS)])
        raise OSError(
            f"Companion runtime dir may contain only {allowed}; found: {', '.join(extras)}"
        )
    for p in path.rglob("*"):
        if p.is_symlink() or p.name.startswith("."):
            raise OSError(f"Companion runtime dir contains a link or hidden file: {p}")


def _ignore_junk(_dir: str, names: list[str]) -> set[str]:
    return {n for n in names if n.startswith(".") or n == "__pycache__"}


def _stage_copy(src_dir: Path, staging: Path) -> None:
    """Copy the allowlisted entries of ``src_dir`` into a fresh ``staging``.

    Links are copied as links, never followed, so ``_verify_runtime_dir``
    rejects them instead of installing whatever they point at.
    """
    staging.mkdir(parents=True)
    for name in (*_RUNTIME_FILES, *_RUNTIME_DIRS):
        if (src_dir / name).is_symlink():
            raise OSError(f"Vendored companion entry is a link: {src_dir / name}")
    for name in _RUNTIME_FILES:
        shutil.copy2(src_dir / name, staging / name)
    for name in _RUNTIME_DIRS:
        if (src_dir / name).is_dir():
            shutil.copytree(
                src_dir / name, staging / name, symlinks=True, ignore=_ignore_junk
            )


def _remove(path: Path) -> None:
    """Best-effort removal of whatever sits at ``path``: file, link or directory."""
    with contextlib.suppress(OSError):
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.is_dir():
            shutil.rmtree(path)


def install(name: str, home: Path) -> None:
    """Stage a copy of the vendored folder, verify it, then replace the runtime dir.

    A failure at any step leaves the previous install (folder or older
    single-file copy) in place.
    """
    src_dir = vendored_dir(name)
    src_skill = src_dir / "SKILL.md"
    if not src_skill.is_file():
        raise FileNotFoundError(f"Vendored companion missing SKILL.md: {src_skill}")

    dest = installed_dir(name, home)
    dest_parent = dest.parent
    dest_parent.mkdir(parents=True, exist_ok=True)
    staging = dest_parent / f".{name}.tmp"
    backup = dest_parent / f".{name}.old"

    _remove(staging)
    _remove(backup)

    try:
        _stage_copy(src_dir, staging)
        _verify_runtime_dir(staging)
        if dest.exists() or dest.is_symlink():
            dest.replace(backup)
        staging.replace(dest)
    except Exception:
        _remove(staging)
        if (backup.exists() or backup.is_symlink()) and not dest.exists():
            backup.replace(dest)
        raise
    finally:
        _remove(backup)


def uninstall(name: str, home: Path) -> None:
    """Remove the runtime directory for this companion."""
    shutil.rmtree(installed_dir(name, home), ignore_errors=True)


def refresh(name: str, home: Path) -> None:
    """Refresh a companion by unconditionally re-copying the vendored folder."""
    install(name, home)


def installed(home: Path) -> list[str]:
    """Return known companions with runtime directories already present."""
    return [name for name in KNOWN if installed_dir(name, home).exists()]


def referenced(config: GlobalConfig) -> list[str]:
    """Return known companions referenced by registered brain configs."""
    refs: list[str] = []
    for entry in config.brains.values():
        brain_path = Path(entry.path)
        if not (brain_path / "kluris.yml").exists():
            continue
        try:
            refs.extend(read_brain_config(brain_path).companions)
        except Exception:
            continue
    return normalize(refs)
