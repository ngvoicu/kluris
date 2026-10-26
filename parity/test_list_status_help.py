"""``list``, ``status`` and ``help`` against the v2.28.11 oracle
(research-03 §1.3, §1.4, §1.14, §1.15).

JSON is compared exactly, apart from the allowlisted git fixes (PAR-012 ...
PAR-015) and ``help`` (PAR-017, PAR-018); human output by v2's wording
(``same_words``) or by substring, never by layout. Brains are real git repos
built with the sandbox's pinned identity and dates.
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

import pytest

from harness import Sandbox, assert_json_equal, normalize
from runs import parse_exact, run_pair, same_words, write_config

DATE = "2026-01-01T12:00:00+00:00"

# v2's help table (cli.py:2010-2024), in order; v3 drops `pack`.
V2_COMMANDS = [
    "create", "register", "list", "status", "search", "wake-up", "companion",
    "dream", "pack", "mri", "remove", "doctor", "help",
]
# v2's commands Kluris 3 keeps, in v2's order.
V2_KEPT = [c for c in V2_COMMANDS if c != "pack"]
# Commands new in v3, after v2's (PAR-064: no v2 counterpart).
V3_ONLY = [
    # PAR-075: v3.1's git auto-sync.
    ("commit", "Commit approved brain files and the maps they change (--paths)"),
    # PAR-074: v3.1's git auto-sync.
    ("sync", "Fetch, merge and push brains; conflicts go to a person (--brain all)"),
    ("self-update", "Update kluris to the newest release (--check only looks)"),
    ("self-uninstall", "Remove the installed kluris (--purge: also ~/.kluris and Kluris skills)"),
]
V3_COMMANDS = V2_KEPT + [name for name, _ in V3_ONLY]


def v3_rows(v2_commands: list[dict]) -> list[dict]:
    """v2's ``help --json`` rows as v3 lists them: without ``pack``, then the
    v3-only commands."""
    kept = [c for c in v2_commands if c["name"] != "pack"]
    return kept + [{"name": name, "description": desc} for name, desc in V3_ONLY]


# --- building brains --------------------------------------------------------------


def git(sb: Sandbox, cwd: Path, *args: str, date: str = DATE) -> str:
    env = dict(sb.env, GIT_AUTHOR_DATE=date, GIT_COMMITTER_DATE=date)
    out = subprocess.run(["git", *args], cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8")
    assert out.returncode == 0, f"git {args}: {out.stderr}"
    return out.stdout


def write(root: Path, files: dict[str, str]) -> None:
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode("utf-8"))


def neuron(title: str) -> str:
    return f"---\nparent: map.md\ncreated: 2026-01-01\nupdated: 2026-01-01\n---\n# {title}\n"


def init(sb: Sandbox, path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    git(sb, path, "init", "-q")
    git(sb, path, "config", "core.autocrlf", "false")
    return path


def commit(sb: Sandbox, repo: Path, message: str, files: dict[str, str] | None = None, date: str = DATE) -> None:
    write(repo, files or {})
    git(sb, repo, "add", "-A")
    git(sb, repo, "commit", "-q", "--allow-empty", "-m", message, date=date)


def brain_files() -> dict[str, str]:
    """Two lobes (one nested), root files, and directories status never counts."""
    return {
        "brain.md": "# demo\n",
        "glossary.md": "# Glossary\n",
        "projects/map.md": "# Projects\n",
        "projects/alpha.md": neuron("Alpha"),
        "projects/sub/beta.md": neuron("Beta"),
        "decisions/map.md": "# Decisions\n",
        "decisions/d1.md": neuron("D1"),
        ".obsidian/notes.md": neuron("hidden"),
        "node_modules/pkg/readme.md": neuron("vendored"),
        "root-note.md": neuron("Root"),
    }


def register(sb: Sandbox, brains: list[tuple[str, Path]], description: str = "demo brain") -> None:
    write_config(sb, [(name, str(path), description) for name, path in brains])


def run(sb: Sandbox, *args: str):
    return run_pair(sb, list(args), list(args))


def json_pair(sb: Sandbox, *args: str) -> tuple[dict, dict]:
    r = run(sb, *args)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 0, f"v2: {v2.stdout} {v2.stderr}"
    assert v3.code == 0, f"v3: {v3.stdout} {v3.stderr}"
    return parse_exact(v2.stdout, sb.root, "v2"), parse_exact(v3.stdout, sb.root, "v3")


def only_brain(doc: dict) -> dict:
    assert doc["ok"] is True and len(doc["brains"]) == 1, doc
    return doc["brains"][0]


# --- status: where v2 is right, byte for byte --------------------------------------


@pytest.fixture
def history_brain(sandbox: Sandbox) -> Path:
    """A git brain with 12 commits and a clean tree."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", brain_files())
    for i in range(1, 12):
        commit(sandbox, brain, f"update {i}", {"projects/alpha.md": neuron(f"Alpha {i}")},
               date=f"2026-02-{i:02d}T08:30:00+02:00")
    register(sandbox, [("demo", brain)])
    sandbox.freeze()
    return brain


def test_status_json_matches_v2(sandbox, history_brain):
    r = run(sandbox, "status", "--json")
    assert r["v2"].code == 0, r["v2"].stderr
    assert r["v3"].code == 0, r["v3"].stderr
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    status = only_brain(json.loads(r["v2"].stdout))
    assert status["lobes"] == 2 and status["neurons"] == 4, status
    assert status["recent_commits"] == [f"update {i}" for i in range(11, 1, -1)], "ten newest"
    assert status["uncommitted"] == "" and status["git_enabled"] is True


def test_status_text_matches_v2(sandbox, history_brain):
    r = run(sandbox, "status")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    same_words(v2.stdout, v3.stdout, sandbox.root)
    lines = v3.stdout.splitlines()
    assert "demo" in lines and "  Lobes: 2, Neurons: 4" in lines, lines
    dated = [line for line in lines if re.match(r"  \d{4}-\d\d-\d\d ", line)]
    assert dated == [f"  2026-02-{i:02d} update {i}" for i in range(11, 6, -1)], "five newest, date[:10]"
    assert "Git: disabled" not in v3.stdout and "Uncommitted" not in v3.stdout


def test_status_of_a_dirty_brain(sandbox: Sandbox):
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", brain_files())
    # A modified tracked file first: v2 strips the whole status text, so the
    # first line loses its leading space (" M brain.md" -> "M brain.md").
    write(brain, {"brain.md": "# changed\n", "projects/alpha.md": "changed", "projects/new.md": neuron("New"),
                  "sp ace.md": "x"})
    git(sandbox, brain, "add", "projects/alpha.md")
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    j2, j3 = json_pair(sandbox, "status", "--json")
    assert j3 == j2
    assert only_brain(j2)["uncommitted"] == 'M brain.md\nM  projects/alpha.md\n?? projects/new.md\n?? "sp ace.md"'

    r = run(sandbox, "status")
    same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert "  Uncommitted:\nM brain.md\nM  projects/alpha.md\n" in r["v3"].stdout


def test_status_of_a_brain_without_git(sandbox: Sandbox):
    brain = sandbox.work / "brains" / "plain"
    write(brain, brain_files())
    register(sandbox, [("plain", brain)])
    sandbox.freeze()

    j2, j3 = json_pair(sandbox, "status", "--json")
    assert j3 == j2
    assert only_brain(j2) == {
        "name": "plain", "lobes": 2, "neurons": 4, "uncommitted": "", "git_enabled": False, "recent_commits": [],
    }
    r = run(sandbox, "status")
    same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert "  Git: disabled\n" in r["v3"].stdout


def test_status_brain_all(sandbox: Sandbox):
    a = init(sandbox, sandbox.work / "brains" / "a")
    commit(sandbox, a, "brain: initialize a", brain_files())
    write(a, {"projects/draft.md": "draft"})
    b = sandbox.work / "brains" / "b"
    write(b, {"brain.md": "# b\n", "only/map.md": "# Only\n", "only/n.md": neuron("N")})
    register(sandbox, [("a", a), ("b", b)])
    sandbox.freeze()

    r = run(sandbox, "status", "--brain", "all", "--json")
    assert r["v2"].code == r["v3"].code == 0, (r["v2"].stderr, r["v3"].stderr)
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert [s["name"] for s in json.loads(r["v3"].stdout)["brains"]] == ["a", "b"]

    r = run(sandbox, "status", "--brain", "all")
    same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    text = r["v3"].stdout
    assert text.index("\na\n") < text.index("\nb\n") < text.index("Git: disabled")

    r = run(sandbox, "status", "--brain", "b", "--json")
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_status_with_two_brains_and_no_choice(sandbox: Sandbox, json_flag):
    a, b = sandbox.work / "a", sandbox.work / "b"
    a.mkdir()
    b.mkdir()
    register(sandbox, [("a", a), ("b", b)])
    sandbox.freeze()
    args = ["status", "--json"] if json_flag else ["status"]
    r = run(sandbox, *args)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 1
    assert v3.stdout == v2.stdout and v3.stderr == v2.stderr
    assert "Pass --brain NAME or 'all'. Available: a, b" in v2.stdout + v2.stderr


# --- status: the allowlisted git fixes ------------------------------------------


def test_status_subject_with_pipes(sandbox: Sandbox):
    """PAR-012: v2 split ``%H|%s|%aI`` on ``|``."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", brain_files())
    commit(sandbox, brain, "has | pipe | char", {"projects/alpha.md": "x"})
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    j2, j3 = json_pair(sandbox, "status", "--json")
    assert only_brain(j2)["recent_commits"] == ["has ", "brain: initialize demo"]
    assert only_brain(j3)["recent_commits"] == ["has | pipe | char", "brain: initialize demo"]
    only_brain(j2)["recent_commits"][0] = "has | pipe | char"
    assert j3 == j2

    r = run(sandbox, "status")
    assert "   pipe | ch has \n" in r["v2"].stdout, "v2's date[:10] was the subject's tail"
    assert "  2026-01-01 has | pipe | char\n" in r["v3"].stdout


def test_status_non_ascii_names(sandbox: Sandbox):
    """PAR-013: ``-c core.quotePath=false``; v2 printed C-quoted octal bytes."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", {**brain_files(), "projects/ñandú.md": neuron("Ñ")})
    write(brain, {"projects/é.md": neuron("E"), "projects/ñandú.md": "changed"})
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    j2, j3 = json_pair(sandbox, "status", "--json")
    assert only_brain(j2)["uncommitted"] == 'M "projects/\\303\\261and\\303\\272.md"\n?? "projects/\\303\\251.md"'
    assert only_brain(j3)["uncommitted"] == "M projects/ñandú.md\n?? projects/é.md"
    only_brain(j2)["uncommitted"] = only_brain(j3)["uncommitted"]
    assert j3 == j2
    r = run(sandbox, "status")
    assert "?? projects/é.md" in r["v3"].stdout


@pytest.fixture
def nested_brain(sandbox: Sandbox) -> Path:
    """A brain in ``docs/brain`` of a larger repo, with commits and changes on
    both sides of the brain directory."""
    outer = init(sandbox, sandbox.work / "outer")
    commit(sandbox, outer, "outer: start", {"README.md": "r\n"})
    commit(sandbox, outer, "brain: initialize demo", {f"docs/brain/{k}": v for k, v in brain_files().items()})
    commit(sandbox, outer, "outer: code", {"src/main.rs": "fn main() {}\n"})
    write(outer, {"README.md": "changed\n", "docs/brain/projects/alpha.md": "changed", "docs/brain/n.md": "n"})
    brain = outer / "docs" / "brain"
    register(sandbox, [("demo", brain)])
    sandbox.freeze()
    return brain


def test_status_nested_brain_is_scoped_to_the_brain(sandbox, nested_brain):
    """PAR-014: v2 reported the outer repo's log and status."""
    j2, j3 = json_pair(sandbox, "status", "--json")
    s2, s3 = only_brain(j2), only_brain(j3)
    assert s2["recent_commits"] == ["outer: code", "brain: initialize demo", "outer: start"]
    assert s2["uncommitted"] == "M ../../README.md\n M projects/alpha.md\n?? n.md"
    assert s3["recent_commits"] == ["brain: initialize demo"]
    assert s3["uncommitted"] == "M projects/alpha.md\n?? n.md"
    for key in ("recent_commits", "uncommitted"):
        s2[key] = s3[key]
    assert j3 == j2

    r = run(sandbox, "status")
    assert "outer: code" in r["v2"].stdout
    assert "outer: code" not in r["v3"].stdout and "README" not in r["v3"].stdout
    assert "  2026-01-01 brain: initialize demo\n" in r["v3"].stdout


def test_status_of_a_repo_without_commits(sandbox: Sandbox):
    """PAR-015: v2's ``git log`` fails without HEAD and status ends in a traceback."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    write(brain, {"brain.md": "# demo\n"})
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    r = run(sandbox, "status", "--json")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1 and v2.stdout == "" and "CalledProcessError" in v2.stderr
    assert v3.code == 0, v3.stderr
    assert json.loads(v3.stdout) == {"ok": True, "brains": [{
        "name": "demo", "lobes": 0, "neurons": 0, "uncommitted": "?? brain.md", "git_enabled": True,
        "recent_commits": [],
    }]}


def test_status_of_a_corrupt_repo(sandbox: Sandbox):
    """PAR-023: a ``git log`` failure other than "no commits yet" is an error
    naming the git command; v2 ends in a ``CalledProcessError`` traceback."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", brain_files())
    head = git(sandbox, brain, "rev-parse", "HEAD").strip()
    obj = brain / ".git" / "objects" / head[:2] / head[2:]
    obj.chmod(0o644)  # git writes loose objects read-only (Windows will not remove them)
    obj.unlink()
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    error = "`git log -10 --format=%H%x00%s%x00%aI%x00` failed (exit 128): fatal: bad object HEAD"
    r = run(sandbox, "status", "--json")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1 and v2.stdout == "" and "CalledProcessError" in v2.stderr
    assert v3.code == 1, v3.stderr
    assert v3.stdout == json.dumps({"ok": False, "error": error}) + "\n"
    r = run(sandbox, "status")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1 and "CalledProcessError" in v2.stderr
    assert (v3.code, v3.stdout, v3.stderr) == (1, "", f"Error: {error}\n")


@pytest.mark.parametrize("damage", ["broken-ref", "orphan-branch"])
def test_status_when_head_does_not_verify(sandbox: Sandbox, damage):
    """PAR-023, review T-79 finding 1: HEAD that does not name a commit is "no
    commits" only in a fresh repo; a broken branch ref (``invalid-oid``) or an
    orphan branch beside other history is the ``git log`` error, as v2 fails."""
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "brain: initialize demo", brain_files())
    if damage == "broken-ref":
        branch = git(sandbox, brain, "symbolic-ref", "HEAD").strip()
        (brain / ".git" / branch).write_text("invalid-oid\n", encoding="utf-8")
        why = "fatal: your current branch appears to be broken"
    else:
        git(sandbox, brain, "checkout", "-q", "--orphan", "fresh")
        why = "fatal: your current branch 'fresh' does not have any commits yet"
    register(sandbox, [("demo", brain)])
    sandbox.freeze()

    error = f"`git log -10 --format=%H%x00%s%x00%aI%x00` failed (exit 128): {why}"
    r = run(sandbox, "status", "--json")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1 and v2.stdout == "" and "CalledProcessError" in v2.stderr
    assert (v3.code, v3.stdout) == (1, json.dumps({"ok": False, "error": error}) + "\n"), v3


# --- human output: user text is never markup (PAR-016) ---------------------------


def test_list_and_status_print_user_text_literally(sandbox: Sandbox):
    brain = init(sandbox, sandbox.work / "brains" / "demo")
    commit(sandbox, brain, "fix [bold]this[/bold] now", brain_files())
    register(sandbox, [("demo", brain)], description="see [red]docs[/red]")
    sandbox.freeze()

    r = run(sandbox, "list")
    assert r["v2"].code == r["v3"].code == 0
    assert "see docs" in r["v2"].stdout, "v2 renders Rich markup"
    assert "see [red]docs[/red]" in r["v3"].stdout
    r = run(sandbox, "status")
    assert "fix this now" in r["v2"].stdout
    assert "  2026-01-01 fix [bold]this[/bold] now\n" in r["v3"].stdout


def test_list_with_a_closing_tag_that_crashed_v2(sandbox: Sandbox):
    brain = sandbox.work / "brains" / "demo"
    brain.mkdir(parents=True)
    register(sandbox, [("demo", brain)], description="ends [/oops]")
    sandbox.freeze()
    r = run(sandbox, "list")
    assert r["v2"].code == 1 and "MarkupError" in r["v2"].stderr
    assert r["v3"].code == 0 and "ends [/oops]" in r["v3"].stdout


# --- list --------------------------------------------------------------------------


def test_list_text_matches_v2(sandbox: Sandbox):
    brains = sandbox.work / "brains"
    for name, companions in (("alpha", "companions:\n- specmint-core\n- specmint-tdd\n"), ("beta", "companions: []\n")):
        write(brains / name, {"kluris.yml": f"name: {name}\n{companions}"})
    (brains / "gamma").mkdir()
    register(sandbox, [("alpha", brains / "alpha"), ("beta", brains / "beta"), ("gamma", brains / "gamma")])
    sandbox.freeze()

    r = run(sandbox, "list")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    for text in ("Registered Brains", "Name", "Path", "Description", "Companions",
                 "specmint-core, specmint-tdd", "(none)", "(unknown)", "demo brain"):
        assert text in v2.stdout and text in v3.stdout, text
    rows = [line for line in v3.stdout.splitlines() if line.startswith(("alpha", "beta", "gamma"))]
    assert [row.split()[0] for row in rows] == ["alpha", "beta", "gamma"]
    for name, cell in (("alpha", "specmint-core, specmint-tdd"), ("beta", "(none)"), ("gamma", "(unknown)")):
        row = next(row for row in rows if row.startswith(name))
        assert normalize(str(brains / name), sandbox.root) in normalize(row, sandbox.root) and row.endswith(cell), row

    r = run(sandbox, "list", "--json")
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)


def test_list_text_without_brains(sandbox: Sandbox):
    r = run(sandbox, "list")
    assert r["v3"].stdout == r["v2"].stdout == "No brains registered. Run 'kluris create <name>' to create one.\n"


# --- help --------------------------------------------------------------------------


def without_pack(v2_text: str) -> str:
    lines = v2_text.splitlines(keepends=True)
    kept = [line for line in lines if not line.lstrip().startswith("pack ")]
    assert len(kept) == len(lines) - 1, "v2 lists pack once"
    return "".join(kept)


def with_v3_only(table: str) -> str:
    """A help table with the v3-only rows after ``help``'s."""
    lines = table.splitlines(keepends=True)
    at = next(i for i, line in enumerate(lines) if line.startswith("  help ")) + 1
    rows = [f"  {name:<10} {desc}\n" for name, desc in V3_ONLY]
    return "".join(lines[:at] + rows + lines[at:])


def test_help_text_matches_v2_without_pack(sandbox: Sandbox):
    write_config(sandbox, [("a", str(sandbox.work / "a"), "x"), ("b", str(sandbox.work / "b"), "y")])
    sandbox.freeze()
    r = run(sandbox, "help")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    same_words(with_v3_only(without_pack(v2.stdout)), v3.stdout, sandbox.root)
    names = [m.group(1) for m in re.finditer(r"^  (\S+) +\S", v3.stdout, re.M)]
    assert names == V3_COMMANDS
    assert "Brains: 2 registered" in v3.stdout
    assert normalize(f"Config: {sandbox.config}", sandbox.root) in normalize(v3.stdout, sandbox.root)
    assert "pack" not in v3.stdout
    assert "  mri        Generate interactive brain visualization; prints a file:// link\n" in v3.stdout


def test_help_json_lists_v2s_commands_without_pack(sandbox: Sandbox):
    j2, j3 = json_pair(sandbox, "help", "--json")
    assert [c["name"] for c in j2["commands"]] == V2_COMMANDS
    j2["commands"] = v3_rows(j2["commands"])
    assert j3 == j2
    assert json.dumps(j3) == json.dumps(j2)


@pytest.mark.parametrize("command", V2_KEPT)
def test_help_for_one_command(sandbox: Sandbox, command):
    r = run(sandbox, "help", command)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    same_words(v2.stdout, v3.stdout, sandbox.root)
    assert v3.stdout.startswith(f"kluris {command} — ")
    assert f"Run 'kluris {command} --help' for full usage." in v3.stdout


@pytest.mark.parametrize(("command", "desc"), V3_ONLY)
def test_help_for_a_v3_only_command(sandbox: Sandbox, command, desc):
    """PAR-064: v2 has no such command."""
    r = run(sandbox, "help", command)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1 and v2.stderr == f"Error: Unknown command: {command}\n"
    assert v3.code == 0, v3.stderr
    assert v3.stdout == f"kluris {command} — {desc}\n\nRun 'kluris {command} --help' for full usage.\n"
    j3 = json.loads(run(sandbox, "help", command, "--json")["v3"].stdout)
    assert j3 == {"ok": True, "commands": [{"name": command, "description": desc}]}


@pytest.mark.parametrize("command", ["bogus", "pack"])
def test_help_for_an_unknown_command(sandbox: Sandbox, command):
    r = run(sandbox, "help", command)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == 1 and v3.stdout == "" and v3.stderr == f"Error: Unknown command: {command}\n"
    if command == "bogus":
        assert (v2.code, v2.stdout, v2.stderr) == (v3.code, v3.stdout, v3.stderr)
    else:
        assert v2.code == 0, "pack is a v2 command (PAR-018)"


def test_help_json_with_a_command(sandbox: Sandbox):
    """PAR-017: v2's ``help --json`` ignored COMMAND."""
    j2, j3 = json_pair(sandbox, "help", "status", "--json")
    assert len(j2["commands"]) == len(V2_COMMANDS)
    assert j3 == {"ok": True, "commands": [
        {"name": "status", "description": "Show brain tree, recent changes, and neuron counts"},
    ]}
    for command in ("bogus", "pack"):
        r = run(sandbox, "help", command, "--json")
        assert r["v2"].code == 0
        assert r["v3"].code == 1
        assert r["v3"].stdout == json.dumps({"ok": False, "error": f"Unknown command: {command}"}) + "\n"


def test_help_with_a_corrupt_config(sandbox: Sandbox):
    sandbox.config.parent.mkdir(parents=True)
    sandbox.config.write_text("brains: [\n", encoding="utf-8")
    sandbox.freeze()
    r = run(sandbox, "help")
    assert r["v2"].code == 1 and "Traceback" in r["v2"].stderr
    assert r["v3"].code == 1 and r["v3"].stderr.startswith("Error: Invalid config file ")


# --- the CLI surface: --help like Click, no -h / -V / clap help ---------------------


# The commands Kluris 3 implements so far, with v2's --help wording.
HELP_TEXTS = {
    "list": ["List registered brains.", "--json", "JSON output", "--help", "Show this message and exit."],
    "status": ["Show brain status and recent changes.", "--brain", "Specific brain", "--json", "JSON output"],
    "help": ["Show help for kluris commands.", "--json", "JSON output"],
    "doctor": ["Check prerequisites and refresh installed agent skills.", "--no-refresh",
               "Skip refreshing installed agent skills", "--json", "JSON output"],
    "wake-up": ["Compact brain snapshot for agent session bootstrap.",
                "kluris wake-up --json          # machine-readable (for agents)", "--brain", "Specific brain"],
}


@pytest.mark.parametrize("command", sorted(HELP_TEXTS))
def test_command_help_flag(sandbox: Sandbox, command):
    r = run(sandbox, command, "--help")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    assert v3.stderr == ""
    assert f"Usage: kluris {command} [OPTIONS]" in v3.stdout
    for text in HELP_TEXTS[command]:
        assert text in v2.stdout, f"v2 no longer says {text!r}"
        assert text in v3.stdout, text


def test_root_help_flag(sandbox: Sandbox):
    r = run(sandbox, "--help")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    assert "Kluris — Turn AI agents into team subject matter experts." in v3.stdout
    assert "--version  Show the version and exit." in v2.stdout
    for text in ("--version", "Show the version and exit.", "--help", "Show this message and exit."):
        assert text in v3.stdout, text
    for command in [*HELP_TEXTS, "companion"]:
        line = next(line for line in v2.stdout.splitlines() if line.startswith(f"  {command} "))
        assert re.search(rf"^  {re.escape(command)} +{re.escape(line.split(maxsplit=1)[1])}$", v3.stdout, re.M), line
    assert "pack" not in v3.stdout
    for command, _ in V3_ONLY:  # PAR-064
        assert command not in v2.stdout
        assert re.search(rf"^  {re.escape(command)} +\S", v3.stdout, re.M), command


@pytest.mark.parametrize("args", [["-h"], ["-V"], ["status", "-h"], ["help", "-h"], ["help", "--bogus"],
                                  ["help", "a", "b"], ["bogus"], ["list", "-h"]])
def test_usage_errors_like_click(sandbox: Sandbox, args):
    r = run(sandbox, *args)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 2, (v2.code, v3.code, v3.stderr)
    assert v3.stdout == v2.stdout == ""
    error = v2.stderr.strip().splitlines()[-1]
    assert error.startswith("Error: ")
    assert v3.stderr.strip().splitlines()[-1] == error


@pytest.mark.parametrize("args", [["help", "--json", "bogus", "extra"], ["bogus", "--json"], ["pack", "--json"]])
def test_usage_errors_with_json(sandbox: Sandbox, args):
    r = run(sandbox, *args)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == 1
    if args[0] == "pack":
        assert v3.stdout == json.dumps({"ok": False, "error": "No such command 'pack'."}) + "\n"
    else:
        assert v2.code == 1
        assert v3.stdout == v2.stdout


# --- options as Click parses them (review T-79 finding 3 and the P3 table) ----------


@pytest.fixture
def two_brains(sandbox: Sandbox) -> None:
    """Brains ``a`` (a lobe with one tagged neuron) and ``b`` (empty), no git."""
    write(sandbox.work / "brains" / "a", {
        "brain.md": "# a\n",
        "lobe/map.md": "# Lobe\n",
        "lobe/needle.md": "---\nparent: map.md\ntags: [y]\nupdated: 2026-01-01\n---\n# Needle\n\nneedle text\n",
    })
    (sandbox.work / "brains" / "b").mkdir(parents=True)
    register(sandbox, [("a", sandbox.work / "brains" / "a"), ("b", sandbox.work / "brains" / "b")])
    sandbox.freeze()


def search_view(doc: dict) -> tuple:
    """What a repeated search option decides: the brain, the query and the
    files found (v3 ranks and adds keys of its own, PAR-020)."""
    return doc["ok"], doc["brain"], doc["query"], sorted(r["file"] for r in doc["results"])


REPEATED = [
    ["list", "--json", "--json"],
    ["help", "--json", "--json"],
    ["help", "status", "--json", "--json"],
    ["status", "--brain", "b", "--brain", "a", "--json", "--json"],
    ["wake-up", "--brain", "b", "--brain", "a", "--json", "--json"],
    ["search", "needle", "--brain", "b", "--brain", "a", "--json", "--json"],
    ["search", "needle", "--brain", "a", "--lobe", "nope", "--lobe", "lobe", "--json"],
    ["search", "needle", "--brain", "a", "--lobe", "lobe", "--lobe", "nope", "--json"],
    ["search", "needle", "--brain", "a", "--tag", "x", "--tag", "y", "--json"],
    ["search", "needle", "--brain", "a", "--tag", "y", "--tag", "x", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "0", "--limit", "1", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "1", "--limit", "0", "--json"],
]


@pytest.mark.parametrize("args", REPEATED, ids=lambda a: " ".join(a))
def test_a_repeated_option_takes_its_last_value(sandbox, two_brains, args):
    """Click lets every option repeat, the last value winning; clap refuses a
    repeat unless told otherwise."""
    j2, j3 = json_pair(sandbox, *args)
    if args[0] == "search":
        assert search_view(j3) == search_view(j2)
        assert j3["brain"] == "a"
    elif args[0] == "wake-up":
        assert j3 == j2 and j3["name"] == "a"
    elif args[0] == "help" and len(args) > 3:
        assert j3["commands"] == [{"name": "status", "description": "Show brain tree, recent changes, and neuron counts"}]
    elif args[0] == "help":
        j2["commands"] = v3_rows(j2["commands"])
        assert j3 == j2
    else:
        assert j3 == j2


def outcome(result, root: Path) -> tuple:
    """How a run ended, in terms both CLIs share: ``("help", "kluris list")``
    (whose help: the command path of its usage line), ``("version",)``,
    ``("error", <code>, <last stderr line>)`` or ``("ran", <code>)``."""
    if result.code == 0 and result.stderr == "" and "Usage: kluris" in result.stdout:
        usage = next(line for line in result.stdout.splitlines() if line.startswith("Usage: "))
        words = [w for w in usage.split()[1:] if not w.startswith(("[", "<")) and not w.isupper()]
        return ("help", " ".join(words))
    if result.code == 0 and re.fullmatch(r"kluris \S+\n", result.stdout):
        return ("version",)
    if result.code != 0 and result.stdout == "" and result.stderr:
        return ("error", result.code, normalize(result.stderr.strip().splitlines()[-1], root))
    return ("ran", result.code)


# Click parses every token first (an unknown option, an option without its
# value, a value given to a flag fail there, wherever --help is), then runs
# the eager --help / --version (in the order given) before converting values,
# checking required arguments or refusing extra ones.
PRECEDENCE = [
    (["list", "--help", "--bogus"], ("error", 2, "Error: No such option '--bogus'.")),
    (["list", "--bogus", "--help"], ("error", 2, "Error: No such option '--bogus'.")),
    (["list", "--help", "--json=1"], ("error", 2, "Error: Option '--json' does not take a value.")),
    (["list", "--json=1", "--help"], ("error", 2, "Error: Option '--json' does not take a value.")),
    (["list", "--help", "--help"], ("help", "kluris list")),
    (["list", "--json", "--help", "--json"], ("help", "kluris list")),
    (["status", "--help", "--brain"], ("error", 2, "Error: Option '--brain' requires an argument.")),
    (["status", "--brain", "x", "--help"], ("help", "kluris status")),
    (["status", "--brain=x", "--help", "--bogus"], ("error", 2, "Error: No such option '--bogus'.")),
    (["wake-up", "--help", "--brain"], ("error", 2, "Error: Option '--brain' requires an argument.")),
    (["search", "--help"], ("help", "kluris search")),
    (["search", "needle", "--limit", "x", "--help"], ("help", "kluris search")),
    (["search", "--limit", "x", "needle", "--help"], ("help", "kluris search")),
    (["search", "needle", "--limit=x", "--help"], ("help", "kluris search")),
    (["search", "needle", "--help", "extra"], ("help", "kluris search")),
    (["search", "--help", "--limit"], ("error", 2, "Error: Option '--limit' requires an argument.")),
    (["search", "--tag", "--help"], ("error", 2, "Error: Missing argument 'QUERY'.")),
    (["help", "a", "b", "--help"], ("help", "kluris help")),
    (["help", "bogus", "--help"], ("help", "kluris help")),
    (["help", "--json", "--json", "--help"], ("help", "kluris help")),
    (["--help", "--help"], ("help", "kluris")),
    (["--help", "list"], ("help", "kluris")),
    (["--help", "bogus"], ("help", "kluris")),
    (["--help", "--bogus"], ("error", 2, "Error: No such option '--bogus'.")),
    (["bogus", "--help"], ("error", 2, "Error: No such command 'bogus'.")),
    (["--version", "--version"], ("version",)),
    (["--version", "--help"], ("version",)),
    (["--help", "--version"], ("help", "kluris")),
    (["--version", "--bogus"], ("error", 2, "Error: No such option '--bogus'.")),
    (["--bogus", "--version"], ("error", 2, "Error: No such option '--bogus'.")),
    (["--version", "list"], ("version",)),
    # The root's eager options act before Click descends into the command,
    # and the first occurrence decides their order (review T-84 finding 4).
    (["--help", "list", "--bogus"], ("help", "kluris")),
    (["--version", "list", "--bogus"], ("version",)),
    (["--version", "bogus", "--bogus"], ("version",)),
    (["--help", "search"], ("help", "kluris")),
    (["--help", "--version", "--help"], ("help", "kluris")),
    (["--version", "--help", "--version"], ("version",)),
    (["--version", "--help", "--help", "--version"], ("version",)),
]


@pytest.mark.parametrize("args,expected", PRECEDENCE, ids=[" ".join(a) for a, _ in PRECEDENCE])
def test_help_and_usage_errors_in_clicks_order(sandbox, two_brains, args, expected):
    r = run(sandbox, *args)
    v2, v3 = outcome(r["v2"], sandbox.root), outcome(r["v3"], sandbox.root)
    assert v2 == expected, f"the oracle no longer does this: {r['v2']}"
    assert v3 == expected, r["v3"]


# Every option that takes a value, repeated with an invalid value before or
# after a valid one: Click converts only the value that wins (review T-84
# finding 1).
REPEATED_VALUES = [
    ["search", "needle", "--brain", "a", "--limit", "", "--limit", "2", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "invalid", "--limit", "2", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "2.0", "--limit", "2", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "2", "--limit", "x", "--json"],
    ["search", "needle", "--brain", "a", "--limit", "2", "--limit", "x"],
    ["search", "needle", "--brain", "a", "--limit", "2", "--limit", "", "--json"],
    ["search", "needle", "--brain", "nope", "--brain", "a", "--json"],
    ["search", "needle", "--brain", "a", "--brain", "nope", "--json"],
    ["search", "needle", "--brain", "", "--brain", "a", "--json"],
    ["search", "needle", "--brain", "a", "--lobe", "", "--lobe", "lobe", "--json"],
    ["search", "needle", "--brain", "a", "--tag", "", "--tag", "y", "--json"],
    ["status", "--brain", "nope", "--brain", "a", "--json"],
    ["status", "--brain", "a", "--brain", "nope", "--json"],
    ["status", "--brain", "a", "--brain", "nope"],
    ["status", "--brain", "", "--brain", "all", "--json"],
    ["wake-up", "--brain", "nope", "--brain", "a", "--json"],
    ["wake-up", "--brain", "a", "--brain", "nope", "--json"],
    ["wake-up", "--brain", "all", "--brain", "a", "--json"],
]


@pytest.mark.parametrize("args", REPEATED_VALUES, ids=lambda a: " ".join(a))
def test_only_the_winning_value_is_converted(sandbox, two_brains, args):
    r = run(sandbox, *args)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == v2.code, (v2.code, v3.code, v2.stdout + v2.stderr, v3.stdout + v3.stderr)
    if v2.code == 0:
        j2, j3 = parse_exact(v2.stdout, sandbox.root, "v2"), parse_exact(v3.stdout, sandbox.root, "v3")
        assert (search_view(j3) == search_view(j2)) if args[0] == "search" else (j3 == j2)
    else:
        assert normalize(v3.stdout, sandbox.root) == normalize(v2.stdout, sandbox.root)
        assert outcome(v3, sandbox.root) == outcome(v2, sandbox.root)


@pytest.mark.parametrize("args", [[], ["list"], ["status"], ["wake-up"], ["search"], ["help"],
                                  ["companion"], ["companion", "list"]],
                         ids=lambda a: " ".join(a) or "root")
def test_usage_line_is_v2s(sandbox, two_brains, args):
    """Required arguments read `QUERY`, optional ones `[COMMAND]`, in help
    (rendered from the strict schema, review T-84 finding 3) and in errors."""
    r = run(sandbox, *args, "--help")
    usage = [next(line for line in r[side].stdout.splitlines() if line.startswith("Usage: ")) for side in ("v2", "v3")]
    assert usage[1] == usage[0]


def test_usage_line_of_a_usage_error_is_v2s(sandbox, two_brains):
    r = run(sandbox, "search")
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 2
    assert v3.stderr == v2.stderr == (
        "Usage: kluris search [OPTIONS] QUERY\nTry 'kluris search --help' for help.\n\n"
        "Error: Missing argument 'QUERY'.\n"
    )
