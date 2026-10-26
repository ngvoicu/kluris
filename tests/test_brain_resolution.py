"""Tests for brain resolution logic."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def test_explicit_brain_flag(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status", "--brain", "brain-b", "--json"])
    data = json.loads(result.stdout)
    assert data["brains"][0]["name"] == "brain-b"


def test_single_brain_auto(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "only-brain", tmp_path)
    result = runner.invoke(cli, ["status", "--json"])
    data = json.loads(result.stdout)
    assert len(data["brains"]) == 1


def test_multi_brain_wake_up_picker_required(tmp_path, monkeypatch):
    """With 2+ brains and no --brain (no TTY under CliRunner), wake-up must error."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["wake-up"])
    assert result.exit_code != 0
    assert "Multiple brains" in result.output


def test_multi_brain_picker_with_tty_input(tmp_path, monkeypatch):
    """When stdin is a TTY, the picker prompts and accepts an integer choice."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status"], input="1\n")
    assert result.exit_code == 0
    assert "[1] brain-a" in result.output
    assert "[2] brain-b" in result.output


def test_multi_brain_all_option_for_fan_out(tmp_path, monkeypatch):
    """Fan-out commands offer [N+1] all and processing all picks every brain."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status"], input="3\n")
    assert result.exit_code == 0
    assert "[3] all" in result.output
    assert "brain-a" in result.output
    assert "brain-b" in result.output


def test_multi_brain_picker_not_offered_for_wake_up(tmp_path, monkeypatch):
    """Single-brain commands like wake-up must NOT show [3] all in the picker."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["wake-up"], input="1\n")
    # Verify [3] all is absent from the picker output
    assert "[1] brain-a" in result.output
    assert "[2] brain-b" in result.output
    assert "[3] all" not in result.output


def test_multi_brain_json_no_brain_errors(tmp_path, monkeypatch):
    """`--json` with multiple brains and no --brain must error cleanly even with a TTY."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.stdout)
    assert data["ok"] is False
    assert "Multiple brains" in data["error"]


def test_unknown_brain_flag_typo_multi_brain(tmp_path, monkeypatch):
    """--brain typo with multiple brains errors with a clear message."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status", "--brain", "typo"])
    assert result.exit_code != 0
    assert "typo" in result.output


def test_unknown_brain_flag_typo_single_brain(tmp_path, monkeypatch):
    """--brain typo with one brain registered also errors clearly."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "only-brain", tmp_path)
    result = runner.invoke(cli, ["dream", "--brain", "typo"])
    assert result.exit_code != 0
    assert "typo" in result.output


def test_brain_all_on_fan_out(tmp_path, monkeypatch):
    """--brain all on dream/push/status/mri processes every brain without prompting."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status", "--brain", "all", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.stdout)
    assert data["ok"] is True
    names = [b["name"] for b in data["brains"]]
    assert "brain-a" in names
    assert "brain-b" in names


def test_brain_all_on_wake_up_rejected(tmp_path, monkeypatch):
    """--brain all is only valid on fan-out commands; wake-up rejects it."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["wake-up", "--brain", "all"])
    assert result.exit_code != 0
    assert "all" in result.output.lower()


def test_brain_all_with_zero_brains_errors(tmp_path, monkeypatch):
    """--brain all with no brains registered errors instead of silently doing nothing."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["dream", "--brain", "all"])
    assert result.exit_code != 0
    assert "No brains" in result.output


def test_picker_invalid_input_reprompts(tmp_path, monkeypatch):
    """Click IntRange re-prompts on out-of-range input."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    # First input "99" is out of range; Click re-prompts; "2" succeeds.
    result = runner.invoke(cli, ["status"], input="99\n2\n")
    assert result.exit_code == 0
    # Some indication of re-prompt or invalid value should appear
    assert "Choice" in result.output or "invalid" in result.output.lower()


def test_kluris_no_prompt_env_disables_picker(tmp_path, monkeypatch):
    """KLURIS_NO_PROMPT=1 forces non-interactive mode even when stdin is a TTY."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("KLURIS_NO_PROMPT", "1")
    import kluris.cli as cli_module
    monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status"])
    assert result.exit_code != 0
    assert "Multiple brains" in result.output


def test_resolver_rejects_brain_with_missing_path(tmp_path, monkeypatch):
    """A registered brain with a missing path raises a clean error from the resolver."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "ghost", tmp_path)
    # Now move the brain dir away so the registered path is stale
    (tmp_path / "ghost").rename(tmp_path / "ghost-moved")
    result = runner.invoke(cli, ["status", "--brain", "ghost"])
    assert result.exit_code != 0
    assert "missing" in result.output.lower() or "invalid" in result.output.lower()
    assert "ghost" in result.output


def test_resolver_rejects_brain_all_when_one_path_missing(tmp_path, monkeypatch):
    """--brain all also runs the missing-path check across every brain."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    (tmp_path / "brain-b").rename(tmp_path / "brain-b-moved")
    result = runner.invoke(cli, ["dream", "--brain", "all"])
    assert result.exit_code != 0
    assert "brain-b" in result.output


def test_validate_brain_name_rejects_all_at_create(tmp_path, monkeypatch):
    """`kluris create all` is rejected because `all` collides with --brain all."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(
        cli, ["create", "all", "--path", str(tmp_path), "--description", "no", "--json"]
    )
    assert result.exit_code != 0
    assert "reserved" in result.output.lower() or "all" in result.output
