"""Config parity: v3 reads the config.yml and kluris.yml files v2 writes (and
the stale or hand-edited ones v2 tolerates) exactly as v2 does."""

from __future__ import annotations

import json

import pytest

from harness import Sandbox, assert_json_equal, run_both, run_v2

UNICODE_DESCRIPTION = (
    "Équipe données — connaissances partagées über alles, naïve café façade 日本語"
    " and a long tail to fold"
)

LEGACY_AGENTS = ["junie", "claude", "cursor", "windsurf", "copilot", "codex", "gemini", "kilocode"]


def _oracle_ok(sandbox: Sandbox, args: list[str]) -> dict:
    r = run_v2(args, sandbox.env)
    assert r.code == 0, f"oracle setup {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout)


def _agents_yaml(names: list[str]) -> str:
    return "agents:\n  commands_for:\n" + "".join(f"  - {n}\n" for n in names)


def _v2_brains(sandbox: Sandbox):
    """Brains written by v2, then hand-edited the ways v2 still reads."""
    brains = sandbox.work / "brains"
    brains.mkdir()
    for name, extra in (
        ("alpha", []),
        ("beta", ["--description", UNICODE_DESCRIPTION]),
        ("custom", []),
        ("legacy", []),
        ("minimal", []),
        ("broken", []),
    ):
        _oracle_ok(sandbox, ["create", name, "--path", str(brains), "--no-git", *extra, "--json"])
    _oracle_ok(sandbox, ["companion", "add", "specmint-tdd-html", "--brain", "beta", "--json"])
    _oracle_ok(sandbox, ["companion", "add", "specmint-core", "--brain", "beta", "--json"])

    # Stale keys in config.yml: on a brain entry and at the top level.
    text = sandbox.config.read_text(encoding="utf-8")
    text = text.replace(
        "    description: alpha knowledge base\n",
        "    description: alpha knowledge base\n    type: product-group\n    repo: git@example.com:t/a.git\n",
    )
    sandbox.config.write_text(text + "default_brain: alpha\n", encoding="utf-8")

    with (brains / "alpha" / "kluris.yml").open("a", encoding="utf-8") as f:
        f.write("type: product-group\ngit:\n  remote: git@example.com:t/a.git\n  branch: main\n")
    (brains / "custom" / "kluris.yml").write_text(
        "name: custom\ndescription: d\n"
        + _agents_yaml(["claude", "devin", "antigravity", "windsurf", "bogus-agent"])
        + "companions:\n- specmint-tdd\n- bogus\n",
        encoding="utf-8",
    )
    (brains / "legacy" / "kluris.yml").write_text(
        "name: legacy\n" + _agents_yaml(LEGACY_AGENTS) + "companions: [specmint-core]\n",
        encoding="utf-8",
    )
    (brains / "minimal" / "kluris.yml").write_text("name: minimal\n", encoding="utf-8")
    (brains / "broken" / "kluris.yml").write_text("name: broken\ncompanions: null\n", encoding="utf-8")
    return brains


def test_list_json_agrees_on_v2_written_and_stale_configs(sandbox: Sandbox):
    _v2_brains(sandbox)
    sandbox.freeze()

    results = run_both(sandbox, ["list", "--json"])
    v2, v3 = results["v2"], results["v3"]
    assert v2.code == 0, v2.stderr
    payload = json.loads(v2.stdout)
    assert [b["name"] for b in payload["brains"]] == ["alpha", "beta", "custom", "legacy", "minimal", "broken"]
    assert [b["companions"] for b in payload["brains"]] == [
        [], ["specmint-core", "specmint-tdd-html"], ["specmint-tdd"], ["specmint-core"], [], None,
    ]
    assert payload["brains"][1]["description"] == UNICODE_DESCRIPTION
    assert v3.code == v2.code
    assert_json_equal(v2.stdout, v3.stdout, sandbox.root)
    assert v3.tree == v2.tree, "list must not write anything"


@pytest.mark.parametrize(
    "args",
    [
        ["list", "--json"],
        ["companion", "list", "--brain", "all", "--json"],
        ["doctor", "--no-refresh", "--json"],
    ],
)
def test_v2_reads_v3_agent_names(sandbox: Sandbox, args: list[str]):
    """A kluris.yml as v3 writes it (devin, antigravity, unknown names) is
    still read by v2 without error."""
    brains = sandbox.work / "brains"
    brains.mkdir()
    _oracle_ok(sandbox, ["create", "v3brain", "--path", str(brains), "--no-git", "--json"])
    cfg = brains / "v3brain" / "kluris.yml"
    cfg.write_text(
        cfg.read_text(encoding="utf-8").replace("  - windsurf\n", "  - devin\n  - antigravity\n  - bogus-agent\n"),
        encoding="utf-8",
    )
    r = run_v2(args, sandbox.env)
    assert r.code == 0, f"{r.stdout} {r.stderr}"
    assert json.loads(r.stdout)["ok"] is True


def test_v2_reads_a_config_rewritten_by_v3():
    pytest.skip("no v3 command writes config.yml or kluris.yml yet (create/register/companion/remove come later)")
