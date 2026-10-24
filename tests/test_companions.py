"""Tests for embedded companion playbook helpers."""

from __future__ import annotations

import fnmatch
import inspect
import json
import re

import pytest

import yaml
from click.testing import CliRunner

from kluris.cli import cli
from kluris.core import companions
from kluris.core.config import BrainEntry, GlobalConfig, read_brain_config, write_global_config


def _fake_vendored(tmp_path, monkeypatch):
    root = tmp_path / "vendored"
    for name in companions.KNOWN:
        d = root / name
        (d / "references").mkdir(parents=True)
        (d / "assets").mkdir()
        (d / "SKILL.md").write_text(f"# {name}\n", encoding="utf-8")
        (d / "references" / "researcher.md").write_text("brief\n", encoding="utf-8")
        (d / "assets" / "spec-runtime.js").write_text("// js\n", encoding="utf-8")
    monkeypatch.setattr(companions, "_VENDORED", root)
    return root


def _tree(root):
    return sorted(p.relative_to(root).as_posix() for p in root.rglob("*"))


_FOLDER = [
    "SKILL.md",
    "assets",
    "assets/spec-runtime.js",
    "references",
    "references/researcher.md",
]


def test_install_copies_folder_to_home(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"

    companions.install("specmint-core", home)

    installed = home / ".kluris" / "companions" / "specmint-core"
    assert (installed / "SKILL.md").read_text(encoding="utf-8") == "# specmint-core\n"
    assert _tree(installed) == _FOLDER
    assert sorted(p.name for p in installed.parent.iterdir()) == ["specmint-core"]


def test_install_copies_only_allowlisted_entries(tmp_path, monkeypatch):
    root = _fake_vendored(tmp_path, monkeypatch)
    src = root / "specmint-core"
    (src / "README.md").write_text("readme", encoding="utf-8")
    (src / "evals").mkdir()
    (src / ".DS_Store").write_text("junk", encoding="utf-8")
    (src / "references" / ".DS_Store").write_text("junk", encoding="utf-8")
    home = tmp_path / "home"

    companions.install("specmint-core", home)

    assert _tree(home / ".kluris" / "companions" / "specmint-core") == _FOLDER


def test_install_without_optional_dirs(tmp_path, monkeypatch):
    root = _fake_vendored(tmp_path, monkeypatch)
    for sub in ("references", "assets"):
        for f in (root / "specmint-core" / sub).iterdir():
            f.unlink()
        (root / "specmint-core" / sub).rmdir()
    home = tmp_path / "home"

    companions.install("specmint-core", home)

    assert _tree(home / ".kluris" / "companions" / "specmint-core") == ["SKILL.md"]


@pytest.mark.parametrize("make", [
    lambda d: (d / "README.md").write_text("x", encoding="utf-8"),
    lambda d: (d / "evals").mkdir(),
    lambda d: (d / "references" / ".DS_Store").write_text("x", encoding="utf-8"),
])
def test_verify_runtime_dir_rejects_entries_outside_allowlist(tmp_path, make):
    d = tmp_path / "c"
    (d / "references").mkdir(parents=True)
    (d / "SKILL.md").write_text("x", encoding="utf-8")
    companions._verify_runtime_dir(d)
    make(d)

    with pytest.raises(OSError):
        companions._verify_runtime_dir(d)


def test_verify_runtime_dir_rejects_allowlisted_name_of_wrong_kind(tmp_path):
    d = tmp_path / "c"
    d.mkdir()
    (d / "SKILL.md").write_text("x", encoding="utf-8")
    (d / "assets").write_text("not a dir", encoding="utf-8")

    with pytest.raises(OSError, match="assets"):
        companions._verify_runtime_dir(d)


def test_install_keeps_existing_folder_when_verify_fails(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    companions.install("specmint-core", home)
    installed = home / ".kluris" / "companions" / "specmint-core"
    (installed / "references" / "researcher.md").write_text("old", encoding="utf-8")

    def boom(_path):
        raise OSError("verify failed")

    monkeypatch.setattr(companions, "_verify_runtime_dir", boom)
    with pytest.raises(OSError, match="verify failed"):
        companions.install("specmint-core", home)

    assert (installed / "references" / "researcher.md").read_text(encoding="utf-8") == "old"
    assert sorted(p.name for p in installed.parent.iterdir()) == ["specmint-core"]


@pytest.mark.parametrize("kind", ["file", "symlink", "dangling-symlink"])
def test_install_twice_over_stray_entry_at_companion_path(tmp_path, monkeypatch, kind):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    dest = home / ".kluris" / "companions" / "specmint-core"
    dest.parent.mkdir(parents=True)
    if kind == "file":
        dest.write_text("stray", encoding="utf-8")
    else:
        target = tmp_path / ("elsewhere" if kind == "symlink" else "missing")
        if kind == "symlink":
            target.mkdir()
        try:
            dest.symlink_to(target, target_is_directory=True)
        except OSError:
            pytest.skip("symlinks unavailable")

    companions.install("specmint-core", home)
    companions.install("specmint-core", home)

    assert not dest.is_symlink()
    assert _tree(dest) == _FOLDER
    assert sorted(p.name for p in dest.parent.iterdir()) == ["specmint-core"]
    if kind == "symlink":
        assert (tmp_path / "elsewhere").is_dir()


def test_install_leaves_no_leftover_after_stray_backup_file(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    parent = home / ".kluris" / "companions"
    parent.mkdir(parents=True)
    (parent / ".specmint-core.old").write_text("stray", encoding="utf-8")
    (parent / ".specmint-core.tmp").write_text("stray", encoding="utf-8")

    companions.install("specmint-core", home)

    assert sorted(p.name for p in parent.iterdir()) == ["specmint-core"]


@pytest.mark.parametrize("where", ["references/leak.md", "references", "SKILL.md"])
def test_install_rejects_symlink_in_vendored_tree(tmp_path, monkeypatch, where):
    root = _fake_vendored(tmp_path, monkeypatch)
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "leak.md").write_text("secret", encoding="utf-8")
    link = root / "specmint-core" / where
    if link.is_dir():
        for f in link.iterdir():
            f.unlink()
        link.rmdir()
    elif link.exists():
        link.unlink()
    try:
        if where == "references":
            link.symlink_to(outside, target_is_directory=True)
        else:
            link.symlink_to(outside / "leak.md")
    except OSError:
        pytest.skip("symlinks unavailable")
    home = tmp_path / "home"

    with pytest.raises(OSError, match="link"):
        companions.install("specmint-core", home)

    parent = home / ".kluris" / "companions"
    assert not parent.exists() or list(parent.iterdir()) == []


def test_refresh_replaces_old_single_file_install(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    old = home / ".kluris" / "companions" / "specmint-core"
    old.mkdir(parents=True)
    (old / "SKILL.md").write_text("old single-file playbook", encoding="utf-8")

    companions.refresh("specmint-core", home)

    assert _tree(old) == _FOLDER
    assert (old / "SKILL.md").read_text(encoding="utf-8") == "# specmint-core\n"
    assert sorted(p.name for p in old.parent.iterdir()) == ["specmint-core"]


def test_refresh_drops_files_no_longer_vendored(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    companions.install("specmint-core", home)
    installed = home / ".kluris" / "companions" / "specmint-core"
    (installed / "references" / "retired.md").write_text("gone", encoding="utf-8")

    companions.refresh("specmint-core", home)

    assert _tree(installed) == _FOLDER


def test_install_keeps_existing_dir_on_copy_failure(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    companions.install("specmint-core", home)
    skill = home / ".kluris" / "companions" / "specmint-core" / "SKILL.md"
    skill.write_text("old", encoding="utf-8")
    (tmp_path / "vendored" / "specmint-core" / "SKILL.md").unlink()

    try:
        companions.install("specmint-core", home)
    except FileNotFoundError:
        pass

    assert skill.read_text(encoding="utf-8") == "old"


def test_uninstall_removes_dir(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    companions.install("specmint-core", home)

    companions.uninstall("specmint-core", home)

    assert not (home / ".kluris" / "companions" / "specmint-core").exists()


def test_refresh_overwrites_user_modifications(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"
    companions.install("specmint-core", home)
    skill = home / ".kluris" / "companions" / "specmint-core" / "SKILL.md"
    skill.write_text("garbage", encoding="utf-8")

    companions.refresh("specmint-core", home)

    assert skill.read_text(encoding="utf-8") == "# specmint-core\n"


def test_refresh_idempotent_back_to_back(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"

    companions.refresh("specmint-core", home)
    companions.refresh("specmint-core", home)

    assert companions.is_installed("specmint-core", home)


def test_normalize_dedupes_and_orders_known_names():
    assert companions.normalize([
        "specmint-tdd-html",
        "specmint-tdd",
        "foo",
        "specmint-core-html",
        "specmint-core",
        "specmint-tdd",
    ]) == [
        "specmint-core",
        "specmint-tdd",
        "specmint-core-html",
        "specmint-tdd-html",
    ]


def test_referenced_reads_known_companions_from_brain_configs(tmp_path):
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "kluris.yml").write_text(
        yaml.dump({
            "name": "brain",
            "companions": ["specmint-tdd-html", "unknown", "specmint-core"],
        }),
        encoding="utf-8",
    )

    refs = companions.referenced(GlobalConfig(
        brains={"brain": BrainEntry(path=str(brain))}
    ))

    assert refs == ["specmint-core", "specmint-tdd-html"]


def test_companion_add_html_updates_brain_and_runtime(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("# Brain\n", encoding="utf-8")
    (brain / "kluris.yml").write_text(
        yaml.dump({"name": "brain", "description": "Brain", "companions": []}),
        encoding="utf-8",
    )
    write_global_config(GlobalConfig(
        brains={"brain": BrainEntry(path=str(brain), description="Brain")}
    ))

    result = CliRunner().invoke(cli, [
        "companion",
        "add",
        "specmint-core-html",
        "--brain",
        "brain",
        "--json",
    ])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload["name"] == "specmint-core-html"
    assert read_brain_config(brain).companions == ["specmint-core-html"]
    installed = tmp_path / ".kluris" / "companions" / "specmint-core-html"
    assert (installed / "SKILL.md").exists()
    assert (installed / "assets" / "spec-styles.css").is_file()
    assert (installed / "references" / "html-template.html").is_file()


def test_companion_list_and_remove_support_html_companions(tmp_path, monkeypatch):
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("# Brain\n", encoding="utf-8")
    (brain / "kluris.yml").write_text(
        yaml.dump({"name": "brain", "description": "Brain", "companions": []}),
        encoding="utf-8",
    )
    write_global_config(GlobalConfig(
        brains={"brain": BrainEntry(path=str(brain), description="Brain")}
    ))
    runner = CliRunner()

    add = runner.invoke(cli, [
        "companion", "add", "specmint-tdd-html", "--brain", "brain", "--json",
    ])
    assert add.exit_code == 0, add.output

    listed = runner.invoke(cli, ["companion", "list", "--json"])
    assert listed.exit_code == 0, listed.output
    payload = json.loads(listed.output)
    assert payload["known"] == list(companions.KNOWN)
    assert "specmint-tdd-html" in payload["installed"]
    assert payload["brains"] == [{
        "name": "brain",
        "path": str(brain),
        "companions": ["specmint-tdd-html"],
    }]

    removed = runner.invoke(cli, [
        "companion", "remove", "specmint-tdd-html", "--brain", "brain", "--json",
    ])
    assert removed.exit_code == 0, removed.output
    assert json.loads(removed.output)["name"] == "specmint-tdd-html"
    assert read_brain_config(brain).companions == []


_ALLOWED_TOP_LEVEL = {"SKILL.md", "references", "assets"}
# Same shape the sync script checks: `references/x.md`, `references/*.md`,
# but not `.specs/assets/...`.
_PATH_MENTION = re.compile(r"(?<![\w./-])((?:references|assets)/[A-Za-z0-9_.*-]+)")


def test_vendored_companion_folders_match_allowlist():
    assert companions.KNOWN == (
        "specmint-core",
        "specmint-tdd",
        "specmint-core-html",
        "specmint-tdd-html",
    )
    for name in companions.KNOWN:
        root = companions.vendored_dir(name)
        top = {p.name for p in root.iterdir()}
        assert "SKILL.md" in top, name
        assert top <= _ALLOWED_TOP_LEVEL, (name, top - _ALLOWED_TOP_LEVEL)
        assert (root / "references").is_dir(), name
        for p in root.rglob("*"):
            assert not p.name.startswith("."), p
            assert p.name != "__pycache__", p
        companions._verify_runtime_dir(root)
        if name.endswith("-html"):
            assert (root / "assets" / "spec-runtime.js").is_file()
            assert (root / "assets" / "spec-styles.css").is_file()


def test_vendored_companions_reference_only_shipped_files():
    for name in companions.KNOWN:
        root = companions.vendored_dir(name)
        shipped = [p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()]
        for md in [root / "SKILL.md", *sorted((root / "references").glob("*.md"))]:
            text = md.read_text(encoding="utf-8")
            for mention in _PATH_MENTION.findall(text):
                mention = mention.rstrip(".")
                if mention.endswith("/"):
                    continue
                if "*" in mention:
                    assert fnmatch.filter(shipped, mention), (name, md.name, mention)
                else:
                    assert (root / mention).is_file(), (name, md.name, mention)


def test_vendored_skill_md_has_no_plugin_install_text():
    forbidden = [
        "commands/",
        "agents/",
        ".claude-plugin",
        "plugin.json",
        "npx skills",
        "/plugin marketplace",
        "~/.claude/skills",
    ]
    for name in companions.KNOWN:
        content = (companions.vendored_dir(name) / "SKILL.md").read_text(encoding="utf-8")
        for pattern in forbidden:
            assert pattern not in content, (name, pattern)
        assert f"~/.kluris/companions/{name}/SKILL.md" in content
        assert "## Cross-Tool Compatibility" not in content
        assert "## Kluris Companion Compatibility" in content
        compat = content.split("## Kluris Companion Compatibility", 1)[1].split("\n## ", 1)[0]
        assert max(len(line) for line in compat.splitlines()) <= 75
        compat = " ".join(compat.split())
        assert "Paths in this file that start with `references/`" in compat
        assert "`.specs/...` paths" in compat
        if name.endswith("-html"):
            assert "SPEC.html" in content
            assert "data-status" in content
            assert f"~/.kluris/companions/{name}/assets/" in content


def test_companions_module_no_upstream_coupling():
    source = inspect.getsource(companions)
    assert "github.com" not in source
    assert "git clone" not in source
    assert "npx" not in source
    assert "/specmint/" not in source
