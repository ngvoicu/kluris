"""Tests for embedded companion playbook helpers."""

from __future__ import annotations

import inspect
import json

import yaml
from click.testing import CliRunner

from kluris.cli import cli
from kluris.core import companions
from kluris.core.config import BrainEntry, GlobalConfig, read_brain_config, write_global_config


def _fake_vendored(tmp_path, monkeypatch):
    root = tmp_path / "vendored"
    for name in companions.KNOWN:
        d = root / name
        d.mkdir(parents=True)
        (d / "SKILL.md").write_text(f"# {name}\n", encoding="utf-8")
    monkeypatch.setattr(companions, "_VENDORED", root)
    return root


def test_install_copies_files_to_home(tmp_path, monkeypatch):
    _fake_vendored(tmp_path, monkeypatch)
    home = tmp_path / "home"

    companions.install("specmint-core", home)

    installed = home / ".kluris" / "companions" / "specmint-core"
    assert (installed / "SKILL.md").read_text(encoding="utf-8") == "# specmint-core\n"
    assert [p.name for p in installed.iterdir()] == ["SKILL.md"]


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
    assert (tmp_path / ".kluris" / "companions" / "specmint-core-html" / "SKILL.md").exists()


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


def test_vendored_companions_are_single_file_self_contained():
    assert companions.KNOWN == (
        "specmint-core",
        "specmint-tdd",
        "specmint-core-html",
        "specmint-tdd-html",
    )
    forbidden = [
        "commands/",
        "references/",
        "agents/",
        ".claude-plugin",
        "plugin.json",
        "npx skills",
        "/plugin marketplace",
    ]
    for name in companions.KNOWN:
        root = companions.vendored_dir(name)
        assert [p.name for p in root.iterdir()] == ["SKILL.md"]
        content = (root / "SKILL.md").read_text(encoding="utf-8")
        for pattern in forbidden:
            assert pattern not in content
        if name.endswith("-html"):
            assert "SPEC.html" in content
            assert "data-status" in content
            assert "SPEC.md" not in content


def test_companions_module_no_upstream_coupling():
    source = inspect.getsource(companions)
    assert "github.com" not in source
    assert "git clone" not in source
    assert "npx" not in source
    assert "/specmint/" not in source
