"""``kluris companion list|add|remove`` against the v2.28.11 oracle
(research-03 §1.7-1.9, §2.10).

Brains and HOME are written by v2 itself (``create``), so every run starts
from what a v2 user has. Compared:

- JSON byte for byte (after root normalization), exit codes, and human
  output by v2's wording (``same_words``) or by substring;
- the installed ``~/.kluris/companions`` tree byte for byte;
- each brain's ``kluris.yml``: v2's bytes with PAR-003's agent names when v3
  rewrote it, untouched otherwise;
- the re-rendered skills: v2's SKILL.md with the decided changes (PAR-026 ...
  PAR-031, via ``test_skill_install``), v2's per-agent copies removed.

Allowlisted: PAR-039 (``remove`` touches only brains that had the companion),
PAR-040 (a brain whose ``kluris.yml`` does not read is an error before
anything is written).
"""

from __future__ import annotations

import json
import subprocess
import time
from pathlib import Path

import pytest

from harness import IS_WINDOWS, Result, Sandbox, assert_json_equal, normalize, run_v2, snapshot_tree
from runs import _env, _exe, run_pty, same_words
from test_skill_install import V2_ONLY, _v2_to_v3

KNOWN = ["specmint-core", "specmint-tdd", "specmint-core-html", "specmint-tdd-html"]
BRAINS = ["a", "b"]
# Skill folders v3 writes for v2's default agents (no Hermes profiles here).
V3_SKILL_DIRS = [".agents/skills", ".claude/skills", ".hermes/skills", ".gemini/config/skills"]
NAME_REQUIRED = (
    "Companion name is required in non-interactive mode. Pass one of: "
    "specmint-core, specmint-tdd, specmint-core-html, specmint-tdd-html"
)
BAD_CHOICE = (
    "Invalid value for '[specmint-core|specmint-tdd|specmint-core-html|specmint-tdd-html]': "
    "'{value}' is not one of 'specmint-core', 'specmint-tdd', 'specmint-core-html', 'specmint-tdd-html'."
)


@pytest.fixture
def sandbox(tmp_path):
    return Sandbox(tmp_path)


def oracle(sb: Sandbox, *args: str) -> dict:
    r = run_v2(list(args), sb.env)
    assert r.code == 0, f"oracle {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout) if "--json" in args else {}


def v2_brains(sb: Sandbox, *companions: tuple[str, str]) -> Path:
    """Brains ``a`` (git) and ``b`` (no git) created by v2, then each
    ``(companion, brain)`` added by v2."""
    brains = sb.work / "brains"
    brains.mkdir()
    oracle(sb, "create", "a", "--path", str(brains), "--description", "Brain A", "--json")
    oracle(sb, "create", "b", "--path", str(brains), "--no-git", "--json")
    for name, brain in companions:
        oracle(sb, "companion", "add", name, "--brain", brain, "--json")
    return brains


def run_trees(sb: Sandbox, args: list[str], env: dict[str, str | None] | None = None,
              stdin: bytes | None = b"") -> dict[str, Result]:
    """Each binary on its own copy of the frozen state; every result carries
    the ``work/`` tree after its run."""
    results: dict[str, Result] = {}
    for side in ("v2", "v3"):
        sb.checkout()
        try:
            proc = subprocess.run(
                [*_exe(side), *args], env=_env(sb, env or {}), cwd=str(sb.work),
                input=stdin, capture_output=True, timeout=120,
            )
            result = Result(proc.returncode, proc.stdout.decode("utf-8").replace("\r\n", "\n"),
                            proc.stderr.decode("utf-8").replace("\r\n", "\n"))
            result.tree = snapshot_tree(sb.work)
        finally:
            sb.stash(f"{side}-{time.monotonic_ns()}")
        results[side] = result
    return results


def pristine(sb: Sandbox) -> dict[str, bytes]:
    return snapshot_tree(sb.root / "pristine")


def _replace_once(text: str, old: str, new: str) -> str:
    assert text.count(old) == 1, (old, text)
    return text.replace(old, new)


def par003(v2_yml: bytes) -> bytes:
    """v2's ``kluris.yml`` as v3 writes the same model: ``windsurf`` read as
    ``devin`` and v2's default list as v3's, with ``antigravity``."""
    text = v2_yml.decode("utf-8")
    crlf = "\r\n" in text
    text = text.replace("\r\n", "\n")
    text = _replace_once(text, "  - windsurf\n", "  - devin\n")
    text = _replace_once(text, "  - junie\n", "  - junie\n  - antigravity\n")
    return (text.replace("\n", "\r\n") if crlf else text).encode("utf-8")


def companions_tree(tree: dict[str, bytes]) -> dict[str, bytes]:
    return {k: v for k, v in tree.items() if k.startswith("home/.kluris/companions/")}


def expected_v3(v2_tree: dict[str, bytes], before: dict[str, bytes], rewritten: set[str]) -> dict[str, bytes]:
    """The tree v3 should leave, from v2's: skills per PAR-026...PAR-031 and
    the brains in ``rewritten`` with PAR-003's ``kluris.yml``; every other
    brain's ``kluris.yml`` as it was before the run."""
    exp = dict(v2_tree)
    for brain in BRAINS:
        v2_skill = v2_tree[f"home/.claude/skills/kluris-{brain}/SKILL.md"].decode("utf-8").replace("\r\n", "\n")
        v3_skill = _v2_to_v3(v2_skill, brain).encode("utf-8")
        for rel in V3_SKILL_DIRS:
            exp[f"home/{rel}/kluris-{brain}/SKILL.md"] = v3_skill
        for rel in V2_ONLY:
            exp.pop(f"home/{rel}/kluris-{brain}/SKILL.md", None)
        exp.pop(f"home/.codeium/windsurf/global_workflows/kluris-{brain}.md", None)
        key = f"brains/{brain}/kluris.yml"
        if key in before:
            exp[key] = par003(v2_tree[key]) if brain in rewritten else before[key]
    return exp


def assert_trees(r: dict[str, Result], before: dict[str, bytes], rewritten: set[str]) -> None:
    v2, v3 = r["v2"].tree, r["v3"].tree
    assert companions_tree(v3) == companions_tree(v2), "the installed companions differ"
    exp = expected_v3(v2, before, rewritten)
    changed = sorted(k for k in set(exp) | set(v3) if exp.get(k) != v3.get(k))
    assert not changed, changed


def json_of(result: Result, sb: Sandbox) -> dict:
    text = normalize(result.stdout, sb.root)
    doc = json.loads(text)
    assert text == json.dumps(doc) + "\n", text
    return doc


# --- list ------------------------------------------------------------------------


def test_list_without_brains(sandbox: Sandbox):
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "list", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    r = run_trees(sandbox, ["companion", "list"])
    assert r["v2"].code == r["v3"].code == 0
    for text in ("Known Companions", "Name", "Installed", "specmint-tdd-html",
                 "No brains registered. Run 'kluris create <name>' to create one."):
        assert text in r["v2"].stdout and text in r["v3"].stdout, text


def list_scenario(sb: Sandbox) -> Path:
    """``a`` with two companions, ``b`` without, ``c`` with no ``kluris.yml``
    (unknown), ``d`` with an invalid one, an installed folder and a stray
    file at a companion path."""
    brains = v2_brains(sb, ("specmint-tdd-html", "a"), ("specmint-core", "a"))
    for name, text in (("c", None), ("d", "name: [broken\n")):
        (brains / name).mkdir()
        if text:
            (brains / name / "kluris.yml").write_text(text, encoding="utf-8")
    config = sb.config.read_text(encoding="utf-8")
    for name in ("c", "d"):
        config += f"  {name}:\n    path: {json.dumps(str(brains / name))}\n    description: ''\n"
    sb.config.write_text(config, encoding="utf-8")
    (sb.home / ".kluris" / "companions" / "specmint-tdd").write_text("stray", encoding="utf-8")
    return brains


@pytest.mark.parametrize("args", [[], ["--brain", "all"], ["--brain", "a"], ["--brain", "c"]])
def test_list_json(sandbox: Sandbox, args):
    list_scenario(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "list", *args, "--json"])
    assert r["v2"].code == r["v3"].code == 0, (r["v2"].stderr, r["v3"].stderr)
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert r["v3"].tree == r["v2"].tree == pristine(sandbox), "list writes nothing"


def test_list_text(sandbox: Sandbox):
    list_scenario(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "list"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0
    for text in ("Known Companions", "Installed", "Brain Companion Opt-ins", "Companions",
                 "specmint-core, specmint-tdd-html", "(none)", "(unknown)"):
        assert text in v2.stdout and text in v3.stdout, text
    rows = {line.split()[0]: line.split()[1:] for line in v3.stdout.splitlines() if line.strip()}
    assert rows["specmint-core"] == ["yes"] and rows["specmint-tdd"] == ["yes"]
    assert rows["specmint-core-html"] == ["no"] and rows["specmint-tdd-html"] == ["yes"]
    assert rows["a"] == ["specmint-core,", "specmint-tdd-html"]
    assert rows["b"] == ["(none)"] and rows["c"] == ["(unknown)"] and rows["d"] == ["(unknown)"]


@pytest.mark.parametrize("json_flag", [True, False])
def test_list_with_an_unknown_brain(sandbox: Sandbox, json_flag):
    v2_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "list", "--brain", "nope", *(["--json"] if json_flag else [])])
    assert r["v2"].code == r["v3"].code == 1
    assert r["v3"].stdout == r["v2"].stdout
    assert r["v3"].stderr == r["v2"].stderr


# --- add ---------------------------------------------------------------------------


@pytest.mark.parametrize("brain", ["a", "b", "all"])
@pytest.mark.parametrize("name", KNOWN)
def test_add_json(sandbox: Sandbox, name, brain):
    v2_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "add", name, "--brain", brain, "--json"])
    assert r["v2"].code == r["v3"].code == 0, (r["v2"].stderr, r["v3"].stderr)
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert_trees(r, pristine(sandbox), set(BRAINS) if brain == "all" else {brain})


def test_add_text_and_an_existing_opt_in(sandbox: Sandbox):
    v2_brains(sandbox, ("specmint-tdd", "a"))
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "add", "specmint-tdd", "--brain", "all"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0
    assert v3.stdout == v2.stdout == "Added companion(s) specmint-tdd to: a, b\n"
    assert_trees(r, pristine(sandbox), set(BRAINS))


def test_add_refreshes_an_old_single_file_install(sandbox: Sandbox):
    v2_brains(sandbox)
    old = sandbox.home / ".kluris" / "companions" / "specmint-core"
    old.mkdir(parents=True)
    (old / "SKILL.md").write_text("old single-file playbook", encoding="utf-8")
    (old.parent / ".specmint-core.old").write_text("stray", encoding="utf-8")
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "add", "specmint-core", "--brain", "a", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    assert_trees(r, pristine(sandbox), {"a"})
    assert "home/.kluris/companions/specmint-core/references/spec-format.md" in r["v3"].tree


# --- remove --------------------------------------------------------------------------


def test_remove_json_from_the_brain_that_has_it(sandbox: Sandbox):
    v2_brains(sandbox, ("specmint-core", "a"), ("specmint-tdd", "a"))
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "remove", "specmint-core", "--brain", "a", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert_trees(r, pristine(sandbox), {"a"})
    assert "home/.kluris/companions/specmint-core/SKILL.md" in r["v3"].tree, "runtime files are kept"


def test_remove_text(sandbox: Sandbox):
    v2_brains(sandbox, ("specmint-core-html", "b"))
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", "remove", "specmint-core-html", "--brain", "b"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0
    assert v3.stdout == v2.stdout == (
        "Removed companion(s) specmint-core-html from: b\n"
        "Runtime companion files were kept under ~/.kluris/companions.\n"
    )
    assert_trees(r, pristine(sandbox), {"b"})


def test_remove_all_touches_only_brains_that_had_it(sandbox: Sandbox):
    """PAR-039 (research-03 §5 #25): v2 rewrites every resolved brain's
    ``kluris.yml`` (here dropping ``b``'s comment) and reports it; v3 only
    the brains that had the companion."""
    brains = v2_brains(sandbox, ("specmint-core", "a"))
    yml = brains / "b" / "kluris.yml"
    yml.write_text("# my notes\n" + yml.read_text(encoding="utf-8"), encoding="utf-8")
    sandbox.freeze()
    before = pristine(sandbox)

    r = run_trees(sandbox, ["companion", "remove", "specmint-core", "--brain", "all", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    j2, j3 = json_of(r["v2"], sandbox), json_of(r["v3"], sandbox)
    assert j2["brains"] == ["a", "b"] and j3["brains"] == ["a"]
    j2["brains"] = j3["brains"]
    assert j3 == j2 and list(j3) == list(j2)
    assert r["v2"].tree["brains/b/kluris.yml"] != before["brains/b/kluris.yml"], "v2 still rewrites b"
    assert_trees(r, before, {"a"})
    assert r["v3"].tree["brains/b/kluris.yml"] == before["brains/b/kluris.yml"]

    r = run_trees(sandbox, ["companion", "remove", "specmint-tdd", "--brain", "all"])
    assert r["v2"].stdout == "Removed companion(s) specmint-tdd from: a, b\n" \
        "Runtime companion files were kept under ~/.kluris/companions.\n"
    assert r["v3"].stdout == "Removed companion(s) specmint-tdd from: (none)\n" \
        "Runtime companion files were kept under ~/.kluris/companions.\n"
    assert_trees(r, before, set())


# --- errors ----------------------------------------------------------------------------


@pytest.mark.parametrize("command", ["add", "remove"])
def test_name_required_when_not_interactive(sandbox: Sandbox, command):
    v2_brains(sandbox)
    sandbox.freeze()
    for args, out, err in (
        (["--brain", "a", "--json"], json.dumps({"ok": False, "error": NAME_REQUIRED}) + "\n", ""),
        (["--brain", "a"], "", f"Error: {NAME_REQUIRED}\n"),
    ):
        r = run_trees(sandbox, ["companion", command, *args])
        for side in ("v2", "v3"):
            assert (r[side].code, r[side].stdout, r[side].stderr) == (1, out, err), side
        assert r["v3"].tree == pristine(sandbox)


@pytest.mark.parametrize("command", ["add", "remove"])
def test_invalid_companion_name(sandbox: Sandbox, command):
    v2_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", command, "bogus", "--brain", "a", "--json"])
    for side in ("v2", "v3"):
        assert r[side].code == 1
        assert r[side].stdout == json.dumps({"ok": False, "error": BAD_CHOICE.format(value="bogus")}) + "\n"
    r = run_trees(sandbox, ["companion", command, "bogus", "--brain", "a"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 2 and v3.stdout == v2.stdout == ""
    assert v3.stderr.splitlines()[-1] == v2.stderr.splitlines()[-1] == "Error: " + BAD_CHOICE.format(value="bogus")
    assert f"Try 'kluris companion {command} --help' for help." in v3.stderr
    assert v3.stderr.startswith(f"Usage: kluris companion {command} [OPTIONS] [specmint-core|")


@pytest.mark.parametrize("args", [
    ["companion", "add", "--brain", "a", "specmint-core", "extra", "--json"],
    ["companion", "list", "x", "--json"],
    ["companion", "bogus", "--json"],
    ["companion", "--json"],
    ["companion", "add", "--brain", "--json"],
    ["companion", "add", "specmint-core", "--json"],
    ["companion", "remove", "specmint-core", "--brain", "all", "--brain", "nope", "--json"],
])
def test_usage_and_resolver_errors_with_json(sandbox: Sandbox, args):
    v2_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, args)
    assert r["v2"].code == r["v3"].code == 1
    assert r["v3"].stdout == r["v2"].stdout
    assert r["v3"].tree == pristine(sandbox)


@pytest.mark.parametrize("args", [
    ["companion", "bogus"],
    ["companion", "list", "x"],
    ["companion", "add", "--brain"],
    ["companion", "add", "specmint-core"],
])
def test_usage_and_resolver_errors(sandbox: Sandbox, args):
    v2_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, args)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == v2.code, (v2.stderr, v3.stderr)
    assert v3.stdout == v2.stdout == ""
    assert v3.stderr.splitlines()[-1] == v2.stderr.splitlines()[-1]


def test_bare_companion_prints_its_help_and_exits_2(sandbox: Sandbox):
    sandbox.freeze()
    r = run_trees(sandbox, ["companion"])
    for side in ("v2", "v3"):
        assert r[side].code == 2 and r[side].stdout == "", side
        assert "Usage: kluris companion" in r[side].stderr
        assert "Manage embedded companion playbooks per brain." in r[side].stderr


# Help keeps v2's wording, not Click's layout (as test_list_status_help's
# test_command_help_flag): checked by substring on both sides.
HELP = {
    (): ["Usage: kluris companion", "Manage embedded companion playbooks per brain.",
         "add", "Opt one or more brains into an embedded companion playbook.",
         "list", "remove", "Remove a companion opt-in from one or more brains.", "--help"],
    ("list",): ["Usage: kluris companion list [OPTIONS]",
                "List known companions, installed runtime copies, and brain opt-ins.",
                "--brain", "Brain name or 'all'", "--json", "JSON output", "Show this message and exit."],
    ("add",): ["Usage: kluris companion add [OPTIONS] [specmint-core|specmint-tdd|specmint-",
               "Opt one or more brains into an embedded companion playbook.",
               "Runs an interactive picker when no NAME is given.", "--brain", "Brain name or 'all'",
               "--json", "JSON output"],
    ("remove",): ["Usage: kluris companion remove [OPTIONS] [specmint-core|specmint-tdd|specmint-",
                  "Remove a companion opt-in from one or more brains.",
                  "The global companion copy under ~/.kluris/companions",
                  "--brain", "Brain name or 'all'", "--json", "JSON output"],
}


@pytest.mark.parametrize("sub", sorted(HELP))
def test_help(sandbox: Sandbox, sub):
    sandbox.freeze()
    r = run_trees(sandbox, ["companion", *sub, "--help"])
    for side in ("v2", "v3"):
        assert r[side].code == 0 and r[side].stderr == "", side
        for text in HELP[sub]:
            assert text in r[side].stdout, (side, text)


@pytest.mark.parametrize("damage", ["missing", "invalid"])
@pytest.mark.parametrize("command", ["add", "remove"])
def test_a_brain_whose_kluris_yml_does_not_read(sandbox: Sandbox, command, damage):
    """PAR-040: v2 rewrites the brains before it, then ends in a traceback;
    v3 stops with a clean error before writing anything."""
    brains = v2_brains(sandbox, ("specmint-core", "a"))
    yml = brains / "b" / "kluris.yml"
    if damage == "missing":
        yml.unlink()
    else:
        yml.write_text("name: [broken\n", encoding="utf-8")
    sandbox.freeze()
    for json_flag in ([], ["--json"]):
        r = run_trees(sandbox, ["companion", command, "specmint-tdd-html", "--brain", "all", *json_flag])
        v2, v3 = r["v2"], r["v3"]
        assert v2.code == 1 and "Traceback (most recent call last)" in v2.stderr
        assert v3.code == 1
        what = "Cannot access config file" if damage == "missing" else "Invalid config file"
        path = normalize(str(yml), sandbox.root)
        if json_flag:
            doc = json_of(v3, sandbox)
            assert doc["ok"] is False and doc["error"].startswith(f"{what} {path}: "), doc
        else:
            assert v3.stdout == ""
            assert normalize(v3.stderr, sandbox.root).startswith(f"Error: {what} {path}: "), v3.stderr
        assert v3.tree == pristine(sandbox), "nothing is written"


# --- the pickers (a terminal on stdin) --------------------------------------------------------

pty_only = pytest.mark.skipif(IS_WINDOWS, reason="the pickers need a pseudo-terminal; Windows has no pty")


def pty_trees(sb: Sandbox, args: list[str], **kw) -> dict[str, tuple]:
    out = {}
    for side in ("v2", "v3"):
        result = run_pty(sb, side, args, **kw)
        stash = max(sb.root.glob(f"{side}-pty-*"), key=lambda p: int(p.name.rsplit("-", 1)[1]))
        out[side] = (result, snapshot_tree(stash))
    return out


def assert_same_run(sb: Sandbox, r: dict[str, tuple], rewritten: set[str] | None) -> None:
    (v2, t2), (v3, t3) = r["v2"], r["v3"]
    assert v3.code == v2.code, (v2, v3)
    assert v3.stderr == v2.stderr, (v2.stderr, v3.stderr)
    same_words(v2.stdout, v3.stdout, sb.root)
    if rewritten is None:
        assert t3 == pristine(sb)
    else:
        fake = {"v2": Result(0, "", "", t2), "v3": Result(0, "", "", t3)}
        assert_trees(fake, pristine(sb), rewritten)


ADD_MENU_PROMPT = "    [7] all\n  Choice [4]: "


@pty_only
@pytest.mark.parametrize("lines,names", [
    (["3"], ["specmint-core", "specmint-tdd"]),
    (["9", "", "x"], []),
    (["", "7"], []),
    (["7"], KNOWN),
    (["6"], ["specmint-tdd-html"]),
    ([" 3", "5"], ["specmint-core-html"]),
])
def test_add_picker(sandbox: Sandbox, lines, names):
    v2_brains(sandbox)
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "add", "--brain", "a"], lines=lines)
    assert_same_run(sandbox, r, {"a"} if names else None)
    v3 = r["v3"][0].stdout
    assert ADD_MENU_PROMPT in v3
    if names:
        assert v3.endswith(f"Added companion(s) {', '.join(names)} to: a\n"), v3
    else:
        assert v3.endswith("  Choice [4]: No companion selected. Nothing to do.\n"), v3
    if "9" in lines:
        assert "  Choice [4]: Error: '9' is not one of '1', '2', '3', '4', '5', '6', '7'.\n" in v3


@pty_only
def test_add_picker_after_the_brain_picker(sandbox: Sandbox):
    v2_brains(sandbox)
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "add"], lines=["3", "2"])
    assert_same_run(sandbox, r, set(BRAINS))
    v3 = r["v3"][0].stdout
    assert "  [3] all\nChoice:   Install specmint companions for this brain?\n" in v3
    assert v3.endswith("Added companion(s) specmint-tdd to: a, b\n")


@pty_only
def test_add_picker_end_of_input(sandbox: Sandbox):
    v2_brains(sandbox)
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "add", "--brain", "a"], eof=True)
    assert_same_run(sandbox, r, None)
    assert r["v3"][0].code == 1 and r["v3"][0].stderr == "Aborted!\n"


@pty_only
@pytest.mark.parametrize("lines,removed", [
    ([""], True),
    (["y"], True),
    (["junk", " YES "], True),
    (["n"], False),
    (["No"], False),
])
def test_remove_confirm_with_one_enabled(sandbox: Sandbox, lines, removed):
    v2_brains(sandbox, ("specmint-tdd", "a"))
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "remove", "--brain", "a"], lines=lines)
    assert_same_run(sandbox, r, {"a"} if removed else None)
    v3 = r["v3"][0].stdout
    assert v3.startswith("  Remove specmint-tdd? [Y/n]: ")
    if "junk" in lines:
        assert "Error: invalid input\n" in v3
    tail = ("Removed companion(s) specmint-tdd from: a\n"
            "Runtime companion files were kept under ~/.kluris/companions.\n"
            if removed else "No companion selected. Nothing to do.\n")
    assert v3.endswith(tail), v3


@pty_only
@pytest.mark.parametrize("lines,names", [
    (["2"], ["specmint-tdd-html"]),
    (["0", "x", "", "3"], ["specmint-core", "specmint-tdd-html"]),
    (["4"], []),
])
def test_remove_picker_with_several_enabled(sandbox: Sandbox, lines, names):
    v2_brains(sandbox, ("specmint-core", "a"), ("specmint-tdd-html", "b"))
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "remove", "--brain", "all"], lines=lines)
    v3 = r["v3"][0].stdout
    assert v3.startswith(
        "  Which companion(s) to remove?\n    [1] specmint-core\n    [2] specmint-tdd-html\n"
        "    [3] all\n    [4] cancel\n  Choice: "
    ), v3
    if "0" in lines:
        assert "  Choice: Error: 0 is not in the range 1<=x<=4.\n" in v3
        assert "  Choice: Error: 'x' is not a valid integer range.\n" in v3
    if not names:
        assert_same_run(sandbox, r, None)
        assert v3.endswith("No companion selected. Nothing to do.\n")
        return
    # PAR-039: only the brains that had a removed companion change in v3.
    had = {"specmint-core": "a", "specmint-tdd-html": "b"}
    changed = sorted({had[n] for n in names})
    assert v3.endswith(f"Removed companion(s) {', '.join(names)} from: {', '.join(changed)}\n"
                       "Runtime companion files were kept under ~/.kluris/companions.\n"), v3
    (v2, t2), (v3r, t3) = r["v2"], r["v3"]
    assert v3r.code == v2.code == 0
    fake = {"v2": Result(0, "", "", t2), "v3": Result(0, "", "", t3)}
    before = pristine(sandbox)
    assert_trees(fake, before, set(changed))


@pty_only
def test_remove_picker_with_nothing_enabled(sandbox: Sandbox):
    v2_brains(sandbox)
    sandbox.freeze()
    r = pty_trees(sandbox, ["companion", "remove", "--brain", "all"])
    assert_same_run(sandbox, r, None)
    assert r["v3"][0].stderr == "Error: No companions are enabled on the selected brain(s).\n"
