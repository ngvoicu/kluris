"""Embedded companion playbooks for spec-worthy work.

Companions are bundled with Kluris and copied into ``~/.kluris`` on opt-in.
They are intentionally tiny: each installed companion directory contains one
runtime file, ``SKILL.md``. Kluris does not version-track companions; refreshing
is an unconditional re-copy from the installed Kluris package.
"""

from __future__ import annotations

import shutil
from pathlib import Path

from kluris.core.config import GlobalConfig, read_brain_config

KNOWN = ("specmint-core", "specmint-tdd", "specmint-core-html", "specmint-tdd-html")
_VENDORED = Path(__file__).parent.parent / "vendored"
_HOME_COMPANIONS_REL = Path(".kluris") / "companions"


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
    extras = [p for p in path.iterdir() if p.name != "SKILL.md"]
    if extras:
        names = ", ".join(sorted(p.name for p in extras))
        raise OSError(f"Companion runtime dir must contain only SKILL.md; found: {names}")


def install(name: str, home: Path) -> None:
    """Stage a vendored SKILL.md copy, verify it, then replace the runtime dir."""
    src_dir = vendored_dir(name)
    src_skill = src_dir / "SKILL.md"
    if not src_skill.is_file():
        raise FileNotFoundError(f"Vendored companion missing SKILL.md: {src_skill}")

    dest = installed_dir(name, home)
    dest_parent = dest.parent
    dest_parent.mkdir(parents=True, exist_ok=True)
    staging = dest_parent / f".{name}.tmp"
    backup = dest_parent / f".{name}.old"

    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(backup, ignore_errors=True)

    staging.mkdir(parents=True)
    try:
        shutil.copy2(src_skill, staging / "SKILL.md")
        _verify_runtime_dir(staging)
        if dest.exists():
            dest.replace(backup)
        staging.replace(dest)
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        if backup.exists() and not dest.exists():
            backup.replace(dest)
        raise
    finally:
        shutil.rmtree(backup, ignore_errors=True)


def uninstall(name: str, home: Path) -> None:
    """Remove the runtime directory for this companion."""
    shutil.rmtree(installed_dir(name, home), ignore_errors=True)


def refresh(name: str, home: Path) -> None:
    """Refresh a companion by unconditionally re-copying the vendored SKILL.md."""
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
