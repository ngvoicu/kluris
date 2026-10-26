"""`kluris doctor` parity: a HOME written by v2.28.11 itself (`create`,
`companion add`, `doctor`), then v2's and v3's `doctor` on identical copies.

Expected (parity/allowlist.md, the doctor entries):

- v3 migrates the skills exactly as the install does (the skill-install
  entries: `test_skill_install._v2_to_v3`) and touches nothing else: the
  config, the companions (rewritten with the same bytes), the brains;
- the `git` check and the companion checks and `companions[]` are v2's;
  `python` is gone, `config_dir` became `config` plus one `brain:<name>` per
  brain, `path` is new, and `skills` keeps v2's detail format with v3's
  counts; `skills` (the per-folder report) is a new key;
- the text output keeps v2's `  PASS|FAIL <name>: <detail>` lines;
- `--no-refresh` writes nothing, on either side;
- a failed check exits 1 on both sides.
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

import pytest

from harness import V3_BIN, Sandbox, run_v2, run_v3
from runs import same_words
from test_skill_install import BRAINS, V2_ONLY, V3_WRITES, _tree, _v2_home, _v2_to_v3

SKILLS_DETAIL = re.compile(r"\d+ files written for \d+ agents across (\d+) brain\(s\)")


@pytest.fixture
def sandbox(tmp_path):
    return Sandbox(tmp_path)


def _v3_env(sandbox: Sandbox, path: str | None = None) -> dict[str, str]:
    """The sandbox env with this kluris first on PATH (so the PATH check is
    deterministic on a machine that has another `kluris` installed)."""
    assert V3_BIN is not None, "KLURIS_V3_BIN is not set"
    rest = sandbox.env.get("PATH", "") if path is None else path
    return dict(sandbox.env, PATH=os.pathsep.join(p for p in (str(V3_BIN.parent), rest) if p))


def _run(sandbox: Sandbox, side: str, args: list[str], env: dict[str, str] | None = None):
    """Run one side on its own copy of the frozen state; returns the result
    and the HOME and brains trees after it."""
    sandbox.checkout()
    try:
        if side == "v2":
            r = run_v2(args, env or sandbox.env)
        else:
            r = run_v3(args, env or _v3_env(sandbox))
        trees = (_tree(sandbox.home), _tree(sandbox.work / "brains"))
    finally:
        sandbox.stash(side)
    return r, trees


def _checks(doc: dict) -> dict[str, dict]:
    return {c["name"]: c for c in doc["checks"]}


def _expected_migration(before: dict[str, bytes]) -> dict[str, bytes]:
    expected = dict(before)
    for brain in BRAINS:
        v2 = before[f".claude/skills/kluris-{brain}/SKILL.md"].decode("utf-8").replace("\r\n", "\n")
        v3 = _v2_to_v3(v2, brain).encode("utf-8")
        for rel in V3_WRITES:
            expected[f"{rel}/kluris-{brain}/SKILL.md"] = v3
        for rel in V2_ONLY:
            del expected[f"{rel}/kluris-{brain}/SKILL.md"]
        del expected[f".codeium/windsurf/global_workflows/kluris-{brain}.md"]
    return expected


def test_v2_home_is_migrated_by_doctor(sandbox: Sandbox):
    _v2_home(sandbox)
    before_home, before_brains = _tree(sandbox.home), _tree(sandbox.work / "brains")
    sandbox.freeze()

    v2, _ = _run(sandbox, "v2", ["doctor", "--json"])
    v3, (home, brains) = _run(sandbox, "v3", ["doctor", "--json"])
    assert v2.code == v3.code == 0, (v2.stdout, v3.stdout, v3.stderr)

    changed = sorted(k for k in set(home) | set(_expected_migration(before_home))
                     if home.get(k) != _expected_migration(before_home).get(k))
    assert not changed, changed
    assert brains == before_brains

    d2, d3 = json.loads(v2.stdout), json.loads(v3.stdout)
    assert list(d2) == ["ok", "checks", "companions"]
    assert list(d3) == ["ok", "checks", "companions", "skills"]
    assert d3["ok"] is d2["ok"] is True
    assert [c["name"] for c in d2["checks"]] == [
        "git", "python", "config_dir", "companion:specmint-core", "skills"]
    assert [c["name"] for c in d3["checks"]] == [
        "git", "path", "config", "brain:acme", "brain:beta", "companion:specmint-core", "skills"]
    c2, c3 = _checks(d2), _checks(d3)
    assert c3["git"] == c2["git"]
    assert c3["companion:specmint-core"] == c2["companion:specmint-core"]
    assert d3["companions"] == d2["companions"] == [{"name": "specmint-core", "refreshed": True}]
    assert "warning" not in c3["path"], c3["path"]
    assert c3["config"]["detail"] == str(sandbox.config)
    for name in ("skills",):
        m2, m3 = SKILLS_DETAIL.fullmatch(c2[name]["detail"]), SKILLS_DETAIL.fullmatch(c3[name]["detail"])
        assert m2 and m3, (c2[name], c3[name])
        assert m2.group(1) == m3.group(1) == "2"
        assert c3[name]["passed"] is c2[name]["passed"] is True

    removed = {d["path"]: d["removed"] for d in d3["skills"]["destinations"]}
    cursor = (sandbox.home / ".cursor/skills").as_posix()
    assert removed[cursor] == ["kluris-acme", "kluris-beta"]


def test_text_output_keeps_v2s_lines(sandbox: Sandbox):
    _v2_home(sandbox)
    sandbox.freeze()
    v2, _ = _run(sandbox, "v2", ["doctor"])
    v3, _ = _run(sandbox, "v3", ["doctor"])
    assert v2.code == v3.code == 0, (v2.stdout, v3.stdout)

    def line(text: str, name: str) -> str:
        # Rich wraps long lines; the check's text runs to the next check.
        found = re.search(rf"^  (PASS|FAIL) {re.escape(name)}: .*?(?=^  [A-Z]{{4}} |\Z)", text, re.M | re.S)
        assert found, (name, text)
        return found.group(0)

    for name in ("git", "companion:specmint-core"):
        same_words(line(v2.stdout, name), line(v3.stdout, name), sandbox.root)
    skills = line(v3.stdout, "skills").splitlines()[0]
    assert SKILLS_DETAIL.fullmatch(skills.removeprefix("  PASS skills: ")), skills


def test_no_refresh_writes_nothing(sandbox: Sandbox):
    _v2_home(sandbox)
    before = (_tree(sandbox.home), _tree(sandbox.work / "brains"))
    sandbox.freeze()
    v2, after2 = _run(sandbox, "v2", ["doctor", "--no-refresh", "--json"])
    v3, after3 = _run(sandbox, "v3", ["doctor", "--no-refresh", "--json"])
    assert v2.code == v3.code == 0, (v2.stdout, v3.stdout)
    assert after3 == after2 == before
    d2, d3 = json.loads(v2.stdout), json.loads(v3.stdout)
    assert d3["companions"] == d2["companions"] == []
    assert d3["skills"] is None
    assert [c["name"] for c in d3["checks"]] == ["git", "path", "config", "brain:acme", "brain:beta"]


def test_a_failed_check_exits_1_on_both_sides(sandbox: Sandbox):
    """No git on PATH: v2's `FileNotFoundError` branch, same text."""
    empty = sandbox.root / "empty-path"
    empty.mkdir()
    sandbox.freeze()
    v2, _ = _run(sandbox, "v2", ["doctor", "--no-refresh", "--json"], dict(sandbox.env, PATH=str(empty)))
    v3, _ = _run(sandbox, "v3", ["doctor", "--no-refresh", "--json"], _v3_env(sandbox, str(empty)))
    assert v2.code == v3.code == 1, (v2.stdout, v3.stdout, v3.stderr)
    d2, d3 = json.loads(v2.stdout), json.loads(v3.stdout)
    assert d3["ok"] is d2["ok"] is False
    assert _checks(d3)["git"] == _checks(d2)["git"] == {
        "name": "git", "passed": False, "detail": "git not found. Install: https://git-scm.com/downloads"}
