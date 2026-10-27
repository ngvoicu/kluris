"""Skill install parity: a HOME written by v2.28.11 itself (`create`,
`companion add`, `doctor`) migrated by Kluris 3's skill install alone
(`cargo xtask skills-install`, the install create, register, remove,
companion add/remove and doctor call).

Expected (parity/allowlist.md, the skill-install entries):

- every v3 copy is v2's SKILL.md with exactly the decided changes (the
  `{brain, file, target}` braces, the marker line, the "Search" paragraph),
  and all copies of one brain are byte-identical;
- written: `~/.agents/skills`, `~/.claude/skills`, `~/.hermes/skills` and each
  profile's, and `~/.gemini/config/skills` (v2's default `commands_for` reads
  as v3's, which adds Antigravity);
- removed: v2's copies in `~/.cursor`, `~/.codex`, `~/.copilot`, `~/.gemini`,
  `~/.kilo`, `~/.junie`, `~/.codeium/windsurf` skills and the Windsurf
  workflows; their folders stay;
- nothing else changes: the user's own `kluris-notes` skills and workflow,
  the config, the companions and the brains.
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import IS_WINDOWS, PARITY_DIR, Sandbox, oracle_kluris, run_v2
from test_yaml_differential import LineProcess

WORKSPACE = PARITY_DIR.parent
PATH_EXAMPLES = int(os.environ.get("KLURIS_PATH_FUZZ_EXAMPLES", "10000"))

MARKER = "<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->"

V2_SEARCH_ADVICE = """\
Start with the cached wake-up snapshot. For targeted lookup, prefer
`kluris search "<query>"{flag} --json`; it ranks neurons,
glossary terms, and brain.md in one pass. Use `--lobe <name>`, `--tag <tag>`,
or `--limit 20` when useful. If search finds nothing, navigate manually:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files.

If nothing is documented, say so plainly: "Nothing documented about X yet."
If a result is deprecated, prefer its replacement and mention that the old
knowledge was superseded.
"""

V3_SEARCH_ADVICE = """\
Start with the cached wake-up snapshot. For targeted lookup, search the brain.

### Search

Run `kluris search "<2-5 distinctive words>"{flag} --json`. Use keywords,
not a sentence. For a question that could be worded several ways, run 2-3
searches in parallel with different wording: synonyms, the team's own terms
from the wake-up glossary, and the language the brain is written in. Search
ranks neurons, glossary terms, and brain.md in one pass; use `--lobe <name>`,
`--tag <tag>`, or `--limit 20` when useful.

Read `title`, `heading` and `snippet` first, then open only the best files,
starting at `line`. Treat a top hit with low `coverage` (under about 0.5) and
no `title` or `heading` match as weak. Follow a hit's `related` neurons when
the answer spans topics. If a hit is `deprecated`, read its `replaced_by`
instead and mention that the old knowledge was superseded.

If every search is weak, navigate manually before answering:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files. If
nothing is documented, say so plainly: "Nothing documented about X yet."
Do not stretch a loose match into an answer.
"""

# v3.1's write flow (PAR-078): reads only fetch and `kluris sync` brings
# teammates' changes in, sync before a write, `kluris commit` + `kluris sync`
# after it instead of v2's `git add -A` reminder, and sync/commit/resolve
# among the CLI commands.
V2_REFRESH = """\
this skill's remember/learn intents, `kluris dream{flag}`,
or direct file edits the user tells you about.

If the `kluris` CLI is unavailable"""

V3_REFRESH = """\
this skill's remember/learn intents, `kluris dream{flag}`,
a `kluris sync{flag}` that reports `merged`,
or direct file edits the user tells you about.

`kluris wake-up` and `kluris search` read the brain as it is on disk; they
fetch the teammates' pushes (at most once a minute) without changing any
file. To bring the teammates' changes in, run `kluris sync{flag} --json`.
The write flow below does it before every write; run it also when the user
asks for the team's latest. `KLURIS_NO_AUTO_SYNC=1` turns the fetch off.

If the `kluris` CLI is unavailable"""

V2_WRITING_RULES = "## Writing rules\n\n### Approval protocol\n"

V3_WRITING_RULES = """\
## Writing rules

### Sync before you write

Before you write, change, or delete any brain file, run
`kluris sync{flag} --json` and read the brain's entry in `brains[]`:
- `conflicts[]` is not empty: this brain and a teammate's push changed the
  same thing (or a conflict is already open: then `refused` says so too).
  STOP and show the human each conflict: its `path`, its `kind` (`content`,
  `delete_edit`, `parent`, `frontmatter`, `structure`, or `ambiguous_base`),
  and what each side changed. The conflict's `base`, `ours`, and `theirs` are blob ids: read
  them with `git -C {path} show <id>`, or compare with
  `git -C {path} diff <base> <ours>` and `git -C {path} diff <base> <theirs>`
  (a `null` side deleted the file). Ask which version to keep, or propose a
  merged text and apply the approval protocol to it. Then settle each path
  with `kluris resolve{flag} <path> --ours`, `--theirs`, or
  `--file <merged file>`; the last one commits the merge and pushes it. A
  `structure` conflict (a file on one side where the other has a directory,
  or a rename) is settled with `--ours` or `--theirs`, which keep that side's
  whole structure.
  Brain files never contain conflict markers, so never edit a file to
  resolve a conflict. If `kluris resolve` says the conflict is stale, run
  `kluris resolve{flag} --abort` and sync again.
- `blocked[]` is not empty: those files have uncommitted changes that the
  incoming changes would overwrite. Ask the human to commit or stash them,
  then sync again.
- `refused` is set with no `conflicts[]`, or `offline` is true: tell the
  human, then go on with the local brain.

### Approval protocol
"""

V2_COMMIT_REMINDER = """\
   on. Then remind them to commit with git (`cd {path} && git add -A && git commit`)
   if the brain uses git.
"""

V3_COMMIT_STEP = """\
   on.
7. **Commit and share what you wrote.** Once the human approved and the
   files are written, commit exactly those files with
   `kluris commit{flag} --paths <every brain-relative path you wrote or deleted, comma-separated> --json`,
   with `KLURIS_AGENT` set to your agent's name (for example
   `KLURIS_AGENT="Claude Code"`) so the commit credits you. It commits those
   files and the `map.md`/`brain.md` files they change, under the human's git
   identity, and nothing else. If its `left_dirty` lists files that
   `kluris dream` changed because of this write (a reverse `related:` link,
   a refreshed date), show them to the human and, once they agree, commit
   them the same way; leave every other uncommitted file alone. Then run
   `kluris sync{flag} --json` and report
   the `commit` and whether it was `pushed`. If sync reports `conflicts[]` or
   `blocked[]`, handle them as in "Sync before you write". If a git hook
   stops the commit, show its output to the human and fix what it reports;
   never pass `--no-verify` unless the human asks for it. A brain inside a
   larger git repository (sync refuses it): commit with `kluris commit`, then
   push with git the way the team normally does. A brain without git: skip
   this step.
"""

V2_CLI_TAIL = """\
- `kluris mri{flag}` -- generate interactive visualization

Sync, commit, and branch operations go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} push / pull / status /
checkout` as you would with any other repo.
"""

V3_CLI_TAIL = """\
- `kluris mri{flag}` -- generate interactive visualization
- `kluris sync{flag} --json` -- fetch, merge, and push the brain; reports `conflicts[]`, `blocked[]`, `refused`, and `offline`
- `kluris commit{flag} --paths <a.md,b.md> --json` -- commit exactly those files and the maps they change
- `kluris resolve{flag} <path> --ours|--theirs|--file <merged file> --json` -- settle a sync conflict path by path (`--abort` drops it)

Branch operations and anything else go through `git` directly -- kluris
brains are plain git repos. Use `git -C {path} status / log / checkout`
as you would with any other repo.
"""

FOREIGN_SKILL = "---\nname: kluris-notes\ndescription: My own notes about kluris\n---\n\n# Notes\n"

# Folders v2 wrote and v3 clears, relative to HOME.
V2_ONLY = [
    ".cursor/skills",
    ".codex/skills",
    ".copilot/skills",
    ".gemini/skills",
    ".kilo/skills",
    ".junie/skills",
    ".codeium/windsurf/skills",
]
# Folders v3 writes (v2 wrote them too, except the Antigravity one).
V3_WRITES = [
    ".agents/skills",
    ".claude/skills",
    ".hermes/skills",
    ".hermes/profiles/work/skills",
    ".gemini/config/skills",
]
BRAINS = ["acme", "beta"]


def _replace_once(text: str, old: str, new: str) -> str:
    assert text.count(old) == 1, old[:60]
    return text.replace(old, new)


def _v2_to_v3(v2: str, brain: str) -> str:
    flag = f" --brain {brain}"
    out = _replace_once(v2, "a list of `{{brain, file, target}}` entries.", "a list of `{brain, file, target}` entries.")
    out = _replace_once(out, "\n---\n\n# Brain: ", f"\n---\n\n{MARKER}\n\n# Brain: ")
    out = _replace_once(out, V2_SEARCH_ADVICE.format(flag=flag), V3_SEARCH_ADVICE.format(flag=flag))
    start = out.index("The brain lives at `") + len("The brain lives at `")
    path = out[start:out.index("`", start)]

    def fill(text: str) -> str:
        return text.replace("{flag}", flag).replace("{path}", path)

    for old, new in [(V2_REFRESH, V3_REFRESH), (V2_WRITING_RULES, V3_WRITING_RULES),
                     (V2_COMMIT_REMINDER, V3_COMMIT_STEP), (V2_CLI_TAIL, V3_CLI_TAIL)]:
        out = _replace_once(out, fill(old), fill(new))
    return out


def _oracle_ok(sandbox: Sandbox, args: list[str]) -> dict:
    r = run_v2(args, sandbox.env)
    assert r.code == 0, f"oracle {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout)


def _tree(root: Path) -> dict[str, bytes]:
    out = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and ".git" not in path.relative_to(root).parts:
            out[path.relative_to(root).as_posix()] = path.read_bytes()
    return out


def _v2_home(sandbox: Sandbox) -> Path:
    brains = sandbox.work / "brains"
    brains.mkdir()
    _oracle_ok(sandbox, ["create", "acme", "--path", str(brains), "--description", 'Acme "platform" — ünïcode', "--json"])
    _oracle_ok(sandbox, ["create", "beta", "--path", str(brains), "--no-git", "--json"])
    _oracle_ok(sandbox, ["companion", "add", "specmint-core", "--brain", "acme", "--json"])
    (sandbox.home / ".hermes" / "profiles" / "work").mkdir(parents=True)
    _oracle_ok(sandbox, ["doctor", "--json"])
    # The user's own things, added after v2 (whose sweep deletes any kluris*).
    for rel in [".agents/skills", ".claude/skills", ".cursor/skills", ".codex/skills"]:
        path = sandbox.home / rel / "kluris-notes" / "SKILL.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(FOREIGN_SKILL, encoding="utf-8")
    (sandbox.home / ".codeium/windsurf/global_workflows/kluris-notes.md").write_text("# mine\n", encoding="utf-8")
    return brains


def _v3_install(sandbox: Sandbox, env: dict[str, str]) -> dict:
    run_env = dict(os.environ)
    for key in ("COPILOT_HOME",):
        run_env.pop(key, None)
    run_env.update({k: v for k, v in env.items() if k in ("KLURIS_CONFIG", "COPILOT_HOME")})
    proc = subprocess.run(
        ["cargo", "--quiet", "xtask", "skills-install", "--home", str(sandbox.home)],
        cwd=WORKSPACE, capture_output=True, text=True, env=run_env,
    )
    assert proc.returncode == 0, proc.stderr
    return json.loads(proc.stdout)


@pytest.fixture
def sandbox(tmp_path):
    return Sandbox(tmp_path)


def test_v2_home_is_migrated(sandbox: Sandbox):
    brains = _v2_home(sandbox)
    before = _tree(sandbox.home)
    brains_before = _tree(brains)
    # The v2 HOME looks as research-03 §2.9 says (guards the fixture).
    for rel in V2_ONLY + [".agents/skills", ".claude/skills", ".hermes/skills", ".hermes/profiles/work/skills"]:
        for brain in BRAINS:
            assert f"{rel}/kluris-{brain}/SKILL.md" in before, rel
    for brain in BRAINS:
        assert f".codeium/windsurf/global_workflows/kluris-{brain}.md" in before

    report = _v3_install(sandbox, sandbox.env)
    assert report["ok"], report
    after = _tree(sandbox.home)

    expected = dict(before)
    for brain in BRAINS:
        # v2 writes in text mode (CRLF on Windows); v3 writes LF (PAR entry).
        v2 = before[f".claude/skills/kluris-{brain}/SKILL.md"].decode("utf-8").replace("\r\n", "\n")
        v3 = _v2_to_v3(v2, brain).encode("utf-8")
        for rel in V3_WRITES:
            expected[f"{rel}/kluris-{brain}/SKILL.md"] = v3
        for rel in V2_ONLY:
            del expected[f"{rel}/kluris-{brain}/SKILL.md"]
        del expected[f".codeium/windsurf/global_workflows/kluris-{brain}.md"]
    changed = sorted(k for k in set(expected) | set(after) if expected.get(k) != after.get(k))
    assert not changed, changed
    assert _tree(brains) == brains_before
    for rel in V2_ONLY + [".codeium/windsurf/global_workflows"]:
        assert (sandbox.home / rel).is_dir(), rel

    roles = [(d["path"], d["role"]) for d in report["destinations"]]
    assert roles[0] == (".agents/skills", "universal")
    assert (".gemini/config/skills", "agent:antigravity") in roles
    assert (".copilot/skills", "migration") in roles


def test_copilot_home_keeps_the_copilot_folder(sandbox: Sandbox):
    _v2_home(sandbox)
    before = _tree(sandbox.home)
    env = dict(sandbox.env, COPILOT_HOME=str(sandbox.home / "copilot-home"))
    report = _v3_install(sandbox, env)
    assert report["ok"], report
    after = _tree(sandbox.home)
    for brain in BRAINS:
        rel = f".copilot/skills/kluris-{brain}/SKILL.md"
        assert after[rel] == before[rel]
        assert f"copilot-home/skills/kluris-{brain}/SKILL.md" in after


# --- the POSIX path rule ------------------------------------------------------
# SKILL.md writes the brain and companion paths through v2's `_posix_path`
# (Python 3.11's PureWindowsPath / PurePosixPath). Random paths built from
# separators, drive letters, `.`/`..`, the `\\?\` and UNC prefixes and
# non-ASCII text, answered by the oracle and by `cargo xtask posix-path`.
# Cases: ``KLURIS_PATH_FUZZ_EXAMPLES`` (default 10000).


@pytest.fixture(scope="module")
def path_oracle():
    venv = oracle_kluris().parent
    python = venv / ("python.exe" if IS_WINDOWS else "python")
    proc = LineProcess([str(python), "-I", str(PARITY_DIR / "tools" / "posix_path_oracle.py")], WORKSPACE)
    yield proc
    proc.close()


@pytest.fixture(scope="module")
def path_rust():
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE, capture_output=True, text=True
    )
    assert build.returncode == 0, build.stderr
    proc = LineProcess(["cargo", "--quiet", "xtask", "posix-path"], WORKSPACE)
    yield proc
    proc.close()


path_pieces = st.sampled_from(
    ["/", "\\", "//", "\\\\", ":", "C:", "c:", "1:", "?", "\\\\?\\", "UNC", "\\\\?\\UNC\\",
     ".", "..", "a", "brain", "Users", " ", "é", "日本", "-", "~", "server", "share"]
)
paths = st.lists(path_pieces, max_size=10).map("".join)


@settings(max_examples=PATH_EXAMPLES, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(path=paths)
def test_posix_path_matches_v2(path, path_oracle, path_rust):
    path_oracle.send(path)
    path_rust.send(path)
    expected, got = path_oracle.receive(), path_rust.receive()
    assert got == expected, f"{path!r}: v2 {expected!r}, v3 {got!r}"
