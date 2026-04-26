"""Boot-time read-only verification of the bundled brain.

Two layers protect the brain inside the packed Docker image:

1. The Dockerfile ``COPY``s ``brain/`` into ``/app/brain/`` and runs
   ``chmod -R a-w /app`` afterward — the runtime user cannot write
   anywhere under ``/app``.
2. This module's :func:`assert_brain_read_only` runs at app boot and
   refuses to start if (a) the brain is missing/invalid, or (b) the
   brain directory accepts a temp-file write. Either condition means
   the image was built incorrectly.

Outside the image (dev/tests) the function still validates the brain
exists and contains ``brain.md``, but skips the writability check —
test brains live in ``tmp_path`` and are intentionally writable. Use
``allow_writable=True`` to keep that convenience explicit.
"""

from __future__ import annotations

import os
from pathlib import Path

# Probe filename — ``.kluris-readonly-probe`` so it sorts visibly
# in case the chmod regression somehow leaves it on disk.
_PROBE_NAME = ".kluris-readonly-probe"


def assert_brain_read_only(brain_dir: Path, *, allow_writable: bool = False) -> None:
    """Verify ``brain_dir`` is a valid read-only kluris brain.

    Raises :class:`RuntimeError` if:
      - ``brain_dir`` does not exist or is not a directory, or
      - ``brain_dir / "brain.md"`` does not exist, or
      - ``allow_writable=False`` and a temp file CAN be created inside
        ``brain_dir`` (proves the image's ``chmod -R a-w`` did not run).

    The error message is short and actionable — it goes to stderr at
    boot, where Compose ``restart: unless-stopped`` keeps cycling until
    the deployer fixes the env or rebuilds the image.
    """
    if not brain_dir.exists() or not brain_dir.is_dir():
        raise RuntimeError(
            f"brain directory is not a valid kluris brain: {brain_dir} "
            "(directory missing)"
        )
    if not (brain_dir / "brain.md").exists():
        raise RuntimeError(
            f"brain directory is not a valid kluris brain: {brain_dir} "
            "(brain.md missing)"
        )

    if allow_writable:
        return

    probe = brain_dir / _PROBE_NAME
    try:
        # Open in exclusive-create mode. If the chmod ran, this raises
        # PermissionError. If the chmod did NOT run (regression), we
        # successfully create the file — we then DELETE it and raise
        # so the writable state never reaches production.
        fd = os.open(str(probe), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except PermissionError:
        # Expected production path: brain is read-only.
        return
    except OSError:
        # Anything else (read-only filesystem, EROFS, ENOSPC) is also
        # a non-writable signal.
        return
    else:
        try:
            os.close(fd)
        finally:
            try:
                probe.unlink()
            except OSError:
                pass
        raise RuntimeError(
            f"brain directory must be read-only inside the image: {brain_dir} "
            "(chmod -R a-w /app likely did not run during build)"
        )
