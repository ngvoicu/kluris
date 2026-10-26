"""Sunset notice (v2.29.0): every kluris path prints one Kluris 3 pointer
to stderr, exactly once, without touching stdout."""

import json
import subprocess

import pytest
from click.testing import CliRunner

from kluris.cli import cli

SUNSET_NOTICE = (
    "kluris: Kluris 3 is a native app — uninstall this version "
    "(pipx uninstall kluris / uv tool uninstall kluris) and install from "
    "https://kluris.ngvoicu.dev"
)

ALL_COMMANDS = [
    "create",
    "register",
    "list",
    "status",
    "search",
    "wake-up",
    "companion",
    "dream",
    "pack",
    "mri",
    "remove",
    "doctor",
    "help",
]

JSON_COMMANDS = [
    ["list", "--json"],
    ["status", "--json"],
    ["wake-up", "--json"],
    ["search", "test", "--json"],
    ["dream", "--json"],
]


def _invoke(args, env=None):
    return CliRunner().invoke(cli, args, env=env)


def _restore_brain(brain_path):
    """Reset a git-backed test brain to HEAD so a mutating command can be
    re-run against identical state."""
    subprocess.run(
        ["git", "checkout", "--", "."],
        cwd=brain_path, check=True, capture_output=True,
    )
    subprocess.run(
        ["git", "clean", "-fd"],
        cwd=brain_path, check=True, capture_output=True,
    )


@pytest.mark.parametrize("command", ALL_COMMANDS)
def test_every_command_prints_one_notice_line(command, temp_config):
    result = _invoke([command, "--help"])
    assert result.stderr == SUNSET_NOTICE + "\n"


@pytest.mark.parametrize(
    "args",
    [
        ["--help"],
        ["--version"],
        ["help"],
        ["help", "list"],
        ["list"],
    ],
)
def test_meta_paths_print_one_notice_line(args, temp_config):
    result = _invoke(args)
    assert result.stderr == SUNSET_NOTICE + "\n"


def test_unknown_command_prints_one_notice_line(temp_config):
    result = _invoke(["not-a-command"])
    assert result.exit_code != 0
    # Usage-error text joins the notice on stderr; the notice appears once.
    assert result.stderr.count(SUNSET_NOTICE) == 1
    assert result.stderr.startswith(SUNSET_NOTICE + "\n")


def test_usage_error_prints_one_notice_line(temp_config):
    result = _invoke(["list", "--no-such-option"])
    assert result.exit_code != 0
    assert result.stderr.count(SUNSET_NOTICE) == 1
    assert result.stderr.startswith(SUNSET_NOTICE + "\n")


def test_json_error_path_prints_one_notice_line(temp_config):
    result = _invoke(["remove", "nonexistent", "--json"])
    assert result.exit_code != 0
    assert result.stderr == SUNSET_NOTICE + "\n"
    assert json.loads(result.stdout)["ok"] is False


def test_nested_group_invocation_prints_notice_once(temp_config):
    """`companion` is itself a KlurisGroup reached via invoke() — the notice
    must not fire twice for nested dispatch."""
    result = _invoke(["companion", "list"])
    assert result.stderr.count(SUNSET_NOTICE) == 1


@pytest.mark.parametrize(
    "args",
    [
        ["--help"],
        ["--version"],
        ["help"],
        ["list"],
        ["companion", "--help"],
        ["not-a-command"],
        ["list", "--no-such-option"],
        ["remove", "nonexistent", "--json"],
    ],
)
def test_notice_suppressed_by_env(args, temp_config):
    result = _invoke(args, env={"KLURIS_NO_SUNSET_NOTICE": "1"})
    assert SUNSET_NOTICE not in result.stderr


@pytest.mark.parametrize("args", JSON_COMMANDS)
def test_json_stdout_identical_with_notice(args, temp_brain):
    """stdout under --json must match byte-for-byte what the same command
    prints with the notice disabled (i.e. v2.28.11 behaviour)."""
    runner = CliRunner()
    suppressed = runner.invoke(cli, args, env={"KLURIS_NO_SUNSET_NOTICE": "1"})
    _restore_brain(temp_brain)
    noticed = runner.invoke(cli, args)
    assert noticed.exit_code == suppressed.exit_code
    assert noticed.stdout == suppressed.stdout
    assert noticed.stderr == SUNSET_NOTICE + "\n"
    assert SUNSET_NOTICE not in suppressed.stderr
