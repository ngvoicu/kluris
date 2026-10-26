"""``kluris dream`` parity against the v2.28.11 oracle (research-03 §1.10, §2.5).

Each scenario runs the same steps (dreams, commits with pinned author dates)
on two identical copies of a brain, one per binary, and compares after every
dream: the exit code, the JSON (``deprecation`` and ``broken_synapses_detail``
as multisets, PAR-008), the whole brain tree byte for byte and ``git status
--porcelain``. A file whose bytes differ must be exactly the converged form of
v2's file (``tools/dream_oracle.py``): quoted top-level dates (PAR-033),
anchors as plain copies (PAR-034), the file's own line endings with new files
LF (PAR-035), and reverse ``related`` links in Kluris 3's path order (PAR-008).
``map.md`` in directories the walker prunes, and their listing, is PAR-036.
The text output is compared by its wording.
"""

from __future__ import annotations

import base64
import datetime
import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

from harness import IS_WINDOWS, PARITY_DIR, Result, Sandbox, _decode, normalize, oracle_kluris, snapshot_tree
from runs import _env, _exe, clear_runs, issue_key, parse_exact, same_words, slash_paths, write_config

DREAM_ORACLE = PARITY_DIR / "tools" / "dream_oracle.py"
D1 = "2026-01-01T12:00:00+00:00"
D2 = "2026-02-03T09:30:00+00:00"
D3 = "2026-03-04T08:00:00+00:00"


# --- brains -------------------------------------------------------------------


def write(root: Path, files: dict[str, str | bytes]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(text, bytes):
            path.write_bytes(text)
        else:
            path.write_bytes(text.encode("utf-8"))


def git(sb: Sandbox, repo: Path, *args: str, date: str = D1) -> str:
    env = _env(sb, {"GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date})
    out = subprocess.run(["git", *args], cwd=repo, env=env, capture_output=True, text=True, encoding="utf-8")
    assert out.returncode == 0, f"git {args}: {out.stderr}"
    return out.stdout


def init(sb: Sandbox, repo: Path) -> None:
    git(sb, repo, "init", "-q")
    git(sb, repo, "config", "core.autocrlf", "false")
    git(sb, repo, "checkout", "-q", "-b", "main")


def commit(sb: Sandbox, repo: Path, message: str, date: str = D1) -> None:
    git(sb, repo, "add", "-A")
    git(sb, repo, "commit", "-q", "--allow-empty", "-m", message, date=date)


def neuron(title: str, *, parent: str = "./map.md", created: str = "'2026-01-01'",
           updated: str = "'2026-01-01'", extra: str = "", body: str = "") -> str:
    head = f"---\ncreated: {created}\n{extra}parent: {parent}\nupdated: {updated}\n---\n\n"
    return head + f"# {title}\n\n{body or 'Body of ' + title + '.'}\n"


GLOSSARY = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n**JWT** -- JSON Web Token.\n"

AGENT_STYLE = """---
parent: ./map.md
title: Agent written
created: 2026-01-01
updated: 2025-12-31
# a comment the writer drops
tags: [auth, "jwt", on, off, 2026, 1.10]
related:
  - ./alpha.md
base: &shared [x, y]
copy: *shared
when: 2026-01-02T10:00:00Z
nested: [2026-01-03]
emoji: "\U0001F680 launch"
long: "a long line that goes past the eighty column limit so that the writer folds it onto two lines"
---

# Agent written

Text with [alpha](./alpha.md).
"""

LINKER_STYLE = """---
created: 2026-01-01
updated: 2026-01-01
tags:
  - linker
when: 2026-01-02T10:00:00Z
---

# Linker written
"""

SPEC_YML = """#---
# title: API spec
# updated: '2025-06-01'
# related:
# - ../alpha.md
#---
openapi: 3.0.0
info:
  title: demo
"""


def rich_files() -> dict[str, str | bytes]:
    """Every dream rule in one brain: lobes, a sub-lobe, siblings, root and
    uncommitted neurons, the three fixes, many reverse links to one target,
    agent-style frontmatter, a YAML neuron, deprecation of every kind, broken
    links, a CRLF file and an upper-case suffix."""
    files: dict[str, str | bytes] = {
        "kluris.yml": "name: rich\ndescription: The rich fixture brain\n",
        "glossary.md": GLOSSARY,
        "README.md": "# Rich\n\nA [link that is not checked](./nowhere.md).\n",
        "root-note.md": neuron("Root note"),
        "projects/alpha.md": neuron("Alpha", extra="related:\n- ../knowledge/beta.md\n"),
        "projects/agent.md": AGENT_STYLE,
        "projects/no-frontmatter.md": "# No frontmatter\n\nJust text.\n",
        "projects/no-parent.md": LINKER_STYLE,
        "projects/api/endpoints.md": neuron("Endpoints", extra="tags:\n- api\n"),
        "projects/api/spec.yml": SPEC_YML,
        "projects/api/raw.yaml": "not: a neuron\n",
        "projects/api/Shout.MD": "# Shouted\n",
        "knowledge/beta.md": neuron("Beta"),
        "knowledge/target.md": neuron("Target", extra="related: []\n"),
        "knowledge/old.md": neuron("Old", extra="status: deprecated\n"),
        "knowledge/older.md": neuron("Older", extra="replaced_by: ./gone.md\nstatus: deprecated\n"),
        "knowledge/newer.md": neuron("Newer", extra="replaced_by: ./old.md\nstatus: deprecated\n"),
        "knowledge/user.md": neuron("User", extra="related:\n- ./old.md\n- ./target.md\n"),
        "knowledge/broken.md": neuron(
            "Broken", extra="related:\n- ./gone.md\n- ./beta.md\n", body="See [x](./missing.md#top) and [web](https://x.io)."
        ),
        "knowledge/crlf.md": neuron("Crlf", updated="'2025-01-01'").replace("\n", "\r\n").encode("utf-8"),
        "knowledge/deep/deeper/leaf.md": neuron("Leaf"),
        "my-lobe/thing.md": neuron("Thing"),
        "uncommitted-lobe/later.md": neuron("Later", updated="'2025-05-05'"),
    }
    for name in ("zeta", "alpha-2", "mid", "Beta", "_u", "a-b", "a.b"):
        files[f"knowledge/src/{name}.md"] = neuron(name, extra="related:\n- ../target.md\n")
    return files


def rich_brain(sb: Sandbox, name: str = "rich") -> Path:
    brain = sb.work / "brains" / name
    files = rich_files()
    later = {k: files.pop(k) for k in list(files) if k.startswith("uncommitted-lobe/")}
    write(brain, files)
    init(sb, brain)
    commit(sb, brain, "initial", D1)
    write(brain, {"projects/alpha.md": neuron("Alpha", extra="related:\n- ../knowledge/beta.md\n", body="Changed.")})
    commit(sb, brain, "alpha changed", D2)
    write(brain, later)
    return brain


def healthy_files() -> dict[str, str]:
    return {
        "kluris.yml": "name: calm\ndescription: A healthy brain\n",
        "glossary.md": GLOSSARY,
        "projects/alpha.md": neuron("Alpha", extra="related:\n- ./beta.md\n"),
        "projects/beta.md": neuron("Beta", extra="related:\n- ./alpha.md\n"),
        "knowledge/gamma.md": neuron("Gamma"),
    }


def healthy_brain(sb: Sandbox, name: str = "calm", git_repo: bool = True) -> Path:
    brain = sb.work / "brains" / name
    write(brain, healthy_files())
    if git_repo:
        init(sb, brain)
        commit(sb, brain, "initial", D1)
    return brain


def register(sb: Sandbox, brains: list[Path], description: str = "fixture") -> None:
    write_config(sb, [(b.name, str(b), description) for b in brains])
    sb.freeze()


# --- running ------------------------------------------------------------------


@dataclass
class Step:
    """A dream (``args``) or a commit of every brain (``commit`` = date)."""
    args: list[str] | None = None
    commit: str | None = None


@dataclass
class Outcome:
    result: Result
    porcelain: dict[str, str]


def porcelain(sb: Sandbox, brains: list[str]) -> dict[str, str]:
    out = {}
    for rel in brains:
        repo = sb.work / rel
        if (repo / ".git").exists():
            out[rel] = git(sb, repo, "-c", "core.quotePath=false", "status", "--porcelain")
    return out


def run_steps(sb: Sandbox, brains: list[str], steps: list[Step], env: dict | None = None) -> dict[str, list[Outcome]]:
    """Run ``steps`` with each binary on its own copy of the frozen state."""
    outcomes: dict[str, list[Outcome]] = {}
    for side in ("v2", "v3"):
        sb.checkout()
        try:
            got = []
            for step in steps:
                if step.commit:
                    for rel in brains:
                        commit(sb, sb.work / rel, "between dreams", step.commit)
                    continue
                proc = subprocess.run(
                    [*_exe(side), *step.args], env=_env(sb, env or {}), cwd=str(sb.work),
                    stdin=subprocess.DEVNULL, capture_output=True, timeout=300,
                )
                what = f"{side} {' '.join(step.args)}"
                result = Result(
                    proc.returncode, _decode(proc.stdout, f"stdout of `{what}`"),
                    _decode(proc.stderr, f"stderr of `{what}`"), snapshot_tree(sb.work),
                )
                got.append(Outcome(result, porcelain(sb, brains)))
            outcomes[side] = got
        finally:
            sb.stash(side)
    return outcomes


# --- comparing ----------------------------------------------------------------


def ending(data: bytes | None) -> str:
    if data is None:
        return "lf"
    at = data.find(b"\n")
    return "crlf" if at > 0 and data[at - 1:at] == b"\r" else "lf"


def converged(items: list[dict]) -> list[bytes | None]:
    if not items:
        return []
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    out = subprocess.run(
        [str(python), "-I", str(DREAM_ORACLE)], input=json.dumps(items), capture_output=True, text=True,
        encoding="utf-8",
    )
    assert out.returncode == 0, out.stderr
    return [None if b is None else base64.b64decode(b) for b in json.loads(out.stdout)]


def is_pruned(rel: str) -> bool:
    parts = rel.split("/")[:-1]
    return any(p.startswith(".") or p in {"node_modules", "__pycache__"} for p in parts)


def compare_trees(v2: dict[str, bytes], v3: dict[str, bytes], before: dict[str, bytes],
                  *, pruned_ok: bool = False) -> list[str]:
    """Assert the trees are equal but for the decided differences; return the
    files that differ that way."""
    v2_only = sorted(set(v2) - set(v3))
    if pruned_ok:
        v2_only = [rel for rel in v2_only if not (is_pruned(rel) and rel.endswith("/map.md") and rel not in before)]
    assert not v2_only and not sorted(set(v3) - set(v2)), (
        f"file sets differ: only v2 {v2_only}, only v3 {sorted(set(v3) - set(v2))}"
    )
    differ = [rel for rel in v3 if v3[rel] != v2[rel]]
    if pruned_ok:
        # v2 regenerates the map.md files already in pruned directories; v3
        # leaves them as they were.
        kept = [rel for rel in differ if is_pruned(rel) and rel.endswith("/map.md") and v3[rel] == before.get(rel)]
        differ = [rel for rel in differ if rel not in kept]
    items = [{"name": rel, "v2": base64.b64encode(v2[rel]).decode(), "v3": base64.b64encode(v3[rel]).decode(),
              "ending": ending(before.get(rel))} for rel in differ]
    bad = []
    for rel, expected in zip(differ, converged(items)):
        got = v3[rel]
        if pruned_ok and rel.endswith("map.md"):
            expected = without_pruned_listing(v2[rel] if expected is None else expected)
        if got != expected:
            bad.append(f"{rel}:\n  v3 {got!r}\n  v2 {v2[rel]!r}\n  expected {expected!r}")
    assert not bad, "files differ beyond the decided differences:\n" + "\n".join(bad)
    return differ


def without_pruned_listing(data: bytes) -> bytes:
    """A v2 ``map.md`` with the pruned directories taken out of ``siblings``,
    ``sideways`` and Contents (PAR-036)."""
    text = data.decode("utf-8")
    eol = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(eol)
    pruned = re.compile(r"\.\./(\.[^/]+|node_modules|__pycache__)/map\.md")
    out = []
    for line in lines:
        if line.startswith("- ") and pruned.search(line) and not line.startswith("- ["):
            continue  # a siblings entry
        if line.startswith("- [") and re.match(r"- \[(\.[^/\]]+|node_modules|__pycache__)/\]", line):
            continue  # a sub-lobe in Contents
        if line.startswith("sideways "):
            links = [l for l in line[len("sideways "):].split(" | ") if not pruned.search(l)]
            if not links:
                continue
            line = "sideways " + " | ".join(links)
        out.append(line)
    out = [
        "siblings: []" if line == "siblings:" and not (i + 1 < len(out) and out[i + 1].startswith("- ")) else line
        for i, line in enumerate(out)
    ]
    if out != lines:
        # The body changed, so Kluris 3 rewrites the file: `updated` is today.
        out = [f"updated: '{today()}'" if line.startswith("updated: ") else line for line in out]
    return eol.join(out).encode("utf-8")


def today() -> str:
    # The harness runs with TZ=UTC; "today" is the date at the end of the run
    # (a run that crosses midnight UTC can fail here, as the allowlist notes).
    return datetime.datetime.now(datetime.timezone.utc).date().isoformat()


def dream_json(result: Result, root: Path, side: str) -> dict:
    doc = parse_exact(result.stdout, root, side)
    for key in ("deprecation", "broken_synapses_detail", "broken_synapses"):
        if isinstance(doc.get(key), list):
            doc[key] = sorted(doc[key], key=issue_key)
    return doc


def assert_step(sb: Sandbox, v2: Outcome, v3: Outcome, before: dict[str, bytes], *, json_out: bool = True,
                pruned_ok: bool = False) -> list[str]:
    r2, r3 = v2.result, v3.result
    assert r3.code == r2.code, f"exit v3 {r3.code} v2 {r2.code}\nv3: {r3.stdout}{r3.stderr}\nv2: {r2.stdout}{r2.stderr}"
    if json_out:
        assert "Traceback" not in r2.stderr, r2.stderr
        j2, j3 = dream_json(r2, sb.root, "v2"), dream_json(r3, sb.root, "v3")
        for key in ("deprecation", "broken_synapses_detail", "broken_synapses"):
            if isinstance(j2.get(key), list) and isinstance(j3.get(key), list):
                slash_paths(j2[key], j3[key])
                j2[key].sort(key=issue_key)
        assert j3 == j2, f"dream JSON differs\nv3 {json.dumps(j3)}\nv2 {json.dumps(j2)}"
        assert list(j3) == list(j2)
        assert r3.stderr == r2.stderr == ""
    differ = compare_trees(r2.tree, r3.tree, before, pruned_ok=pruned_ok)
    if not pruned_ok:
        assert v3.porcelain == v2.porcelain, f"git status differs\nv3 {v3.porcelain}\nv2 {v2.porcelain}"
    return differ


def text_words(text: str) -> tuple[str, list[str]]:
    """The text without its deprecation item lines, and those lines sorted
    (their order is the walk's, PAR-008)."""
    items = sorted(re.sub(r"\s+", " ", l).strip() for l in text.splitlines() if l.startswith("    - "))
    rest = "\n".join(l for l in text.splitlines() if not l.startswith("    - "))
    return rest, items


def assert_text(sb: Sandbox, v2: Result, v3: Result) -> None:
    rest2, items2 = text_words(v2.stdout)
    rest3, items3 = text_words(v3.stdout)
    same_words(rest2, rest3, sb.root)
    assert items3 == items2
    assert v3.stderr == v2.stderr == ""


DREAM = Step(["dream", "--json"])


def frozen(sb: Sandbox) -> dict[str, bytes]:
    return snapshot_tree(sb.root / "pristine")


# --- scenarios ----------------------------------------------------------------


def test_three_runs_converge_like_v2(sandbox):
    brain = rich_brain(sandbox)
    register(sandbox, [brain])
    before = frozen(sandbox)
    steps = [DREAM, Step(commit=D3), DREAM, DREAM, Step(["dream"])]
    out = run_steps(sandbox, ["brains/rich"], steps)
    v2, v3 = out["v2"], out["v3"]
    allowed = []
    for i, (a, b) in enumerate(zip(v2[:3], v3[:3])):
        allowed.append(assert_step(sandbox, a, b, before))
    assert_step(sandbox, v2[3], v3[3], before, json_out=False)
    assert_text(sandbox, v2[3].result, v3[3].result)

    first = json.loads(v2[0].result.stdout)
    assert v2[0].result.code == 1 and first["healthy"] is False
    assert first["fixes"]["parents_inferred"] >= 2 and first["fixes"]["reverse_synapses_added"] >= 8
    assert first["dates_updated"] >= 3 and first["deprecation_issues"] >= 3
    # Run 3 changes nothing in either.
    assert v3[2].result.tree == v3[1].result.tree and v2[2].result.tree == v2[1].result.tree
    tree = v3[0].result.tree
    # The decided differences show where expected: dates quoted on the
    # linker path, anchors as copies, CRLF kept, related in path order.
    assert "brains/rich/projects/no-parent.md" in allowed[0]
    assert b"created: '2026-01-01'" in tree["brains/rich/projects/no-parent.md"]
    assert b"&id001" not in tree["brains/rich/projects/agent.md"]
    assert b"\r\n" in tree["brains/rich/knowledge/crlf.md"]
    for side in (v2, v3):
        assert "brains/rich/uncommitted-lobe/map.md" in side[0].result.tree
        # map.md lists an upper-case suffix (case-insensitive there only).
        assert b"(./Shout.MD) \xe2\x80\x94 Shouted" in side[0].result.tree["brains/rich/projects/api/map.md"]


def test_dream_on_a_brain_v2_normalized_changes_nothing(sandbox):
    brain = rich_brain(sandbox)
    write_config(sandbox, [("rich", str(brain), "fixture")])
    for _ in range(2):
        # The second dream moves `updated` to the first one's commit date;
        # committing on the same date again leaves nothing to change.
        proc = subprocess.run([str(oracle_kluris()), "dream", "--json"], env=sandbox.env, capture_output=True)
        assert proc.returncode == 1, proc.stderr
        commit(sandbox, brain, "v2 dream", D3)
    sandbox.freeze()
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/rich"], [DREAM])
    for side in ("v2", "v3"):
        outcome = out[side][0]
        assert outcome.result.tree == before, f"{side} changed files"
        assert outcome.porcelain == {"brains/rich": ""}, f"{side}: {outcome.porcelain}"
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)


def test_healthy_brain_exits_zero(sandbox):
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/calm"], [DREAM, Step(["dream"])])
    assert out["v2"][0].result.code == 0
    assert json.loads(out["v2"][0].result.stdout)["healthy"] is True
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    assert_step(sandbox, out["v2"][1], out["v3"][1], before, json_out=False)
    assert_text(sandbox, out["v2"][1].result, out["v3"][1].result)
    assert "Brain is healthy." in out["v3"][1].result.stdout


def test_brain_without_git(sandbox):
    brain = healthy_brain(sandbox, git_repo=False)
    write(brain, {"projects/no-dates.md": "---\nparent: ./map.md\n---\n\n# No dates\n"})
    register(sandbox, [brain])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/calm"], [DREAM, DREAM])
    for a, b in zip(out["v2"], out["v3"]):
        assert_step(sandbox, a, b, before)
    doc = json.loads(out["v3"][0].result.stdout)
    assert doc["dates_updated"] == 0 and doc["frontmatter_issues"] == 2


def test_dates_near_midnight_in_the_author_time_zone(sandbox):
    brain = healthy_brain(sandbox)
    write(brain, {"projects/late.md": neuron("Late", updated="'2020-01-01'", created="'2020-01-01'")})
    commit(sandbox, brain, "late at night in Chicago", "2026-10-24T23:30:00-05:00")
    write(brain, {"projects/early.md": neuron("Early", updated="'2020-01-01'")})
    commit(sandbox, brain, "early morning in Bucharest", "2026-10-25T00:30:00+03:00")
    register(sandbox, [brain])
    before = frozen(sandbox)
    # Local "today" (map.md) in a zone 14 hours ahead; Windows' C runtime does
    # not read IANA names from TZ, so it keeps UTC there.
    env = {} if IS_WINDOWS else {"TZ": "Pacific/Kiritimati"}
    out = run_steps(sandbox, ["brains/calm"], [DREAM], env=env)
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    tree = out["v3"][0].result.tree
    assert b"updated: '2026-10-24'" in tree["brains/calm/projects/late.md"]
    assert b"updated: '2026-10-25'" in tree["brains/calm/projects/early.md"]


def test_dates_moved_by_later_commits(sandbox):
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    before = frozen(sandbox)
    steps = [DREAM, Step(commit=D2), DREAM, Step(commit=D3), DREAM]
    out = run_steps(sandbox, ["brains/calm"], steps)
    for a, b in zip(out["v2"], out["v3"]):
        assert_step(sandbox, a, b, before)


def test_write_only_when_the_body_changed(sandbox):
    brain = healthy_brain(sandbox)
    write_config(sandbox, [("calm", str(brain), "fixture")])
    proc = subprocess.run([str(oracle_kluris()), "dream", "--json"], env=sandbox.env, capture_output=True)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    stale = brain / "projects" / "map.md"
    text = stale.read_text(encoding="utf-8")
    text = re.sub(r"updated: '[0-9-]+'", "updated: '2020-01-01'", text)
    text = text.replace("parent: ../brain.md", "parent: ../brain.md\nextra: kept")
    stale.write_text(text, encoding="utf-8")
    (brain / "kluris.yml").write_text("name: calm\ndescription: A new description\n", encoding="utf-8")
    commit(sandbox, brain, "normalized", D3)
    sandbox.freeze()
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/calm"], [DREAM])
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    tree = out["v3"][0].result.tree
    assert tree["brains/calm/projects/map.md"] == before["brains/calm/projects/map.md"]
    assert tree["brains/calm/brain.md"] != before["brains/calm/brain.md"]
    assert b"A new description" in tree["brains/calm/brain.md"]


def test_broken_only(sandbox):
    brain = rich_brain(sandbox)
    register(sandbox, [brain])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/rich"], [Step(["dream", "--json", "--broken-only"]), Step(["dream", "--broken-only"])])
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    doc = json.loads(out["v3"][0].result.stdout)
    assert list(doc) == ["ok", "broken_synapses_count", "broken_synapses"] and doc["broken_synapses_count"] == 2
    a, b = out["v2"][1].result, out["v3"][1].result
    assert (a.code, a.stdout, a.stderr) == (b.code, b.stdout, b.stderr) == (1, "", "Error: --broken-only requires --json\n")


def test_brain_all(sandbox):
    rich = rich_brain(sandbox)
    calm = healthy_brain(sandbox)
    register(sandbox, [rich, calm])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/rich", "brains/calm"], [Step(["dream", "--json", "--brain", "all"]),
                                                              Step(["dream", "--brain", "all"])])
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    assert_step(sandbox, out["v2"][1], out["v3"][1], before, json_out=False)
    assert_text(sandbox, out["v2"][1].result, out["v3"][1].result)
    doc = json.loads(out["v3"][0].result.stdout)
    assert {d["brain"] for d in doc["deprecation"]} == {"rich"}


def test_one_brain_of_two_by_name(sandbox):
    rich = rich_brain(sandbox)
    calm = healthy_brain(sandbox)
    register(sandbox, [rich, calm])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/rich", "brains/calm"], [Step(["dream", "--json", "--brain", "calm"])])
    assert_step(sandbox, out["v2"][0], out["v3"][0], before)
    assert out["v3"][0].result.tree["brains/rich/projects/alpha.md"] == before["brains/rich/projects/alpha.md"]


def test_pruned_directories(sandbox):
    """PAR-036: v3 writes no map.md into dot directories, ``node_modules`` or
    ``__pycache__`` and lists none of them as a sub-lobe or sibling."""
    brain = healthy_brain(sandbox)
    write(brain, {
        ".archive/map.md": "---\nauto_generated: true\nparent: ../brain.md\nsiblings: []\nupdated: '2026-01-01'\n---\n\n# .Archive\n\nup [brain.md](../brain.md)\n\n## Contents\n\n(empty)\n",
        ".archive/old.md": neuron("Old archived"),
        ".github/workflows/ci.yml": "on: push\n",
        "node_modules/pkg/index.md": "# pkg\n",
        "projects/.hidden/map.md": "---\nauto_generated: true\n---\n\n# Hidden\n",
        "projects/__pycache__/x.md": "# cache\n",
    })
    commit(sandbox, brain, "pruned dirs", D2)
    register(sandbox, [brain])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/calm"], [DREAM, DREAM])
    for a, b in zip(out["v2"], out["v3"]):
        assert_step(sandbox, a, b, before, pruned_ok=True)
    v2_tree, v3_tree = out["v2"][0].result.tree, out["v3"][0].result.tree
    assert "brains/calm/.github/workflows/map.md" in v2_tree
    assert not [rel for rel in v3_tree if is_pruned(rel) and rel.endswith("map.md") and rel not in before]
    assert b".archive" in v2_tree["brains/calm/projects/map.md"]
    assert b".archive" not in v3_tree["brains/calm/projects/map.md"]
    assert b".hidden" not in v3_tree["brains/calm/projects/map.md"]


def test_windows_paths_in_json_use_forward_slashes(sandbox):
    """PAR-037: ``file``/``source``/``target`` are ``/``-joined on every OS;
    v2 writes native paths, so on Windows they differ."""
    brain = rich_brain(sandbox)
    write_config(sandbox, [("rich", str(brain), "fixture")])
    proc = subprocess.run([*_exe("v3"), "dream", "--json"], env=sandbox.env, capture_output=True)
    doc = json.loads(proc.stdout)
    paths = [d.get(k) for d in [*doc["deprecation"], *doc["broken_synapses_detail"]] for k in ("file", "source", "target")]
    assert any(p and "/" in p for p in paths)
    assert not [p for p in paths if p and "\\" in p]
    if IS_WINDOWS:
        v2 = json.loads(subprocess.run([str(oracle_kluris()), "dream", "--json"], env=sandbox.env,
                                       capture_output=True).stdout)
        assert any("\\" in (d.get("file") or "") for d in v2["broken_synapses_detail"])


def test_dream_help_wording(sandbox):
    write_config(sandbox, [])
    sandbox.freeze()
    out = run_steps(sandbox, [], [Step(["dream", "--help"])])
    v2, v3 = out["v2"][0].result, out["v3"][0].result
    assert v2.code == v3.code == 0
    assert "Usage: kluris dream [OPTIONS]" in v3.stdout
    for text in ("Brain maintenance — regenerate maps, update dates, auto-fix safe issues,", "--brain", "Specific brain",
                 "--json", "JSON output", "--broken-only", "In --json mode, return only broken-synapse entries"):
        assert text in v2.stdout, f"v2 no longer says {text!r}"
        assert text in v3.stdout, text
    same_words(v2.stdout.replace("TEXT", "<BRAIN>").replace("Usage: kluris dream [OPTIONS]", ""),
               v3.stdout.replace("Usage: kluris dream [OPTIONS]", ""), sandbox.root)


# --- resilience: where v2 crashes or damages a file (TEST-K3-21) --------------------


def resilience_brain(sb: Sandbox, files: dict[str, str | bytes]) -> Path:
    """The healthy brain plus ``files``, all committed."""
    brain = healthy_brain(sb)
    write(brain, files)
    commit(sb, brain, "resilience case", D2)
    return brain


def both(sb: Sandbox, args: list[str] | None = None) -> tuple[Outcome, Outcome]:
    out = run_steps(sb, ["brains/calm"], [Step(args or ["dream", "--json"])])
    return out["v2"][0], out["v3"][0]


def assert_v2_traceback(outcome: Outcome, error: str) -> None:
    r = outcome.result
    assert r.code == 1 and r.stdout == "", (r.code, r.stdout)
    assert "Traceback" in r.stderr and error in r.stderr.strip().splitlines()[-1], r.stderr


def lf(data: bytes) -> bytes:
    """v2's bytes with CRLF read as LF: on Windows v2 writes markdown in text
    mode (PAR-035)."""
    return data.replace(b"\r\n", b"\n")


NO_PARENT = "---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n# Needs a parent\n"


@pytest.mark.parametrize("bad,error", [
    ("---\ntitle: Foo: bar\n---\n\n# Invalid\n", "yaml"),
    ("---\n1: x\n---\n\n# Key\n", "TypeError: keywords must be strings"),
    (b"---\ncreated: '2026-01-01'\n---\n\n# Caf\xe9\n", "UnicodeDecodeError"),
], ids=["invalid-yaml", "non-string-key", "not-utf8"])
def test_malformed_neuron_v2_crashes_v3_skips_and_reports(sandbox, bad, error):
    """PAR-038: v2 ends in a traceback (no JSON); v3 leaves the file alone,
    reports it in ``skipped`` and processes the rest, exit 1."""
    resilience_brain(sandbox, {"projects/bad.md": bad, "projects/fix-me.md": NO_PARENT})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    before = frozen(sandbox)
    v2, v3 = both(sandbox)
    if error == "yaml":
        error = "line 2, column 11"
    assert_v2_traceback(v2, error)
    doc = parse_exact(v3.result.stdout, sandbox.root, "v3")
    assert v3.result.code == 1 and doc["healthy"] is False
    assert list(doc)[-1] == "skipped" and [s["file"] for s in doc["skipped"]] == ["projects/bad.md"]
    assert list(doc["skipped"][0]) == ["brain", "file", "error"]
    tree = v3.result.tree
    assert tree["brains/calm/projects/bad.md"] == before["brains/calm/projects/bad.md"]
    assert b"parent: ./map.md" in tree["brains/calm/projects/fix-me.md"]


def test_yaml_neuron_whose_block_does_not_parse(sandbox):
    """PAR-038: v2 writes a second ``#---`` block above the one it cannot
    read; v3 leaves the file alone and reports it."""
    bad = "#---\n# title: Foo: bar\n#---\nk: 1\n"
    resilience_brain(sandbox, {"projects/bad.yml": bad})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    assert v2.result.code == 0
    assert v2.result.tree["brains/calm/projects/bad.yml"].count(b"#---") == 4
    assert v3.result.tree["brains/calm/projects/bad.yml"] == bad.encode()
    doc = json.loads(v3.result.stdout)
    assert v3.result.code == 1 and doc["skipped"][0]["file"] == "projects/bad.yml"
    assert doc["frontmatter_issues"] == 0


def test_bom_file_keeps_one_block(sandbox):
    """PAR-038 (with PAR-010): v2 does not see frontmatter behind a BOM and
    prepends a second block; v3 keeps the BOM and writes one."""
    resilience_brain(sandbox, {"projects/bom.md": "\ufeff" + NO_PARENT})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    assert v2.result.code == v3.result.code == 0
    assert lf(v2.result.tree["brains/calm/projects/bom.md"]).count(b"---\n") == 4
    assert v3.result.tree["brains/calm/projects/bom.md"] == (
        "\ufeff---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# Needs a parent\n"
    ).encode()


def test_first_body_line_keeps_its_indentation(sandbox):
    """PAR-002: v2 strips the first body line's indentation."""
    body = "    let x = 1;\n    let y = 2;\n\nAfter.\n"
    resilience_brain(sandbox, {"projects/code.md": "---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n" + body})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    assert lf(v2.result.tree["brains/calm/projects/code.md"]).endswith(b"\n\nlet x = 1;\n    let y = 2;\n\nAfter.\n")
    assert v3.result.tree["brains/calm/projects/code.md"].endswith(("\n\n" + body).encode())


def test_crlf_file_stays_crlf(sandbox):
    """PAR-035: v2 writes a CRLF neuron back LF (macOS/Linux; CRLF everywhere
    on Windows); v3 keeps CRLF, and new files are LF."""
    resilience_brain(sandbox, {"projects/crlf.md": NO_PARENT.replace("\n", "\r\n").encode()})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    got = v3.result.tree["brains/calm/projects/crlf.md"]
    assert b"parent: ./map.md\r\n" in got and got.count(b"\n") == got.count(b"\r\n")
    if not IS_WINDOWS:
        assert b"\r" not in v2.result.tree["brains/calm/projects/crlf.md"]
    assert b"\r" not in v3.result.tree["brains/calm/brain.md"]


def test_non_string_related_entry(sandbox):
    """PAR-047: v2's one-way check raises ``TypeError``; v3 reports a
    ``related`` type issue."""
    resilience_brain(sandbox, {"projects/odd.md": neuron("Odd", extra="related:\n- 1\n- ./alpha.md\n")})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    assert_v2_traceback(v2, "TypeError: unsupported operand type(s) for /")
    doc = json.loads(v3.result.stdout)
    assert v3.result.code == 1 and doc["frontmatter_issues"] == 1 and doc["one_way_synapses"] == 0


def test_related_outside_the_brain(sandbox):
    """PAR-047: v2's one-way check raises ``ValueError`` on a target outside
    the brain; v3 counts the links as broken, not one-way."""
    resilience_brain(sandbox, {"projects/out.md": neuron("Out", extra="related:\n- ../../../outside.md\n- /etc/hosts.md\n")})
    write(sandbox.work, {"outside.md": neuron("Outside")})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    v2, v3 = both(sandbox)
    assert_v2_traceback(v2, "ValueError")
    doc = json.loads(v3.result.stdout)
    assert [b["target"] for b in doc["broken_synapses_detail"]] == ["../../../outside.md", "/etc/hosts.md"]
    assert doc["one_way_synapses"] == 0


def test_missing_kluris_yml(sandbox):
    """PAR-048: v2 ends in a ``FileNotFoundError`` traceback; v3 prints a
    clean error naming the file and writes nothing."""
    brain = healthy_brain(sandbox)
    (brain / "kluris.yml").unlink()
    register(sandbox, [brain])
    before = frozen(sandbox)
    for args in (["dream"], ["dream", "--json"]):
        v2, v3 = both(sandbox, args)
        assert_v2_traceback(v2, "FileNotFoundError")
        assert v3.result.code == 1 and v3.result.tree == before
        message = json.loads(v3.result.stdout)["error"] if "--json" in args else v3.result.stderr
        expected = f"Brain calm has no kluris.yml: {brain / 'kluris.yml'}"
        assert message == (expected if "--json" in args else f"Error: {expected}\n"), message
        clear_runs(sandbox)


def test_link_checking(sandbox):
    """PAR-049: v2 reports links in code, ``mailto:`` and other schemes,
    titled links and ``%20`` paths as broken; v3 reports only the links whose
    file is missing (a missing image too, as v2)."""
    body = ("# Links\n\n```\n[fenced](./nope1.md)\n```\n\n`[inline](./nope2.md)`\n\n"
            "[mail](mailto:a@b.c) [ftp](ftp://x/y)\n\n[titled](./alpha.md \"Alpha\")\n\n"
            "[encoded](./my%20note.md)\n\n![img](./pic.png) ![gone](./nopic.png)\n\n[missing](./missing.md)\n")
    resilience_brain(sandbox, {"projects/links.md": neuron("Links", body=body), "projects/my note.md": neuron("Note"),
                               "projects/pic.png": "png"})
    register(sandbox, [sandbox.work / "brains" / "calm"])
    before = frozen(sandbox)
    v2, v3 = both(sandbox)
    targets = lambda o: [b["target"] for b in json.loads(o.result.stdout)["broken_synapses_detail"]]  # noqa: E731
    assert targets(v2) == ["./nope1.md", "./nope2.md", "mailto:a@b.c", "ftp://x/y", './alpha.md "Alpha"',
                           "./my%20note.md", "./nopic.png", "./missing.md"]
    assert targets(v3) == ["./nopic.png", "./missing.md"]
    j2, j3 = json.loads(v2.result.stdout), json.loads(v3.result.stdout)
    for doc in (j2, j3):
        del doc["broken_synapses"], doc["broken_synapses_detail"]
    assert j3 == j2
    compare_trees(v2.result.tree, v3.result.tree, before)


# --- the brains registered on this machine -----------------------------------------


def _registered_brains() -> list[tuple[str, Path, str]]:
    kluris = shutil.which("kluris")
    if not kluris:
        return []
    out = subprocess.run([kluris, "list", "--json"], capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        return []
    try:
        brains = json.loads(out.stdout).get("brains", [])
    except json.JSONDecodeError:
        return []
    return [(b["name"], Path(b["path"]), b.get("description", "")) for b in brains if Path(b["path"]).is_dir()]


def changed(before: dict[str, bytes], after: dict[str, bytes]) -> list[str]:
    return sorted(rel for rel in set(before) | set(after) if before.get(rel) != after.get(rel))


def test_registered_brains(tmp_path, oracle):
    """Copies (with ``.git``, so dates are real) of every registered brain:
    a brain v2 already normalized changes in neither; on the others v3's
    changes equal v2's but for the decided differences. Prints, per brain, how
    many files each changed and each file whose bytes differ between them."""
    brains = _registered_brains()
    if not brains:
        pytest.skip("no brains registered with an installed kluris on this machine")
    failures, report = [], []
    for name, source, description in brains:
        sb = Sandbox(tmp_path / name)
        copy = sb.work / "brains" / name
        shutil.copytree(source, copy, symlinks=True)
        write_config(sb, [(name, str(copy), description)])
        sb.freeze()
        before = frozen(sb)
        rel = f"brains/{name}"
        out = run_steps(sb, [rel], [DREAM])
        v2, v3 = out["v2"][0], out["v3"][0]
        n2, n3 = changed(before, v2.result.tree), changed(before, v3.result.tree)
        try:
            differ = assert_step(sb, v2, v3, before, pruned_ok=True)
        except AssertionError as exc:
            failures.append(f"{name}: {exc}")
            differ = changed(v2.result.tree, v3.result.tree)
        report.append(f"{name}: v2 changed {len(n2)} files, v3 changed {len(n3)}; "
                      f"{len(differ)} differ between them: {differ}")
    print("\n" + "\n".join(report))
    assert not failures, "\n".join(failures)
