"""TEST-PACK-08 — boot-time read-only check on the bundled brain."""

from __future__ import annotations

import os
import stat

import pytest

from kluris.pack.readonly import assert_brain_read_only


def _make_brain(root) -> "Path":  # noqa: F821 (type hint only for clarity)
    brain = root / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("# Brain\n", encoding="utf-8")
    (brain / "knowledge").mkdir()
    return brain


def test_missing_brain_dir_raises(tmp_path):
    with pytest.raises(RuntimeError) as exc:
        assert_brain_read_only(tmp_path / "nope")
    assert "brain directory is not a valid kluris brain" in str(exc.value)


def test_missing_brain_md_raises(tmp_path):
    bad = tmp_path / "brain"
    bad.mkdir()
    with pytest.raises(RuntimeError) as exc:
        assert_brain_read_only(bad)
    assert "brain.md" in str(exc.value)


def test_writable_brain_dir_raises(tmp_path):
    brain = _make_brain(tmp_path)
    with pytest.raises(RuntimeError) as exc:
        assert_brain_read_only(brain)
    assert "read-only" in str(exc.value).lower()


def test_writable_brain_dir_passes_when_allowed(tmp_path):
    brain = _make_brain(tmp_path)
    # No exception when the test explicitly opts out of the writability
    # check (matches the dev/test path).
    assert_brain_read_only(brain, allow_writable=True)


@pytest.mark.skipif(
    os.name == "nt",
    reason="POSIX permission semantics — chmod read-only doesn't model "
           "Docker's chmod -R a-w on Windows the same way.",
)
def test_actual_readonly_chmod_passes(tmp_path):
    """Mimic the Docker image: ``chmod -R a-w /app/brain`` blocks
    writes from the same user. The probe must not raise here.
    """
    brain = _make_brain(tmp_path)
    # Strip write bits for owner/group/other on the brain dir AND the
    # brain.md file so a temp-file create truly fails.
    for path in [brain, brain / "brain.md", brain / "knowledge"]:
        os.chmod(path, stat.S_IRUSR | stat.S_IRGRP | stat.S_IROTH | (
            stat.S_IXUSR if path.is_dir() else 0
        ))
    try:
        assert_brain_read_only(brain)
    finally:
        # Restore so pytest's tmp_path cleanup can remove the tree.
        for path in [brain, brain / "brain.md", brain / "knowledge"]:
            os.chmod(path, 0o755 if path.is_dir() else 0o644)


def test_probe_file_cleaned_up_on_writable_brain(tmp_path):
    brain = _make_brain(tmp_path)
    with pytest.raises(RuntimeError):
        assert_brain_read_only(brain)
    leftover = list(brain.glob(".kluris-readonly-probe*"))
    assert leftover == [], (
        f"readonly probe must clean up its own file: {leftover}"
    )
