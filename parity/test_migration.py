"""The v2 -> v3 migration scenario (SPEC TEST-K3-41): a machine Kluris
v2.28.11 set up, then the first commands a user runs after installing
Kluris 3.

1. v2 (the oracle) writes the world in a sandbox HOME: `create` a product
   brain and a blank brain (git on both), `companion add specmint-core`,
   `doctor` with v2's default `commands_for` (every v2 agent: claude, cursor,
   windsurf, copilot, codex, gemini, hermes with two profiles, kilocode,
   junie), neurons written the way an agent writes them (unquoted dates,
   `tags`/`related`, one deprecated, `dream` then a commit, one more neuron
   written after the last commit and not dreamed yet), then the user's own
   `~/.cursor/skills/kluris-notes` and a foreign entry in `~/.agents/skills`.
2. Everything is snapshotted: the brains (bytes, `git log`, `git status`),
   `~/.kluris/config.yml`, every skill folder and Windsurf workflow (the whole
   HOME).
3. v3 runs `doctor --json`, `wake-up --json`, `search --json` (two queries),
   `dream --json` on each brain, then `doctor --json` again. v2 runs the same
   brain commands (`wake-up`, `search`, `dream`) on its own identical copy.
4. Asserted:
   - every exit code is 0;
   - the brains: v3's `dream` output (JSON and every file) is v2's own, but
     for the allowlisted differences named in ``DREAM_DIFFERENCES``; `git log`
     is the snapshot's on both sides and `git status` is the same; every
     `kluris.yml` is byte-identical (v2's default `commands_for` *reads* as
     v3's, PAR-003; nothing rewrites it);
   - `config.yml` is byte-identical (no allowlisted key changes it);
   - the skills are the matrix (PAR-030, PAR-031): v3's SKILL.md (PAR-026..029)
     in `~/.agents/skills`, `~/.claude/skills`, `~/.hermes/skills` and both
     profiles', and `~/.gemini/config/skills` (v2's default list reads as v3's
     default, which adds Antigravity: PAR-003, PAR-030); v2's copies in
     cursor/codex/copilot/gemini/kilo/junie and the Windsurf skills and
     `global_workflows/kluris-*.md` removed, their folders kept;
   - the user's `kluris-notes` and the foreign `~/.agents/skills` entry are
     byte-identical, and not listed in `kept` (PAR-031);
   - the second `doctor` changes no file;
   - `wake-up` equals v2's (PAR-008, PAR-037); `search` keeps v2's keys, items,
     `file_type`, `deprecated` and `matched_fields` (PAR-020: v3 ranks with
     BM25 and adds keys).
5. The variant with `COPILOT_HOME` set: v2's `~/.copilot/skills` copies stay,
   and v3 writes `$COPILOT_HOME/skills` (PAR-030, PAR-031).

Nothing here needs an OS skip: no symlinks, no permissions, plain folders.
On Windows v2 writes text mode (CRLF) and v3 LF; the expectations follow
PAR-029 (SKILL.md) and PAR-035 (brain files), as the reused helpers do.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from harness import Result, Sandbox, run_v2, run_v3
from runs import assert_wake_up_equal, parse_exact
from test_doctor import _v3_env
from test_dream import Outcome, assert_step, commit, git, porcelain, write
from test_search import check_against_v2, check_shape, oracle_search
from test_skill_install import FOREIGN_SKILL, V2_ONLY, _tree, _v2_to_v3

# name -> `create --type`
BRAINS = {"prod": "product", "team": "blank"}
PROFILES = ["work", "personal"]
V2_AGENTS = ["claude", "cursor", "windsurf", "copilot", "codex", "gemini", "hermes", "kilocode", "junie"]

# Folders v3 writes (relative to HOME); v2 wrote all but the Antigravity one.
V3_WRITES = [
    ".agents/skills",
    ".claude/skills",
    ".hermes/skills",
    *(f".hermes/profiles/{p}/skills" for p in PROFILES),
    ".gemini/config/skills",
]
WORKFLOWS = ".codeium/windsurf/global_workflows"

# The user's own things (relative to HOME), added after v2's last install
# (v2's sweep deletes every `kluris*` entry).
USER_FILES = {
    ".cursor/skills/kluris-notes/SKILL.md": FOREIGN_SKILL.encode("utf-8"),
    ".agents/skills/my-tool/SKILL.md": b"---\nname: my-tool\ndescription: Not Kluris's\n---\n\n# My tool\n",
    ".agents/skills/my-tool/run.sh": b"#!/bin/sh\necho mine\n",
    ".agents/skills/README.txt": b"skills I installed by hand\n",
}

# Neurons as an agent writes them (the skill's writing rules: parent,
# related, tags, created, updated; dates unquoted, as agents type them).
PROD_NEURONS = {
    "prod/features/login.md": """---
parent: ./map.md
related:
  - ../decisions/jwt-auth.md
tags: [auth, login]
created: 2026-01-01
updated: 2026-01-01
---

# Login flow

Users sign in with email and password; the API returns a JWT access token.
""",
    "prod/decisions/jwt-auth.md": """---
parent: ./map.md
related:
  - ../features/login.md
tags:
  - auth
  - decision
created: 2026-01-01
updated: 2026-01-01
---

# Use JWT access tokens

We chose stateless JWT access tokens over server sessions.
""",
    "prod/decisions/server-sessions.md": """---
parent: ./map.md
tags: [auth]
created: 2026-01-01
updated: 2026-01-01
status: deprecated
deprecated_at: 2026-01-01
replaced_by: ./jwt-auth.md
---

# Server-side sessions

Sessions lived in Redis. Superseded by JWT access tokens.
""",
}
TEAM_NEURONS = {
    "team/process/standup.md": """---
parent: ./map.md
tags: [process]
created: 2026-01-01
updated: 2026-01-01
---

# Daily standup

Fifteen minutes at 9:30, blockers first.
""",
}
# Written by an agent after the last commit, not dreamed yet: no `parent`,
# unquoted dates.
PENDING = {
    "prod/features/search.md": """---
tags: [search]
created: 2026-01-01
updated: 2026-01-01
---

# Search

Full-text search over products; see [login](./login.md) for auth.
""",
}

QUERIES = {"prod": ["access token", "sessions"], "team": ["standup", "blockers"]}

# The files (brain-relative) whose bytes may differ after `dream` on each
# brain, each with the allowlist entry that allows it.
# `test_dream.compare_trees` checks each is exactly the converged form of
# v2's file.
DREAM_DIFFERENCES = {
    "prod": {
        # v2's parent fix writes the neuron's dates unquoted, v3 quoted.
        "features/search.md": "PAR-033",
    },
    "team": {},
}


def _oracle_ok(sb: Sandbox, args: list[str]) -> dict:
    r = run_v2(args, sb.env)
    assert r.code == 0, f"oracle {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout)


def _git_log(sb: Sandbox, repo: Path) -> str:
    return git(sb, repo, "log", "--format=%H %aI %s")


def _v2_world(sb: Sandbox) -> Path:
    brains = sb.work / "brains"
    brains.mkdir()
    for profile in PROFILES:
        (sb.home / ".hermes" / "profiles" / profile).mkdir(parents=True)
    for name, kind in BRAINS.items():
        _oracle_ok(sb, ["create", name, "--type", kind, "--path", str(brains),
                        "--description", f"The {name} brain", "--json"])
    _oracle_ok(sb, ["companion", "add", "specmint-core", "--brain", "prod", "--json"])
    doc = _oracle_ok(sb, ["doctor", "--json"])
    assert doc["ok"] is True, doc

    # An agent writes neurons, runs dream, and the human commits.
    write(brains, {**PROD_NEURONS, **TEAM_NEURONS})
    for name in BRAINS:
        dream = _oracle_ok(sb, ["dream", "--brain", name, "--json"])
        assert dream["healthy"] is True, dream
        commit(sb, brains / name, f"brain: add {name} neurons")
    write(brains, PENDING)

    for rel, data in USER_FILES.items():
        (sb.home / rel).parent.mkdir(parents=True, exist_ok=True)
        (sb.home / rel).write_bytes(data)
    return brains


def _expected_home(before: dict[str, bytes], copilot_home: bool) -> dict[str, bytes]:
    expected = dict(before)
    for brain in BRAINS:
        v2 = before[f".claude/skills/kluris-{brain}/SKILL.md"].decode("utf-8").replace("\r\n", "\n")
        v3 = _v2_to_v3(v2, brain).encode("utf-8")
        for rel in V3_WRITES + (["copilot-home/skills"] if copilot_home else []):
            expected[f"{rel}/kluris-{brain}/SKILL.md"] = v3
        for rel in V2_ONLY:
            if copilot_home and rel == ".copilot/skills":
                continue  # PAR-031: kept while COPILOT_HOME is set
            del expected[f"{rel}/kluris-{brain}/SKILL.md"]
        del expected[f"{WORKFLOWS}/kluris-{brain}.md"]
    return expected


def _diff(a: dict[str, bytes], b: dict[str, bytes]) -> list[str]:
    return sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))


def _search_args(brain: str, query: str) -> list[str]:
    return ["search", "--brain", brain, "--json", "--limit", "1000", "--", query]


def _run_side(sb: Sandbox, side: str, env: dict[str, str]) -> dict:
    """The command sequence on a fresh copy of the frozen world; every result
    and the trees it needs."""
    brains = sb.work / "brains"
    out: dict = {"results": [], "dream": {}, "wake_up": {}, "search": {}, "oracle": {}}
    run = run_v3 if side == "v3" else run_v2

    def step(args: list[str]) -> Result:
        r = run(args, env)
        out["results"].append((args, r))
        return r

    sb.checkout()
    try:
        if side == "v3":
            out["doctor1"] = step(["doctor", "--json"])
            out["after_doctor1"] = (_tree(sb.home), _tree(brains))
        for name in BRAINS:
            out["wake_up"][name] = step(["wake-up", "--brain", name, "--json"])
        for name in BRAINS:
            for q in QUERIES[name]:
                out["search"][(name, q)] = step(_search_args(name, q))
            if side == "v2":
                queries = [{"q": q, "limit": 1000} for q in QUERIES[name]]
                out["oracle"][name] = oracle_search(brains / name, queries)
        for name in BRAINS:
            before = _tree(brains / name)
            r = step(["dream", "--brain", name, "--json"])
            r.tree = _tree(brains / name)
            out["dream"][name] = (before, Outcome(r, porcelain(sb, [f"brains/{name}"])))
        out["brains"] = _tree(brains)
        out["git_log"] = {name: _git_log(sb, brains / name) for name in BRAINS}
        if side == "v3":
            out["before_doctor2"] = (_tree(sb.home), _tree(brains))
            out["doctor2"] = step(["doctor", "--json"])
            out["after_doctor2"] = (_tree(sb.home), _tree(brains))
        out["home"] = _tree(sb.home)
        out["config"] = sb.config.read_bytes()
    finally:
        sb.stash(side)
    return out


@pytest.mark.parametrize("copilot_home", [False, True], ids=["default", "COPILOT_HOME"])
def test_a_v2_machine_moves_to_v3(sandbox: Sandbox, copilot_home: bool):
    sb = sandbox
    brains = _v2_world(sb)
    home_before, brains_before = _tree(sb.home), _tree(brains)
    config_before = sb.config.read_bytes()
    logs_before = {name: _git_log(sb, brains / name) for name in BRAINS}

    # The world is v2's (guards the fixture).
    for name in BRAINS:
        assert f"{name}/kluris.yml" in brains_before
        listed = brains_before[f"{name}/kluris.yml"].decode("utf-8").replace("\r\n", "\n")
        assert "agents:\n  commands_for:\n" + "".join(f"  - {a}\n" for a in V2_AGENTS) in listed, listed
        for rel in V2_ONLY + [r for r in V3_WRITES if r != ".gemini/config/skills"]:
            assert f"{rel}/kluris-{name}/SKILL.md" in home_before, rel
        assert f"{WORKFLOWS}/kluris-{name}.md" in home_before
    assert not any(k.startswith(".gemini/config/") for k in home_before)
    for name in BRAINS:
        assert logs_before[name].count("\n") == 2, logs_before[name]

    sb.freeze()
    extra = {"COPILOT_HOME": str(sb.home / "copilot-home")} if copilot_home else {}
    v2 = _run_side(sb, "v2", dict(sb.env, **extra))
    v3 = _run_side(sb, "v3", dict(_v3_env(sb), **extra))

    # Every exit code is 0.
    failed = [(side, args, r.code, r.stdout[-800:], r.stderr[-800:])
              for side, out in (("v2", v2), ("v3", v3)) for args, r in out["results"] if r.code != 0]
    assert not failed, failed

    # doctor: passes, and leaves the brains alone.
    for key in ("doctor1", "doctor2"):
        doc = parse_exact(v3[key].stdout, sb.root, "v3")
        assert doc["ok"] is True, doc
        assert {c["name"]: c["passed"] for c in doc["checks"]}["skills"] is True, doc["checks"]
        for dest in doc["skills"]["destinations"]:
            assert dest["failed"] == [], dest
            assert "kluris-notes" not in dest["kept"] and "my-tool" not in dest["kept"], dest
    assert v3["after_doctor1"][1] == brains_before

    # The skills are the matrix, the user's entries untouched, config.yml
    # and the companions unchanged.
    expected = _expected_home(home_before, copilot_home)
    assert not _diff(v3["after_doctor1"][0], expected), _diff(v3["after_doctor1"][0], expected)
    assert not _diff(v3["home"], expected), _diff(v3["home"], expected)
    for rel, data in USER_FILES.items():
        assert v3["home"][rel] == data, rel
    for rel in V2_ONLY + [WORKFLOWS]:
        assert (sb.root / "v3" / "home" / rel).is_dir(), rel
    assert v3["config"] == config_before
    copies = {v3["home"][f"{rel}/kluris-prod/SKILL.md"] for rel in V3_WRITES}
    assert len(copies) == 1, "the copies of one skill are not byte-identical"

    # The second doctor changes nothing.
    before2, after2 = v3["before_doctor2"], v3["after_doctor2"]
    assert not _diff(before2[0], after2[0]), _diff(before2[0], after2[0])
    assert not _diff(before2[1], after2[1]), _diff(before2[1], after2[1])
    doc2 = parse_exact(v3["doctor2"].stdout, sb.root, "v3")
    assert all(d["removed"] == [] for d in doc2["skills"]["destinations"]), doc2["skills"]

    # wake-up agrees with v2's.
    for name in BRAINS:
        assert_wake_up_equal(v2["wake_up"][name], v3["wake_up"][name], sb.root)

    # search agrees with v2's (PAR-020: v3's own ranking and extra keys).
    failures = []
    for name in BRAINS:
        oracle = v2["oracle"][name]
        for q in QUERIES[name]:
            r2, r3 = v2["search"][(name, q)], v3["search"][(name, q)]
            d2, d3 = parse_exact(r2.stdout, sb.root, "v2"), parse_exact(r3.stdout, sb.root, "v3")
            assert list(d2) == ["ok", "brain", "query", "total", "results"], list(d2)
            assert d2["results"], f"v2 finds nothing for {q!r} in {name}: the query tests nothing"
            check_shape(d3, name, q, 1000)
            if q == "sessions":
                assert any(r["deprecated"] for r in d2["results"]), "the deprecated neuron is not hit"
            allowed = oracle["items"][json.dumps([None, None])]
            failures += check_against_v2(name, {"q": q, "limit": 1000}, d3, d2["results"], allowed)
    assert not failures, failures

    # The brains: v3's dream is v2's, but for the named differences.
    for name in BRAINS:
        before, o2 = v2["dream"][name]
        before3, o3 = v3["dream"][name]
        assert before3 == before, f"{name} differs before its dream"
        differ = assert_step(sb, o2, o3, before)
        assert sorted(differ) == sorted(DREAM_DIFFERENCES[name]), (name, differ)
    assert v3["git_log"] == v2["git_log"] == logs_before
    for name in BRAINS:
        rel = f"{name}/kluris.yml"
        assert v3["brains"][rel] == v2["brains"][rel] == brains_before[rel], rel
