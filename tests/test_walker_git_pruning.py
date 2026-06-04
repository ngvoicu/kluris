"""Regression: brain walkers must never descend into ``.git/``.

The walkers used to ``rglob`` the whole brain and filter ``.git`` out of the
*results*. But ``rglob`` still ``scandir``s every directory it crosses —
including ``.git/objects/*`` — which races with git's background gc deleting
loose-object fan-out dirs mid-walk (``FileNotFoundError``), and chokes on the
deep ``*-pack`` nest a crashed ``kluris pack`` could leave inside a brain
(``File name too long``). The fix walks with ``os.walk`` and prunes skip dirs
*in place*, so ``.git`` is never entered at all.

These tests spy on ``os.scandir`` (which ``os.walk`` uses) and fail if any
walker scandirs a ``.git`` path — a deterministic, cross-platform guard that a
revert to ``rglob`` would trip.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest

import kluris.cli as cli_module
import kluris.core.mri as core_mri
import kluris_runtime.neuron_index as rt_neuron_index
import kluris_runtime.wake_up as rt_wake_up


@pytest.fixture
def git_brain(tmp_path) -> Path:
    """A brain with a real neuron plus a ``.git`` tree holding a stray ``.md``."""
    brain = tmp_path / "b"
    (brain / "knowledge").mkdir(parents=True)
    (brain / "brain.md").write_text("---\n---\n# B\n", encoding="utf-8")
    (brain / "knowledge" / "map.md").write_text("---\n---\n# K\n", encoding="utf-8")
    (brain / "knowledge" / "n.md").write_text("---\n---\n# n\n", encoding="utf-8")
    # A markdown file buried in .git must never be indexed — and never walked.
    git_objects = brain / ".git" / "objects" / "ab"
    git_objects.mkdir(parents=True)
    (git_objects / "loose.md").write_text("# loose\n", encoding="utf-8")
    return brain


@pytest.fixture
def no_git_descent(monkeypatch):
    """Make ``os.scandir`` raise if anything tries to walk into ``.git``."""
    real_scandir = os.scandir

    def guard(path=".", *args, **kwargs):
        parts = os.fspath(path).replace("\\", "/").split("/")
        if ".git" in parts:
            raise AssertionError(f"walker descended into .git: {path!r}")
        return real_scandir(path, *args, **kwargs)

    monkeypatch.setattr(os, "scandir", guard)
    return guard


def test_all_neuron_files_skips_git(git_brain, no_git_descent):
    names = {p.name for p in rt_neuron_index.all_neuron_files(git_brain)}
    assert "n.md" in names
    assert "loose.md" not in names  # the .git-buried file is not indexed


def test_wake_up_iter_neurons_skips_git(git_brain, no_git_descent):
    names = {p.name for p in rt_wake_up._iter_neurons(git_brain)}
    assert "n.md" in names
    assert "loose.md" not in names


def test_mri_all_neuron_files_skips_git(git_brain, no_git_descent):
    names = {p.name for p in core_mri._all_neuron_files(git_brain)}
    assert "n.md" in names
    assert "loose.md" not in names


def test_brain_directories_skips_git(git_brain, no_git_descent):
    dirs = cli_module._brain_directories(git_brain)
    rels = {d.relative_to(git_brain).as_posix() for d in dirs}
    assert "knowledge" in rels
    assert not any(".git" in r.split("/") for r in rels)
