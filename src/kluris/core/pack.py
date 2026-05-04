"""Stager for ``kluris pack``.

Produces a self-contained directory:

```
<output>/
├── brain/                # brain content COPY'd into the image
├── app/                  # chat server source (from src/kluris/pack/)
├── kluris_runtime/       # minimal read-only runtime
├── Dockerfile
├── docker-compose.yml
├── .dockerignore
├── .gitignore
├── .env.example
└── README.md
```

Only files that go into the build context are copied — no
``__pycache__/``, no ``*.pyc/.pyo``. After copying, brain file mtimes
are stamped from each file's ``git log -1 --format=%aI`` author date
so the in-image ``recent`` tool's mtime fallback is meaningful.
"""

from __future__ import annotations

from importlib import resources
import os
import shutil
from datetime import datetime
from pathlib import Path
from typing import Iterable

import pathspec


_SOURCE_PACKAGING_ROOT = Path(__file__).resolve().parent.parent.parent.parent / "packaging"
_TEMPLATE_PACKAGE = "kluris._packaging"
_PACK_SRC = Path(__file__).resolve().parent.parent / "pack"
_RUNTIME_SRC = (
    Path(__file__).resolve().parent.parent.parent / "kluris_runtime"
)

# Brain entries the stager always excludes.
_BRAIN_EXCLUDES_DEFAULT = (
    ".git",
    "__pycache__",
    "*.pyc",
    "*.pyo",
    "kluris.yml",
    "brain-mri.html",
    ".DS_Store",
)

# Files inside an existing pack output that ``--force`` rebuilds must
# preserve. ``.env`` is created by the deployer (``cp .env.example
# .env``) and carries LLM credentials they typed in by hand; wiping it
# on every rebuild would force them to refill creds for every brain-
# edit cycle. ``.env.example`` is template-only and IS regenerated.
_PRESERVE_ON_FORCE = (
    ".env",
    ".env.local",
    ".env.production",
    ".env.staging",
)


def stage_pack(
    brain_path: Path,
    output_dir: Path,
    *,
    brain_name: str,
    excludes: Iterable[str] = (),
    force: bool = False,
) -> dict:
    """Build the pack output directory at ``output_dir``.

    Returns a manifest dict ``{ok, output, brain, neuron_count,
    files: [...], preserved: [...]}`` so the CLI can emit JSON and
    the test suite can assert exact file lists.

    When ``force=True`` and ``output_dir`` already exists, the
    directory is wiped and rebuilt — but any ``.env`` / ``.env.local``
    / ``.env.production`` / ``.env.staging`` files inside are
    preserved across the rebuild. The deployer's typed-in credentials
    survive a brain-edit / Dockerfile-edit / kluris-upgrade cycle.
    """
    output_dir = Path(output_dir).resolve()

    preserved_files: dict[str, bytes] = {}
    if output_dir.exists():
        if not force:
            raise FileExistsError(
                f"output directory already exists: {output_dir} "
                "(rerun with `--force` to wipe + rebuild while "
                "preserving .env)"
            )
        # Snapshot files we want to carry across the rebuild.
        for name in _PRESERVE_ON_FORCE:
            candidate = output_dir / name
            if candidate.is_file():
                try:
                    preserved_files[name] = candidate.read_bytes()
                except OSError:
                    continue
        shutil.rmtree(output_dir)

    output_dir.mkdir(parents=True)

    _copy_pack_source(_PACK_SRC, output_dir / "app")
    _copy_runtime(_RUNTIME_SRC, output_dir / "kluris_runtime")
    neuron_count = _copy_brain(
        brain_path,
        output_dir / "brain",
        excludes=tuple(_BRAIN_EXCLUDES_DEFAULT) + tuple(excludes),
    )
    _stamp_brain_mtimes(brain_path, output_dir / "brain")

    _render_template(
        "Dockerfile.template", output_dir / "Dockerfile", brain_name=brain_name,
    )
    _render_template(
        "docker-compose.yml.template",
        output_dir / "docker-compose.yml",
        brain_name=brain_name,
    )
    _render_template(
        "dockerignore.template", output_dir / ".dockerignore", brain_name=brain_name,
    )
    _render_template(
        "gitignore.template", output_dir / ".gitignore", brain_name=brain_name,
    )
    _render_template(
        "env.example.template", output_dir / ".env.example", brain_name=brain_name,
    )
    _render_template(
        "README.template.md", output_dir / "README.md", brain_name=brain_name,
    )

    # Restore preserved files AFTER staging so they overwrite the
    # freshly-templated copies (e.g., the regenerated commented-out
    # `.env` is replaced by the deployer's filled-in `.env`).
    for name, contents in preserved_files.items():
        (output_dir / name).write_bytes(contents)

    files = sorted(
        str(p.relative_to(output_dir)).replace("\\", "/")
        for p in output_dir.rglob("*")
        if p.is_file()
    )
    return {
        "ok": True,
        "output": str(output_dir),
        "brain": brain_name,
        "neuron_count": neuron_count,
        "files": files,
        "preserved": sorted(preserved_files.keys()),
    }


def _copy_pack_source(src: Path, dest: Path) -> None:
    shutil.copytree(
        src,
        dest,
        ignore=shutil.ignore_patterns(
            "__pycache__", "*.pyc", "*.pyo", "*.swp",
        ),
    )


def _copy_runtime(src: Path, dest: Path) -> None:
    shutil.copytree(
        src,
        dest,
        ignore=shutil.ignore_patterns(
            "__pycache__", "*.pyc", "*.pyo", "*.swp",
        ),
    )


def _copy_brain(
    brain_path: Path,
    dest: Path,
    *,
    excludes: Iterable[str],
) -> int:
    """Copy ``brain_path`` into ``dest`` honoring gitignore-style globs.

    Returns the count of files actually copied.
    """
    spec = pathspec.GitIgnoreSpec.from_lines(excludes)
    dest.mkdir(parents=True)
    file_count = 0
    for src in brain_path.rglob("*"):
        rel = src.relative_to(brain_path)
        rel_posix = str(rel).replace("\\", "/")
        if src.is_dir():
            if spec.match_file(rel_posix + "/") or spec.match_file(rel_posix):
                continue
            (dest / rel).mkdir(exist_ok=True)
            continue
        if spec.match_file(rel_posix):
            continue
        target = dest / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, target)
        file_count += 1
    return file_count


def _stamp_brain_mtimes(brain_src: Path, brain_dest: Path) -> None:
    """Stamp brain file mtimes from ``git log -1 --format=%aI`` per file.

    Untracked files keep their source mtime. Brains that are not git
    repos are skipped silently.
    """
    try:
        from kluris.core.git import git_log_file_dates, is_git_repo
    except Exception:
        return
    if not is_git_repo(brain_src):
        return
    try:
        latest_by_path, _created = git_log_file_dates(brain_src)
    except Exception:
        return
    for rel, iso in latest_by_path.items():
        target = brain_dest / rel
        if not target.exists() or not iso:
            continue
        try:
            stamp = datetime.fromisoformat(iso).timestamp()
        except ValueError:
            continue
        try:
            os.utime(target, (stamp, stamp))
        except OSError:
            continue


def _read_template(name: str) -> str:
    """Read a pack template from source checkout or installed package data."""
    source_path = _SOURCE_PACKAGING_ROOT / name
    if source_path.exists():
        return source_path.read_text(encoding="utf-8")
    return (
        resources.files(_TEMPLATE_PACKAGE)
        .joinpath(name)
        .read_text(encoding="utf-8")
    )


def _render_template(name: str, output_path: Path, *, brain_name: str) -> None:
    text = _read_template(name)
    text = text.replace("{brain_name}", brain_name)
    output_path.write_text(text, encoding="utf-8")
