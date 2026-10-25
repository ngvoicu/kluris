"""Smoke parity: the binaries start, agree on `list --json`, and share the
JSON error envelope and exit codes."""

from __future__ import annotations

import json

import pytest

from harness import Sandbox, assert_json_equal, normalize, run_both, run_v2


def test_version(sandbox: Sandbox):
    results = run_both(sandbox, ["--version"])
    for side, r in results.items():
        assert r.code == 0, f"{side}: {r.stderr}"
        assert normalize(r.stdout, sandbox.root) == "kluris <VERSION>\n", side


def test_list_json_without_config(sandbox: Sandbox):
    assert not sandbox.config.exists()
    results = run_both(sandbox, ["list", "--json"])
    v2, v3 = results["v2"], results["v3"]
    assert v2.code == 0, v2.stderr
    assert v3.code == v2.code
    assert_json_equal(v2.stdout, v3.stdout, sandbox.root)


def _oracle_ok(sandbox: Sandbox, args: list[str]) -> dict:
    r = run_v2(args, sandbox.env)
    assert r.code == 0, f"oracle setup {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout)


def test_list_json_two_brains(sandbox: Sandbox):
    brains = sandbox.work / "brains"
    brains.mkdir()
    _oracle_ok(sandbox, ["create", "a", "--path", str(brains), "--no-git", "--json"])
    _oracle_ok(sandbox, ["create", "b", "--path", str(brains), "--json"])
    _oracle_ok(sandbox, ["companion", "add", "specmint-core", "--brain", "b", "--json"])
    sandbox.freeze()

    results = run_both(sandbox, ["list", "--json"])
    v2, v3 = results["v2"], results["v3"]
    assert v2.code == 0, v2.stderr
    assert [b["companions"] for b in json.loads(v2.stdout)["brains"]] == [[], ["specmint-core"]]
    assert v3.code == v2.code
    assert_json_equal(v2.stdout, v3.stdout, sandbox.root)
    assert v3.tree == v2.tree, "list must not write anything"


def test_unknown_command_json_envelope(sandbox: Sandbox):
    results = run_both(sandbox, ["bogus", "--json"])
    for side, r in results.items():
        assert r.code == 1, f"{side}: exit {r.code}"
        lines = r.stdout.splitlines()
        assert len(lines) == 1, f"{side}: stdout is not one line: {r.stdout!r}"
        payload = json.loads(lines[0])
        assert payload["ok"] is False, side
        assert isinstance(payload["error"], str) and payload["error"], side
        assert list(payload) == ["ok", "error"], side


def test_no_arguments_exits_2(sandbox: Sandbox):
    results = run_both(sandbox, [])
    for side, r in results.items():
        assert r.code == 2, f"{side}: exit {r.code}"


@pytest.mark.parametrize(
    "args",
    [
        ["bogus"],
        ["--json"],
        ["list", "--bogus"],
        ["list", "--bogus", "--json"],
    ],
    ids=["unknown-command", "root-json-flag", "unknown-option", "unknown-option-json"],
)
def test_usage_error_exit_codes(sandbox: Sandbox, args: list[str]):
    """Usage errors exit 2 unless `--json` reaches a subcommand, which turns them
    into the JSON envelope with exit 1 (a root-level `--json` does not)."""
    results = run_both(sandbox, args)
    v2, v3 = results["v2"], results["v3"]
    assert v3.code == v2.code, f"exit v3 {v3.code} != v2 {v2.code}"
    if v2.stdout:
        payload = json.loads(v3.stdout)
        assert payload["ok"] is False and isinstance(payload["error"], str)
    else:
        assert v3.stdout == ""
