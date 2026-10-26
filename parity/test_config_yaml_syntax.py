"""Parity: `kluris list --json` reads `~/.kluris/config.yml` with PyYAML's
YAML 1.1 syntax, flow-style corner cases included."""

from __future__ import annotations

import pytest

from harness import Sandbox, assert_json_equal, run_both


@pytest.mark.parametrize(
    "config",
    [
        # `alpha:{` is a key and a flow mapping in PyYAML (a `:` followed by a
        # flow indicator ends a plain scalar in a flow collection).
        "brains: {alpha:{path: /missing}}\n",
        "brains: {alpha:{path: /missing, description: d}, beta: {path: /b}}\n",
        "brains:\n  alpha: {path: /missing, description: [a,b]}\n",
    ],
    ids=["glued-flow-mapping", "two-flow-brains", "flow-in-block"],
)
def test_list_reads_flow_config_like_v2(sandbox: Sandbox, config: str):
    sandbox.config.parent.mkdir(parents=True, exist_ok=True)
    sandbox.config.write_text(config, encoding="utf-8")
    sandbox.freeze()
    results = run_both(sandbox, ["list", "--json"])
    v2, v3 = results["v2"], results["v3"]
    assert v3.code == v2.code, f"exit v3 {v3.code} != v2 {v2.code}: {v3.stdout} {v3.stderr}"
    if v2.code == 0:
        assert_json_equal(v2.stdout, v3.stdout, sandbox.root)


@pytest.mark.parametrize(
    "config",
    [
        "brains: {}\nextra: [&a?x]\n",
        "brains: {}\nextra: {!!str &a: |}\n",
        "brains: {}\nextra: \"a\x07\"\n",
    ],
    ids=["anchor-before-question-mark", "block-scalar-in-flow", "non-printable"],
)
@pytest.mark.parametrize("args", [["list"], ["list", "--json"]], ids=["text", "json"])
def test_list_fails_on_invalid_config_like_v2(sandbox: Sandbox, config: str, args: list[str]):
    sandbox.config.parent.mkdir(parents=True, exist_ok=True)
    sandbox.config.write_text(config, encoding="utf-8")
    sandbox.freeze()
    results = run_both(sandbox, args)
    v2, v3 = results["v2"], results["v3"]
    assert v2.code != 0, f"v2 accepted {config!r}: {v2.stdout}"
    assert v3.code == v2.code, f"exit v3 {v3.code} != v2 {v2.code}: {v3.stdout} {v3.stderr}"
