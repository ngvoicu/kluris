"""Regenerate ``crates/kluris-core/tests/fixtures/skills/`` with the v2.28.11 oracle.

Run from the repo root (the oracle venv must exist, e.g. after one parity run):

    parity/.oracle/venv/bin/python -I parity/tools/gen_skill_fixtures.py

Writes, for every case in ``CASES``:

- ``v2/<case>.md``: v2's ``kluris.core.agents.render_skill`` for those inputs,
  byte for byte (the SKILL.md v2 installs into every agent folder);
- ``v2/<case>.workflow.md``: v2's Windsurf workflow for the same inputs (the
  v3 migration recognises and removes these);

plus ``cases.json`` (the inputs, so the Rust tests render the same brains) and
``posix_paths.json`` (``agents._posix_path`` for awkward paths, the rule v2
uses for the brain and companion paths it bakes into SKILL.md).
"""

from __future__ import annotations

import json
from pathlib import Path

from kluris.core.agents import _posix_path, _render_workflow, render_skill

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "crates" / "kluris-core" / "tests" / "fixtures" / "skills"

ALL_COMPANIONS = ["specmint-core", "specmint-tdd", "specmint-core-html", "specmint-tdd-html"]

CASES = {
    "basic": dict(
        brain_name="acme",
        brain_path="/home/dev/brains/acme",
        has_git=True,
        brain_description="Acme platform knowledge",
        companions=[],
        companion_home="/home/dev/.kluris/companions",
    ),
    "no-git-default-description": dict(
        brain_name="notes",
        brain_path="/home/dev/notes",
        has_git=False,
        brain_description="",
        companions=[],
        companion_home="/home/dev/.kluris/companions",
    ),
    "quotes-and-unicode": dict(
        brain_name="team-data-2",
        brain_path="/srv/team brains/data",
        has_git=True,
        brain_description='The "data" team\'s brain \\ Équipe données — 日本語 ✓',
        companions=[],
        companion_home="/srv/home/.kluris/companions",
    ),
    "windows-path": dict(
        brain_name="winbrain",
        brain_path="C:\\Users\\Dev\\brains\\winbrain",
        has_git=True,
        brain_description="Windows brain",
        companions=["specmint-core"],
        companion_home="C:/Users/Dev/.kluris/companions",
    ),
    "companion-core-only": dict(
        brain_name="core",
        brain_path="/home/dev/brains/core",
        has_git=True,
        brain_description="Core companion only",
        companions=["specmint-core"],
        companion_home="/home/dev/.kluris/companions",
    ),
    "companions-all": dict(
        brain_name="all-four",
        brain_path="/home/dev/brains/all-four",
        has_git=True,
        brain_description="Every companion",
        companions=ALL_COMPANIONS,
        companion_home="/home/dev/.kluris/companions",
    ),
    "companions-html-pair": dict(
        brain_name="html",
        brain_path="/home/dev/brains/html",
        has_git=False,
        brain_description="HTML companions, given out of order",
        companions=["specmint-tdd-html", "specmint-core-html"],
        companion_home="/home/dev/.kluris/companions",
    ),
    "companion-tdd-only": dict(
        brain_name="tdd",
        brain_path="/home/dev/brains/tdd",
        has_git=True,
        brain_description="TDD companion only",
        companions=["specmint-tdd"],
        companion_home="/home/dev/.kluris/companions",
    ),
    "messy-posix-path": dict(
        brain_name="messy",
        brain_path="/srv//brains/./messy/",
        has_git=True,
        brain_description="A path v2 normalizes",
        companions=[],
        companion_home="/srv/home/.kluris/companions",
    ),
}

POSIX_PATHS = [
    "/home/dev/brains/acme",
    "/srv//brains/./messy/",
    "//srv/share/x",
    "///three/leading",
    "relative/path",
    "./relative/./x/",
    "a/../b",
    "",
    ".",
    "/",
    "/trailing/",
    "C:\\Users\\Dev\\brains\\acme",
    "C:\\Users\\\\Dev\\brains\\",
    "C:/Users/Dev/brains",
    "c:\\lower\\case",
    "C:",
    "C:relative\\x",
    "D:\\",
    "\\\\server\\share\\brains\\acme",
    "\\\\server\\share",
    "\\rooted\\no\\drive",
    "mixed/sep\\here",
    "C:\\a\\.\\b\\..\\c",
    "/with space/ünïcode/日本",
    "x:",
    "/a:b/c",
    "\\\\?\\C:\\x\\y",
    "\\\\?\\UNC\\server\\share\\x",
    "\\\\server",
    "\\\\\\triple",
    "\\\\server\\\\share",
    "1:\\not\\drive",
    "//two/slashes/",
]


def main() -> None:
    v2 = OUT / "v2"
    v2.mkdir(parents=True, exist_ok=True)
    for case, inputs in CASES.items():
        kwargs = dict(inputs, skill_name=f"kluris-{inputs['brain_name']}")
        (v2 / f"{case}.md").write_bytes(render_skill(**kwargs).encode("utf-8"))
        (v2 / f"{case}.workflow.md").write_bytes(_render_workflow(**kwargs).encode("utf-8"))
    (OUT / "cases.json").write_text(json.dumps(CASES, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    posix = [[p, _posix_path(p)] for p in POSIX_PATHS]
    (OUT / "posix_paths.json").write_text(json.dumps(posix, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
