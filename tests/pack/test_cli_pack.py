"""TEST-PACK-57 — ``kluris pack`` CLI command."""

from __future__ import annotations

import json

import pytest
from click.testing import CliRunner

from kluris.cli import cli


@pytest.fixture
def cli_runner_local():
    return CliRunner()


def test_pack_single_brain_default_output(temp_brain, cli_runner_local, tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    result = cli_runner_local.invoke(cli, ["pack", "--json"])
    assert result.exit_code == 0, result.stdout + result.stderr
    data = json.loads(result.stdout)
    assert data["ok"] is True
    assert data["brain"] == "test-brain"
    assert (tmp_path / "test-brain-pack").is_dir()


def test_pack_custom_output(temp_brain, cli_runner_local, tmp_path):
    out = tmp_path / "build" / "custom"
    result = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--json"]
    )
    assert result.exit_code == 0, result.stdout + result.stderr
    data = json.loads(result.stdout)
    assert data["output"] == str(out.resolve())
    assert out.is_dir()


def test_pack_excludes_honored(temp_brain, cli_runner_local, tmp_path, monkeypatch):
    (temp_brain / "knowledge" / "private.md").write_text(
        "---\nupdated: 2026-04-01\n---\n# private\n", encoding="utf-8"
    )
    monkeypatch.chdir(tmp_path)
    result = cli_runner_local.invoke(
        cli,
        ["pack", "--exclude", "knowledge/private.md", "--json"],
    )
    assert result.exit_code == 0
    data = json.loads(result.stdout)
    assert "brain/knowledge/private.md" not in data["files"]


def test_pack_existing_output_errors(temp_brain, cli_runner_local, tmp_path):
    out = tmp_path / "exists"
    out.mkdir()
    result = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--json"]
    )
    assert result.exit_code != 0
    err = json.loads(result.stdout)
    assert err["ok"] is False
    assert "exists" in err["error"].lower()
    # Hint at the escape hatch.
    assert "--force" in err["error"]


def test_pack_force_rebuilds_existing_output(
    temp_brain, cli_runner_local, tmp_path,
):
    out = tmp_path / "out"
    # Initial pack succeeds.
    first = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--json"],
    )
    assert first.exit_code == 0, first.stdout + first.stderr
    # Deployer fills in .env with real creds.
    real_env = "KLURIS_API_KEY=sk-do-not-lose\n"
    (out / ".env").write_text(real_env, encoding="utf-8")

    # --force rebuild succeeds and preserves the .env.
    second = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--force", "--json"],
    )
    assert second.exit_code == 0, second.stdout + second.stderr
    data = json.loads(second.stdout)
    assert data["ok"] is True
    assert ".env" in data["preserved"]
    assert (out / ".env").read_text(encoding="utf-8") == real_env


def test_pack_help_documents_flags(cli_runner_local):
    result = cli_runner_local.invoke(cli, ["pack", "--help"])
    assert result.exit_code == 0
    for flag in ("--brain", "--output", "--exclude", "--json"):
        assert flag in result.stdout


def test_pack_from_inside_brain_uses_safe_default_json(
    temp_brain, cli_runner_local, tmp_path, monkeypatch,
):
    """Run from the brain root with --json: the pack lands next to the brain,
    never inside it (the recursion footgun)."""
    monkeypatch.chdir(temp_brain)
    result = cli_runner_local.invoke(cli, ["pack", "--json"])
    assert result.exit_code == 0, result.stdout + result.stderr
    data = json.loads(result.stdout)
    assert data["ok"] is True
    # Lands at <brain-parent>/<name>-pack, not inside the brain.
    assert data["output"] == str((tmp_path / "test-brain-pack").resolve())
    assert (tmp_path / "test-brain-pack").is_dir()
    assert not (temp_brain / "test-brain-pack").exists()


def test_pack_explicit_output_inside_brain_errors(
    temp_brain, cli_runner_local,
):
    """An explicit --output pointing inside the brain is refused."""
    inside = temp_brain / "build" / "pack"
    result = cli_runner_local.invoke(
        cli, ["pack", "--output", str(inside), "--json"],
    )
    assert result.exit_code != 0
    err = json.loads(result.stdout)
    assert err["ok"] is False
    assert "inside a brain" in err["error"].lower()
    assert not inside.exists()


def test_pack_from_inside_brain_prompts_for_path(
    temp_brain, cli_runner_local, tmp_path, monkeypatch,
):
    """On a TTY, running from inside the brain prompts for an output path."""
    import kluris.cli as cli_module

    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    monkeypatch.chdir(temp_brain)
    chosen = tmp_path / "chosen-pack"
    result = cli_runner_local.invoke(
        cli, ["pack"], input=f"{chosen}\n",
    )
    assert result.exit_code == 0, result.stdout + result.stderr
    assert chosen.is_dir()
    assert not (temp_brain / "test-brain-pack").exists()


def test_resolve_pack_output_never_lands_in_another_brain(tmp_path, monkeypatch):
    """Standing inside brain A while packing brain B must not drop the pack
    inside A — the invariant holds against *every* registered brain."""
    from kluris.cli import _resolve_pack_output, _inside_any_brain

    brain_a = tmp_path / "brain-a"
    brain_b = tmp_path / "brain-b"
    brain_a.mkdir()
    brain_b.mkdir()
    roots = [brain_a, brain_b]

    # cwd is inside brain A; we're packing brain B. --json → safe default.
    monkeypatch.chdir(brain_a)
    out = _resolve_pack_output(
        None, brain_path=brain_b, brain_roots=roots,
        brain_name="brain-b", as_json=True,
    )
    assert _inside_any_brain(out, roots) is None
    assert out == (brain_b.parent / "brain-b-pack").resolve()


def test_resolve_pack_output_explicit_inside_other_brain_errors(
    tmp_path, monkeypatch,
):
    """An explicit --output inside a *different* registered brain is refused."""
    import click
    from kluris.cli import _resolve_pack_output

    brain_a = tmp_path / "brain-a"
    brain_b = tmp_path / "brain-b"
    brain_a.mkdir()
    brain_b.mkdir()

    with pytest.raises(click.ClickException) as exc:
        _resolve_pack_output(
            str(brain_a / "sub"), brain_path=brain_b,
            brain_roots=[brain_a, brain_b], brain_name="brain-b", as_json=False,
        )
    assert "inside a brain" in str(exc.value).lower()
