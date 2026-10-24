"""Tests for scripts/sync-specmint.py, the companion re-vendoring script."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "sync-specmint.py"


@pytest.fixture(scope="module")
def sync():
    spec = importlib.util.spec_from_file_location("sync_specmint", _SCRIPT)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # dataclasses look the module up by name
    try:
        spec.loader.exec_module(module)
        yield module
    finally:
        sys.modules.pop(spec.name, None)


_SKILL = """---
name: specmint-core
---

# Spec Mint Core

See `references/spec-format.md`.

## Cross-Tool Compatibility

- **Claude Code**: Skill via `npx skills add`

## Behavioral Notes

Be nice.
"""

_HTML_STEP = """4. Copy assets. AI tools
   should know the skill install location; for skill installs it
   is typically the installed skill's assets/ directory (e.g.
   `~/.claude/skills/specmint-core-html/assets/`). These
"""


def _source(tmp_path, skill, files=()):
    src = tmp_path / "src"
    src.mkdir()
    (src / "SKILL.md").write_text(skill, encoding="utf-8")
    for rel in files:
        (src / rel).parent.mkdir(parents=True, exist_ok=True)
        (src / rel).write_text("x", encoding="utf-8")
    return src


def test_sanitize_replaces_compat_section(sync):
    out = sync._sanitize_for_kluris("specmint-core", _SKILL)

    assert "npx skills" not in out
    assert "## Cross-Tool Compatibility" not in out
    assert "~/.kluris/companions/specmint-core/SKILL.md" in out
    assert out.endswith("## Behavioral Notes\n\nBe nice.\n")


def test_sanitize_fails_loudly_when_heading_is_renamed(sync):
    renamed = _SKILL.replace("## Cross-Tool Compatibility", "## Tool Compatibility")

    with pytest.raises(SystemExit, match="Cross-Tool Compatibility"):
        sync._sanitize_for_kluris("specmint-core", renamed)


def test_sanitize_rewrites_html_asset_location(sync):
    skill = _SKILL.replace("Be nice.", _HTML_STEP)

    out = sync._sanitize_for_kluris("specmint-core-html", skill)

    assert "~/.claude/skills" not in out
    assert "`~/.kluris/companions/specmint-core-html/assets/`. These" in out


def test_sanitize_fails_when_html_asset_sentence_is_missing(sync):
    with pytest.raises(SystemExit, match="asset location"):
        sync._sanitize_for_kluris("specmint-core-html", _SKILL)


def test_build_copies_allowlist_only(sync, tmp_path):
    src = _source(tmp_path, _SKILL, [
        "references/spec-format.md", "README.md", "evals/evals.json", ".idea/x.xml",
        "references/.DS_Store",
    ])

    tree = sync._build("specmint-core", src)

    assert sorted(tree) == ["SKILL.md", "references/spec-format.md"]


def test_build_fails_when_a_mentioned_file_is_not_shipped(sync, tmp_path):
    src = _source(tmp_path, _SKILL)

    with pytest.raises(SystemExit, match="references/spec-format.md"):
        sync._build("specmint-core", src)


def test_build_accepts_plugin_text_the_sanitizer_removes(sync, tmp_path):
    src = _source(tmp_path, _SKILL, ["references/spec-format.md"])

    tree = sync._build("specmint-core", src)

    assert b"npx skills" not in tree["SKILL.md"]


def test_build_fails_when_plugin_text_survives_sanitizing(sync, tmp_path):
    skill = _SKILL.replace("Be nice.", "Run `npx skills add ngvoicu/specmint-core`.")
    src = _source(tmp_path, skill, ["references/spec-format.md"])

    with pytest.raises(SystemExit, match="still mentions forbidden paths: npx skills"):
        sync._build("specmint-core", src)


def test_report_shows_per_file_changes(sync, capsys):
    changed = sync._report(
        "specmint-core",
        {"SKILL.md": b"a\n", "old.md": b"x\n"},
        {"SKILL.md": b"b\n", "references/new.md": b"y\n"},
        full_diff=False,
    )

    out = capsys.readouterr().out
    assert changed is True
    assert "changed  SKILL.md (+1 -1 lines" in out
    assert "added    references/new.md" in out
    assert "removed  old.md" in out
    assert sync._report("specmint-core", {"a": b"1"}, {"a": b"1"}, full_diff=False) is False


def test_main_writes_then_second_run_is_a_noop(sync, tmp_path, monkeypatch, capsys):
    html_skill = _SKILL.replace("Be nice.", _HTML_STEP)
    sources = {}
    for name in ("specmint-core", "specmint-tdd", "specmint-core-html", "specmint-tdd-html"):
        src = tmp_path / "sources" / name
        src.mkdir(parents=True)
        is_html = name.endswith("-html")
        skill = html_skill.replace("specmint-core-html", name) if is_html else _SKILL
        (src / "SKILL.md").write_text(skill, encoding="utf-8")
        (src / "references").mkdir()
        (src / "references" / "spec-format.md").write_text("format\n", encoding="utf-8")
        (src / "README.md").write_text("not shipped\n", encoding="utf-8")
        if is_html:
            (src / "assets").mkdir()
            (src / "assets" / "spec-runtime.js").write_text("// js\n", encoding="utf-8")
        sources[name] = src
    dest_root = tmp_path / "vendored"
    dest_root.mkdir()
    monkeypatch.setattr(sync, "DEST_ROOT", dest_root)
    argv = ["sync-specmint.py"]
    for name, flag in (
        ("specmint-core", "--core"),
        ("specmint-tdd", "--tdd"),
        ("specmint-core-html", "--core-html"),
        ("specmint-tdd-html", "--tdd-html"),
    ):
        argv += [flag, str(sources[name])]

    monkeypatch.setattr(sys, "argv", [*argv, "--check"])
    assert sync.main() == 1
    assert list(dest_root.iterdir()) == []

    monkeypatch.setattr(sys, "argv", argv)
    assert sync.main() == 0
    assert sorted(p.name for p in dest_root.iterdir()) == sorted(sources)
    html = dest_root / "specmint-tdd-html"
    assert sorted(p.relative_to(html).as_posix() for p in html.rglob("*") if p.is_file()) == [
        "SKILL.md", "assets/spec-runtime.js", "references/spec-format.md",
    ]
    assert "~/.kluris/companions/specmint-tdd-html/assets/" in (
        html / "SKILL.md"
    ).read_text(encoding="utf-8")
    capsys.readouterr()

    monkeypatch.setattr(sys, "argv", [*argv, "--check"])
    assert sync.main() == 0
    assert capsys.readouterr().out.count("in sync") == 4
