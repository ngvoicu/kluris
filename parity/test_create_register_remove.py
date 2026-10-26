"""``kluris create|register|remove`` against the v2.28.11 oracle (research-03
§1.1, §1.2, §1.12, §1.15, §2.2).

Compared, after root normalization:

- JSON byte for byte, exit codes, Kluris's own error messages exactly, human
  output by v2's wording (``same_words``);
- the whole ``work/`` tree byte for byte: every scaffolded brain file (each
  ``--type``, ``--from-config``), ``config.yml``, and HOME, with the
  decided differences applied to v2's tree: a ``kluris.yml`` v3 writes has
  PAR-003's agent names, and the skills are v2's SKILL.md with the decided
  changes in v3's folders (PAR-026 ... PAR-031, via ``test_skill_install``);
- each new brain's git state: ``HEAD``, its branch, every path and blob of
  the first commit, author and committer, the remote (dates and identity are
  pinned, so the commit hashes are equal too).

The wizard and the pickers need a terminal on stdin: those tests drive both
binaries through a pseudo-terminal (stdout and stderr stay pipes) and are
skipped on Windows, which has no ``pty``.

Allowlisted (``parity/allowlist.md``): the JSON error for ``create --json``
without a name, the wizard's brain type checked like ``--type``, no prompt
without a terminal or with ``KLURIS_NO_PROMPT=1`` (``create`` and
``register``), ``--from-config`` lobe names and nested values, the stricter
rule for new brain names, a git failure that leaves the brain unregistered,
and a target path that is a file.
"""

from __future__ import annotations

import json
import os
import select
import shutil
import subprocess
import time
from pathlib import Path

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from harness import (
    IS_WINDOWS, Result, Sandbox, _decode, assert_json_equal, normalize, oracle_kluris, run_v2, snapshot_tree,
)
from runs import _env, _exe, same_words
from test_skill_install import V2_ONLY, _v2_to_v3

TYPES = {"product-group": 3, "personal": 3, "product": 6, "research": 6, "blank": 0}
# Skill folders v3 writes for the default agents (no Hermes profiles here).
V3_SKILL_DIRS = [".agents/skills", ".claude/skills", ".hermes/skills", ".gemini/config/skills"]
NAME_EXAMPLES = int(os.environ.get("KLURIS_NAME_FUZZ_EXAMPLES", "2000"))
pty_only = pytest.mark.skipif(IS_WINDOWS, reason="the wizard needs a pseudo-terminal; Windows has no pty")

ZIP_HINT = (
    "kluris register no longer accepts .zip files. Unzip first, then run 'kluris register <directory>'. "
    "For git-hosted brains, use 'git clone <url> <path>' then 'kluris register <path>'."
)
NAME_REQUIRED = "Brain name is required in non-interactive mode. Run 'kluris create <name>'."
PATH_REQUIRED = "Brain path is required in non-interactive mode. Run 'kluris register <path>'."


def at(rel: str) -> str:
    """``<ROOT>`` + ``rel`` (``/``-separated) as the OS prints a path."""
    return "<ROOT>" + (rel.replace("/", "\\") if IS_WINDOWS else rel)


@pytest.fixture
def sandbox(tmp_path, oracle):
    return Sandbox(tmp_path)


# --- running ------------------------------------------------------------------


def git_state(repo: Path, env: dict[str, str]) -> dict | None:
    """What the first commit holds and how the repo is set up; ``None``
    without a repository."""
    if not (repo / ".git").exists():
        return None

    def git(*args: str) -> str:
        proc = subprocess.run(["git", *args], cwd=repo, env=env, capture_output=True, text=True)
        return proc.stdout if proc.returncode == 0 else f"<exit {proc.returncode}> {proc.stderr.strip()}"

    head, tree = git("rev-parse", "HEAD"), git("ls-tree", "-r", "HEAD")
    if IS_WINDOWS:
        # v2 writes the files CRLF there and v3 LF (PAR-035), so the blob and
        # commit hashes differ; the paths, modes and the rest must not.
        head = "<HEAD>"
        tree = "".join(f"{line.split(' ', 2)[0]} {line.split(chr(9), 1)[-1]}\n" for line in tree.splitlines())
    return {
        "head": head,
        "branch": git("symbolic-ref", "HEAD"),
        "tree": tree,
        "log": git("log", "--format=%an <%ae>|%cn <%ce>|%aI|%cI|%s"),
        "remote": git("remote", "-v"),
        "status": git("status", "--porcelain", "--ignored"),
    }


def _finish(sb: Sandbox, result: Result, git: tuple[str, ...], env: dict[str, str]) -> Result:
    result.tree = snapshot_tree(sb.work)
    result.git = {rel: git_state(sb.work / rel, env) for rel in git}  # type: ignore[attr-defined]
    return result


def run_trees(sb: Sandbox, args: list[str], *, env: dict[str, str | None] | None = None,
              stdin: bytes | None = b"", git: tuple[str, ...] = (), sides=("v2", "v3")) -> dict[str, Result]:
    """Each binary on its own copy of the frozen state, stdin a pipe; every
    result carries the ``work/`` tree and the git state of each ``git``
    directory (relative to ``work/``) after its run."""
    results: dict[str, Result] = {}
    run_env = _env(sb, env or {})
    for side in sides:
        sb.checkout()
        try:
            proc = subprocess.run(
                [*_exe(side), *args], env=run_env, cwd=str(sb.work),
                input=stdin, capture_output=True, timeout=120,
            )
            what = f"{side} {' '.join(args)}"
            result = Result(proc.returncode, _decode(proc.stdout, f"stdout of `{what}`"),
                            _decode(proc.stderr, f"stderr of `{what}`"))
            results[side] = _finish(sb, result, git, run_env)
        finally:
            sb.stash(f"{side}-{time.monotonic_ns()}")
    return results


def run_pty(sb: Sandbox, side: str, args: list[str], lines: list[str], *, eof: bool = True,
            env: dict[str, str | None] | None = None, git: tuple[str, ...] = ()) -> Result:
    """One binary with stdin a pseudo-terminal (stdout and stderr pipes) on a
    fresh copy of the frozen state; ``lines`` are typed one per line, then
    Ctrl-D when ``eof``. POSIX only."""
    import pty

    changes: dict[str, str | None] = {"KLURIS_NO_PROMPT": None}
    changes.update(env or {})
    run_env = _env(sb, changes)
    sb.checkout()
    master, slave = pty.openpty()
    try:
        proc = subprocess.Popen(
            [*_exe(side), *args], env=run_env, cwd=str(sb.work),
            stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        os.close(slave)
        slave = -1
        typed = b"".join(line.encode("utf-8") + b"\n" for line in lines)
        try:
            os.write(master, typed + (b"\x04" if eof else b""))
        except OSError:
            pass  # the process is gone and the terminal closed
        # Drain the terminal so the line discipline never blocks the child.
        out, err = b"", b""
        deadline = time.monotonic() + 60
        while proc.poll() is None and time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                try:
                    os.read(master, 65536)
                except OSError:
                    pass
        rest, err = proc.communicate(timeout=60)
        out += rest
        result = Result(proc.returncode, _decode(out, f"stdout of {side}"), _decode(err, f"stderr of {side}"))
        return _finish(sb, result, git, run_env)
    finally:
        if slave >= 0:
            os.close(slave)
        os.close(master)
        sb.stash(f"{side}-pty-{time.monotonic_ns()}")


def pty_both(sb: Sandbox, args: list[str], lines: list[str], **kw) -> dict[str, Result]:
    return {side: run_pty(sb, side, args, lines, **kw) for side in ("v2", "v3")}


def oracle(sb: Sandbox, *args: str, cwd: Path | None = None) -> dict:
    r = run_v2(list(args), sb.env, cwd or sb.work)
    assert r.code == 0, f"oracle {args} failed: {r.stdout} {r.stderr}"
    return json.loads(r.stdout) if "--json" in args else {}


def pristine(sb: Sandbox) -> dict[str, bytes]:
    return snapshot_tree(sb.root / "pristine")


def json_of(result: Result, sb: Sandbox) -> dict:
    text = normalize(result.stdout, sb.root)
    doc = json.loads(text)
    assert text == json.dumps(doc) + "\n", text
    return doc


# --- the expected v3 tree -----------------------------------------------------------


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


def expected_v3(v2_tree: dict[str, bytes], rewritten_yml: set[str] = frozenset(),
                v3_yml: dict[str, bytes] | None = None) -> dict[str, bytes]:
    """The tree v3 should leave, from v2's: every brain's skills per
    PAR-026...PAR-031 (v2's copies migrated), and each ``kluris.yml`` in
    ``rewritten_yml`` (paths relative to ``work/``) with PAR-003's names."""
    exp = dict(v2_tree)
    prefix, suffix = "home/.claude/skills/kluris-", "/SKILL.md"
    skills = [k[len(prefix):-len(suffix)] for k in v2_tree if k.startswith(prefix) and k.endswith(suffix)]
    for brain in skills:
        v2_skill = v2_tree[f"{prefix}{brain}{suffix}"].decode("utf-8").replace("\r\n", "\n")
        v3_skill = _v2_to_v3(v2_skill, brain).encode("utf-8")
        for rel in V3_SKILL_DIRS:
            exp[f"home/{rel}/kluris-{brain}/SKILL.md"] = v3_skill
        for rel in V2_ONLY:
            exp.pop(f"home/{rel}/kluris-{brain}/SKILL.md", None)
        exp.pop(f"home/.codeium/windsurf/global_workflows/kluris-{brain}.md", None)
    for key in rewritten_yml:
        exp[key] = par003(v2_tree[key])
    exp.update(v3_yml or {})
    return exp


def _lf(tree: dict[str, bytes]) -> dict[str, bytes]:
    return {k: v.replace(b"\r\n", b"\n") for k, v in tree.items()}


def assert_tree(v3_tree: dict[str, bytes], exp: dict[str, bytes]) -> None:
    if IS_WINDOWS:
        # v2 writes text files CRLF there, v3 creates them LF and keeps an
        # existing file's endings (PAR-005, PAR-029, PAR-035).
        v3_tree, exp = _lf(v3_tree), _lf(exp)
    changed = sorted(k for k in set(exp) | set(v3_tree) if exp.get(k) != v3_tree.get(k))
    assert not changed, {k: (v3_tree.get(k, b"<none>")[:300], exp.get(k, b"<none>")[:300]) for k in changed[:6]}


def assert_same_run(r: dict[str, Result], sb: Sandbox, *, yml: set[str] = frozenset(),
                    json_out: bool = True) -> None:
    """Both succeeded with the same output, tree and git state."""
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 0, (v2.stdout, v2.stderr)
    assert v3.code == 0, (v3.stdout, v3.stderr)
    if json_out:
        assert_json_equal(v2.stdout, v3.stdout, sb.root)
    else:
        same_words(v2.stdout, v3.stdout, sb.root)
    assert v3.stderr == v2.stderr
    assert_tree(v3.tree, expected_v3(v2.tree, yml))
    assert v3.git == v2.git  # type: ignore[attr-defined]


def assert_same_error(r: dict[str, Result], sb: Sandbox, code: int = 1) -> None:
    """Both failed the same way and wrote nothing."""
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == code, (v2.stdout, v2.stderr)
    assert v3.code == code, (v3.stdout, v3.stderr)
    assert normalize(v3.stdout, sb.root) == normalize(v2.stdout, sb.root)
    assert normalize(v3.stderr, sb.root) == normalize(v2.stderr, sb.root)
    assert v2.tree == pristine(sb), "v2 wrote something"
    assert v3.tree == pristine(sb), "v3 wrote something"


# --- create: scaffolds ------------------------------------------------------------------

DESCRIPTION = "Dëmo \"brain\" — it's ünïcode"


@pytest.mark.parametrize("git", [True, False], ids=["git", "no-git"])
@pytest.mark.parametrize("brain_type", list(TYPES))
def test_create_every_type(sandbox: Sandbox, brain_type, git):
    (sandbox.work / "brains").mkdir()
    sandbox.freeze()
    args = ["create", "demo", "--path", "brains", "--type", brain_type, "--description", DESCRIPTION,
            *([] if git else ["--no-git"]), "--json"]
    r = run_trees(sandbox, args, git=("brains/demo",))
    assert_same_run(r, sandbox, yml={"brains/demo/kluris.yml"})
    assert json_of(r["v3"], sandbox)["lobes"] == TYPES[brain_type]
    files = {k for k in r["v3"].tree if k.startswith("brains/demo/")}
    assert {"brains/demo/README.md", "brains/demo/.gitignore", "brains/demo/glossary.md",
            "brains/demo/brain.md", "brains/demo/kluris.yml"} <= files
    state = r["v3"].git["brains/demo"]
    assert (state is not None) == git
    if git:
        assert "brain: initialize demo" in state["log"] and "kluris.yml" not in state["tree"]


def test_create_defaults_with_json(sandbox: Sandbox):
    """No ``--path``, ``--type`` or ``--description``: ``<cwd>/<name>``, the
    product-group lobes, ``<name> knowledge base``."""
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "solo", "--json"], git=("solo",))
    assert_same_run(r, sandbox, yml={"solo/kluris.yml"})
    assert b"solo knowledge base" in r["v3"].tree["solo/brain.md"]


def test_create_text_output(sandbox: Sandbox):
    (sandbox.work / "brains").mkdir()
    sandbox.freeze()
    args = ["create", "demo", "--path", "brains", "--type", "personal", "--description", "d", "--no-git"]
    r = run_trees(sandbox, args)
    assert_same_run(r, sandbox, yml={"brains/demo/kluris.yml"}, json_out=False)
    for text in ("Brain created: demo (personal)", "Lobes: 3", "Agent skill installed: kluris-demo",
                 "Claude/Codex/Gemini/etc: /kluris-demo", "Hermes: /skill kluris-demo",
                 "(or start with `hermes -s kluris-demo`)",
                 "Created a new Hermes profile later? Run `kluris doctor` to refresh Kluris skills."):
        assert text in r["v3"].stdout, text


@pytest.mark.parametrize("extra", [
    ["--branch", "dev"],
    ["--remote", "git@example.com:team/brain.git"],
    ["--remote", "https://example.com/brain.git", "--branch", "trunk"],
    ["--branch", "main"],
    ["--remote", "git@example.com:team/brain.git", "--no-git"],
], ids=["branch", "remote", "remote-branch", "branch-main", "remote-no-git"])
def test_create_git_setup(sandbox: Sandbox, extra):
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "demo", "--type", "blank", *extra, "--json"], git=("demo",))
    assert_same_run(r, sandbox, yml={"demo/kluris.yml"})


def test_create_commit_identity_fallback(sandbox: Sandbox):
    """Without a git identity the first commit is ``kluris <kluris@local>``."""
    sandbox.freeze()
    no_identity = {k: None for k in ("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL",
                                     "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL")}
    r = run_trees(sandbox, ["create", "demo", "--json"], env=no_identity, git=("demo",))
    assert_same_run(r, sandbox, yml={"demo/kluris.yml"})
    assert r["v3"].git["demo"]["log"].startswith("kluris <kluris@local>|kluris <kluris@local>|")


def test_create_keeps_a_git_identity(sandbox: Sandbox):
    """With ``user.name``/``user.email`` in git config and no identity in the
    environment, the commit is by that identity."""
    (sandbox.root / "gitconfig-empty").write_text("[user]\n\tname = Ada\n\temail = ada@example.com\n")
    sandbox.freeze()
    no_identity = {k: None for k in ("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL",
                                     "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL")}
    r = run_trees(sandbox, ["create", "demo", "--json"], env=no_identity, git=("demo",))
    assert_same_run(r, sandbox, yml={"demo/kluris.yml"})
    assert r["v3"].git["demo"]["log"].startswith("Ada <ada@example.com>|")


def test_create_into_an_existing_empty_directory(sandbox: Sandbox):
    (sandbox.work / "brains" / "demo").mkdir(parents=True)
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "demo", "--path", "brains", "--json"], git=("brains/demo",))
    assert_same_run(r, sandbox, yml={"brains/demo/kluris.yml"})


def test_create_second_brain_installs_both_skills(sandbox: Sandbox):
    oracle(sandbox, "create", "first", "--no-git", "--json")
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "second", "--no-git", "--json"])
    assert_same_run(r, sandbox, yml={"second/kluris.yml"})
    for brain in ("first", "second"):
        assert f"home/.agents/skills/kluris-{brain}/SKILL.md" in r["v3"].tree


# --- create: validation -------------------------------------------------------------------


def validation_scenario(sb: Sandbox) -> None:
    """A registered brain ``taken`` (v2), a file, a non-empty directory, an
    empty one and one holding a ``kluris.yml``."""
    brains = sb.work / "brains"
    brains.mkdir()
    oracle(sb, "create", "taken", "--path", str(brains), "--no-git", "--json")
    (sb.work / "afile").write_text("x", encoding="utf-8")
    (brains / "full").mkdir()
    (brains / "full" / "notes.md").write_text("x", encoding="utf-8")
    (brains / "empty").mkdir()
    (brains / "hasyml").mkdir()
    (brains / "hasyml" / "kluris.yml").write_text("name: x\n", encoding="utf-8")
    (brains / "outer").mkdir()
    (brains / "outer" / "kluris.yml").write_text("name: outer\n", encoding="utf-8")
    sb.freeze()


ERRORS = {
    "reserved": (["all"], "'all' is a reserved brain name (used by --brain all). Pick a different name."),
    "too-long": (["a" * 49], f"Brain name '{'a' * 49}' is too long (49 chars). Maximum is 48."),
    "upper": (["Bad"], "Brain name 'Bad' is invalid. Use lowercase letters, numbers, and hyphens only."),
    "digit-first": (["9x"], "Brain name '9x' is invalid. Use lowercase letters, numbers, and hyphens only."),
    "dot": ([".x"], "Brain name '.x' is invalid. Use lowercase letters, numbers, and hyphens only."),
    "empty": ([""], "Brain name '' is invalid. Use lowercase letters, numbers, and hyphens only."),
    "registered": (["taken"], f"A brain named 'taken' is already registered at {at('/work/brains/taken')}. "
                              "Choose a different name."),
    "path-file": (["new", "--path", "afile"], "--path 'afile' is not a directory."),
    "path-missing": (["new", "--path", "no/such"], "--path 'no/such' is not a directory."),
    "path-brain": (["new", "--path", "brains/taken"],
                   "--path 'brains/taken' is already a brain. Pass the parent directory, not the brain itself."),
    "inside-brain": (["inner", "--path", "brains/taken/projects"],
                     f"Target path {at('/work/brains/taken/projects/inner')} is inside existing brain "
                     f"'{at('/work/brains/taken')}'. Create the brain outside of other brains."),
    "not-empty": (["full", "--path", "brains"],
                  f"{at('/work/brains/full')} already exists and is not empty. "
                  "Choose a different name or use --path to specify a location."),
    "has-yml": (["hasyml", "--path", "brains"],
                f"{at('/work/brains/hasyml')} already contains a kluris.yml. Use a different name."),
    "order-reserved": (["all", "--path", "afile"],
                       "'all' is a reserved brain name (used by --brain all). Pick a different name."),
    "order-registered": (["taken", "--path", "afile"],
                         f"A brain named 'taken' is already registered at {at('/work/brains/taken')}. "
                         "Choose a different name."),
    "order-path": (["inner", "--path", "brains/taken"],
                   "--path 'brains/taken' is already a brain. Pass the parent directory, not the brain itself."),
}


@pytest.mark.parametrize("case", list(ERRORS))
def test_create_validation_json(sandbox: Sandbox, case):
    validation_scenario(sandbox)
    args, message = ERRORS[case]
    r = run_trees(sandbox, ["create", *args, "--json"])
    assert_same_error(r, sandbox)
    assert json_of(r["v3"], sandbox) == {"ok": False, "error": message}


@pytest.mark.parametrize("case", list(ERRORS))
def test_create_validation_text(sandbox: Sandbox, case):
    """Every value given, so v2 asks nothing."""
    validation_scenario(sandbox)
    args, message = ERRORS[case]
    full = ["--description", "d", "--type", "blank", "--no-git"]
    if "--path" not in args:
        full += ["--path", "brains"]
    if case == "empty":
        pytest.skip("an empty NAME is a missing one in text mode (the wizard asks for it)")
    r = run_trees(sandbox, ["create", *args, *full])
    assert_same_error(r, sandbox)
    assert normalize(r["v3"].stderr, sandbox.root) == f"Error: {message}\n"


@pytest.mark.parametrize("args,code", [
    (["create", "x", "--type", "bogus"], 2),
    (["create", "x", "--type", "bogus", "--json"], 1),
    (["create", "x", "--from-config", "nope.yml"], 2),
    (["create", "x", "--from-config", "nope.yml", "--json"], 1),
    (["create", "x", "--type", "bogus", "--type", "blank", "--no-git", "--json"], 0),
], ids=["type", "type-json", "from-config", "from-config-json", "last-type-wins"])
def test_create_option_values(sandbox: Sandbox, args, code):
    sandbox.freeze()
    r = run_trees(sandbox, args)
    if code == 0:
        assert_same_run(r, sandbox, yml={"x/kluris.yml"})
    else:
        assert_same_error(r, sandbox, code)


# --- create: --from-config ------------------------------------------------------------------


FROM_CONFIG = {
    "flat": "structure:\n  alpha-beta: First lobe\n  zeta: Ünïcode — dash\n  mid-lobe-name: Mid\n",
    "no-structure": "other: 1\n",
    "empty-file": "",
    "empty-structure": "structure: {}\n",
    "number-value": "structure:\n  stats: 42\n",
}


@pytest.mark.parametrize("case", list(FROM_CONFIG))
def test_create_from_config(sandbox: Sandbox, case):
    (sandbox.work / "structure.yml").write_text(FROM_CONFIG[case], encoding="utf-8")
    sandbox.freeze()
    args = ["create", "demo", "--type", "research", "--from-config", "structure.yml", "--json"]
    r = run_trees(sandbox, args, git=("demo",))
    assert_same_run(r, sandbox, yml={"demo/kluris.yml"})
    assert json_of(r["v3"], sandbox)["lobes"] == TYPES["research"], "v2 counts the --type lobes"


@pytest.mark.parametrize("text,message", [
    ("structure:\n  ../escape: x\n", "lobe name '../escape'"),
    ("structure:\n  a/b: x\n", "lobe name 'a/b'"),
    ("structure:\n  a\\b: x\n", "lobe name 'a\\\\b'"),
    ("structure:\n  /abs: x\n", "lobe name '/abs'"),
    ("structure:\n  ..: x\n", "lobe name '..'"),
    ("structure:\n  lobe:\n    sub: x\n", "lobe 'lobe'"),
    ("structure:\n  lobe: [a, b]\n", "lobe 'lobe'"),
], ids=["dotdot", "slash", "backslash", "absolute", "parent", "nested-map", "nested-list"])
def test_from_config_refuses_paths_and_nested_values(sandbox: Sandbox, text, message):
    """v2 writes ``../escape`` outside the brain and a nested value as a
    Python repr; v3 refuses before writing anything (allowlisted)."""
    (sandbox.work / "structure.yml").write_text(text, encoding="utf-8")
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "demo", "--from-config", "structure.yml", "--json"], sides=("v3",))
    v3 = r["v3"]
    assert v3.code == 1, (v3.stdout, v3.stderr)
    doc = json_of(v3, sandbox)
    assert doc["ok"] is False and doc["error"].startswith("--from-config: "), doc
    assert message in doc["error"], doc
    assert v3.tree == pristine(sandbox)


def test_from_config_escape_is_v2_damage(sandbox: Sandbox):
    """Guards the allowlist entry: v2 really writes outside the brain."""
    (sandbox.work / "structure.yml").write_text("structure:\n  ../escape: x\n", encoding="utf-8")
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "demo", "--from-config", "structure.yml", "--no-git", "--json"],
                  sides=("v2",))
    assert r["v2"].code == 0
    assert "escape/map.md" in r["v2"].tree


# --- create: the wizard (a terminal) -----------------------------------------------------------


def wizard_transcript(stdout: str) -> str:
    """The prompts: everything up to the last ``Choice [4]: ``, or without
    the picker up to the result (human output, compared by wording only)."""
    at = stdout.rfind("Choice [4]: ")
    if at >= 0:
        return stdout[: at + len("Choice [4]: ")]
    ends = [i for i in (stdout.find("Brain created:"), stdout.find("Brain registered:")) if i >= 0]
    return stdout[: min(ends)] if ends else stdout


def assert_wizard(r: dict[str, Result], sb: Sandbox, yml: set[str]) -> None:
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 0, (v2.stdout, v2.stderr)
    assert v3.code == 0, (v3.stdout, v3.stderr)
    assert normalize(wizard_transcript(v3.stdout), sb.root) == normalize(wizard_transcript(v2.stdout), sb.root)
    same_words(v2.stdout, v3.stdout, sb.root)
    assert v3.stderr == v2.stderr == ""
    assert_tree(v3.tree, expected_v3(v2.tree, yml))
    assert v3.git == v2.git  # type: ignore[attr-defined]


@pty_only
def test_wizard_full(sandbox: Sandbox):
    """Every prompt, empty answers re-asked, answers kept unstripped, remote
    and branch, and the companion picker (an invalid choice re-asked)."""
    sandbox.freeze()
    lines = ["", "wiz", "", " my desc ", "", "personal", "2", "", "git@x:y.git", "dev", "9", "3"]
    r = pty_both(sandbox, ["create"], lines, git=("wiz",))
    assert_wizard(r, sandbox, {"wiz/kluris.yml"})
    out = normalize(r["v3"].stdout, sandbox.root)
    assert out.startswith(
        "\nCreate a new brain\n\n"
        "  Brain name (lowercase, hyphens ok):   Brain name (lowercase, hyphens ok): "
        "  What does this brain cover? (one sentence):   What does this brain cover? (one sentence): "
        "  Location [<ROOT>/work]: "
        "  Brain type (product-group, personal, product, research, blank) [product-group]: "
        "  Git setup (1=local, 2=with remote, 3=no git) [1]: "
        "  Git remote URL:   Git remote URL:   Default branch [main]: \n"
        "  Install specmint companions for this brain?\n"
    ), out
    assert "home/.kluris/companions/specmint-tdd/SKILL.md" in r["v3"].tree


@pytest.mark.parametrize("lines", [
    ["wiz", "d", "", "", "3", "4"],
    ["wiz", "d", "", "blank", "1", "", "7"],
    ["wiz", "d", "", "research", "x", "trunk", ""],
], ids=["no-git", "local-all-companions", "other-choice-is-local"])
@pty_only
def test_wizard_git_choices(sandbox: Sandbox, lines):
    sandbox.freeze()
    r = pty_both(sandbox, ["create"], lines, git=("wiz",))
    assert_wizard(r, sandbox, {"wiz/kluris.yml"})


@pty_only
def test_wizard_asks_only_what_is_missing(sandbox: Sandbox):
    (sandbox.work / "brains").mkdir()
    sandbox.freeze()
    args = ["create", "demo", "--description", "d", "--path", "brains", "--type", "blank"]
    r = pty_both(sandbox, args, ["", "", "5"], git=("brains/demo",))
    assert_wizard(r, sandbox, {"brains/demo/kluris.yml"})
    assert normalize(r["v3"].stdout, sandbox.root).startswith(
        "  Git setup (1=local, 2=with remote, 3=no git) [1]:   Default branch [main]: \n"
        "  Install specmint companions"
    )


@pty_only
def test_wizard_with_every_value_given_asks_nothing(sandbox: Sandbox):
    sandbox.freeze()
    args = ["create", "demo", "--description", "d", "--path", ".", "--type", "blank", "--no-git"]
    r = pty_both(sandbox, args, [])
    assert_wizard(r, sandbox, {"demo/kluris.yml"})
    assert "Install specmint" not in r["v3"].stdout


@pty_only
def test_wizard_end_of_input_aborts(sandbox: Sandbox):
    sandbox.freeze()
    r = pty_both(sandbox, ["create"], ["wiz"])
    for side in ("v2", "v3"):
        assert r[side].code == 1 and r[side].stderr == "Aborted!\n", r[side]
    assert r["v3"].stdout == r["v2"].stdout
    assert r["v3"].tree == r["v2"].tree == pristine(sandbox)


@pty_only
def test_wizard_validates_after_the_prompts(sandbox: Sandbox):
    sandbox.freeze()
    r = pty_both(sandbox, ["create"], ["Bad", "d", "", "", "3"])
    for side in ("v2", "v3"):
        assert r[side].code == 1, r[side]
        assert r[side].stderr == ("Error: Brain name 'Bad' is invalid. "
                                  "Use lowercase letters, numbers, and hyphens only.\n")
    assert r["v3"].stdout == r["v2"].stdout
    assert r["v3"].tree == pristine(sandbox)


@pty_only
def test_wizard_brain_type_is_checked_like_the_option(sandbox: Sandbox):
    """v2 takes any text (``foo`` scaffolds a blank brain reported as
    ``(foo)``); v3 asks again with ``--type``'s message (allowlisted)."""
    sandbox.freeze()
    v2 = run_pty(sandbox, "v2", ["create"], ["wiz", "d", "", "foo", "3", "4"])
    assert v2.code == 0 and "Brain created: wiz (foo)" in v2.stdout
    v3 = run_pty(sandbox, "v3", ["create"], ["wiz", "d", "", "foo", "Personal", "personal", "3", "4"])
    assert v3.code == 0, (v3.stdout, v3.stderr)
    prompt = "  Brain type (product-group, personal, product, research, blank) [product-group]: "
    choices = "is not one of 'product-group', 'personal', 'product', 'research', 'blank'."
    assert f"{prompt}Error: 'foo' {choices}\n{prompt}Error: 'Personal' {choices}\n{prompt}" in v3.stdout
    assert "Brain created: wiz (personal)" in v3.stdout
    assert "wiz/projects/map.md" in v3.tree and "wiz/tasks/map.md" in v3.tree


# --- create: no terminal (allowlisted) -------------------------------------------------------------


def test_create_without_a_terminal_uses_the_defaults(sandbox: Sandbox):
    """v2 prompts for the description from the pipe and aborts; v3 asks
    nothing and creates what ``--json`` would."""
    sandbox.freeze()
    v2 = run_trees(sandbox, ["create", "demo"], sides=("v2",))["v2"]
    assert v2.code == 1 and v2.stderr.endswith("Aborted!\n")
    v2_json = run_trees(sandbox, ["create", "demo", "--json"], git=("demo",), sides=("v2",))["v2"]
    v3 = run_trees(sandbox, ["create", "demo"], git=("demo",), sides=("v3",))["v3"]
    assert v3.code == 0, (v3.stdout, v3.stderr)
    assert "Brain created: demo (product-group)" in v3.stdout
    assert_tree(v3.tree, expected_v3(v2_json.tree, {"demo/kluris.yml"}))
    assert v3.git == v2_json.git  # type: ignore[attr-defined]


@pytest.mark.parametrize("json_flag", [False, True], ids=["text", "json"])
def test_create_without_a_name_and_no_terminal_is_an_error(sandbox: Sandbox, json_flag):
    """v2 aborts at the name prompt (text) or raises a TypeError traceback
    (``--json``); v3 names the missing value."""
    sandbox.freeze()
    args = ["create", *(["--json"] if json_flag else [])]
    r = run_trees(sandbox, args)
    assert r["v2"].code == 1
    assert ("Traceback" in r["v2"].stderr) if json_flag else r["v2"].stderr.endswith("Aborted!\n")
    v3 = r["v3"]
    assert v3.code == 1
    if json_flag:
        assert v3.stdout == json.dumps({"ok": False, "error": NAME_REQUIRED}) + "\n" and v3.stderr == ""
    else:
        assert v3.stderr == f"Error: {NAME_REQUIRED}\n"
    assert v3.tree == pristine(sandbox)


@pty_only
def test_create_with_no_prompt_set_asks_nothing_on_a_terminal(sandbox: Sandbox):
    sandbox.freeze()
    v3 = run_pty(sandbox, "v3", ["create", "demo", "--no-git"], [], env={"KLURIS_NO_PROMPT": "1"})
    assert v3.code == 0, (v3.stdout, v3.stderr)
    assert "Brain name" not in v3.stdout and "What does this brain cover" not in v3.stdout
    assert "Install specmint" not in v3.stdout
    assert b"demo knowledge base" in v3.tree["demo/brain.md"]


# --- create: the stricter name rule (allowlisted) ------------------------------------------------------


@pytest.mark.parametrize("name", ["a--b", "a-", "ab-", "x--", "a-b--c"])
def test_new_names_follow_the_stricter_rule(sandbox: Sandbox, name):
    sandbox.freeze()
    r = run_trees(sandbox, ["create", name, "--no-git", "--json"])
    assert r["v2"].code == 0, "v2 accepts it"
    v3 = r["v3"]
    assert v3.code == 1
    error = json_of(v3, sandbox)["error"]
    assert error.startswith(f"Brain name '{name}' is invalid."), error
    assert v3.tree == pristine(sandbox)


def test_a_registered_name_v2_allowed_still_loads(sandbox: Sandbox):
    """``a--b`` created by v2 keeps working in v3."""
    oracle(sandbox, "create", "a--b", "--json")
    sandbox.freeze()
    for args in (["wake-up", "--brain", "a--b", "--json"], ["list", "--json"], ["status", "--brain", "a--b", "--json"],
                 ["remove", "a--b", "--json"]):
        r = run_trees(sandbox, args)
        assert r["v2"].code == r["v3"].code == 0, (args, r["v3"].stderr)
        assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)


def _v2_rule_process(sb: Sandbox) -> subprocess.Popen:
    python = Path(oracle_kluris()).parent / ("python.exe" if IS_WINDOWS else "python")
    script = (
        "import sys, json\n"
        "from kluris.core.brain import validate_brain_name\n"
        "for line in sys.stdin:\n"
        "    print(json.dumps(validate_brain_name(json.loads(line))), flush=True)\n"
    )
    return subprocess.Popen([str(python), "-I", "-c", script], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            text=True, encoding="utf-8", cwd=sb.root)


@settings(max_examples=NAME_EXAMPLES, deadline=None,
          suppress_health_check=[HealthCheck.too_slow, HealthCheck.function_scoped_fixture])
@given(name=st.one_of(
    st.text(alphabet="az09-._A/ \n", min_size=1, max_size=12),
    st.from_regex(r"[a-z][a-z0-9-]{0,50}", fullmatch=True),
    st.text(min_size=1, max_size=6).filter(lambda s: "\x00" not in s),
))
def test_name_rule_against_v2(sandbox: Sandbox, name):
    """v3 accepts exactly ``^[a-z][a-z0-9]*(-[a-z0-9]+)*$`` (at most 48, not
    ``all``), a subset of what v2 accepts. v3 is asked through ``create``
    with ``--path`` a file, so a valid name stops at the path check and
    nothing is written."""
    import re

    if not hasattr(sandbox, "_rule"):
        (sandbox.work / "afile").write_text("x", encoding="utf-8")
        sandbox._rule = _v2_rule_process(sandbox)  # type: ignore[attr-defined]
    proc = sandbox._rule  # type: ignore[attr-defined]
    proc.stdin.write(json.dumps(name) + "\n")
    proc.stdin.flush()
    v2_valid = json.loads(proc.stdout.readline())
    v3 = subprocess.run([*_exe("v3"), "create", name, "--path", "afile", "--json"], env=sandbox.env,
                        cwd=sandbox.work, capture_output=True)
    error = json.loads(v3.stdout)["error"]
    v3_valid = error == "--path 'afile' is not a directory."
    strict = re.fullmatch(r"[a-z][a-z0-9]*(-[a-z0-9]+)*", name) is not None and len(name) <= 48 and name != "all"
    assert v3_valid == strict, (name, error)
    assert not v3_valid or v2_valid, name


# --- create: git failure (allowlisted) -------------------------------------------------------------


def test_a_git_failure_leaves_the_brain_unregistered(sandbox: Sandbox):
    """v2 raises ``CalledProcessError`` (traceback) after scaffolding; v3
    names the git command and what is left, and registers nothing."""
    sandbox.freeze()
    r = run_trees(sandbox, ["create", "demo", "--branch", "bad..name", "--json"])
    assert r["v2"].code == 1 and "CalledProcessError" in r["v2"].stderr
    v3 = r["v3"]
    assert v3.code == 1
    error = json_of(v3, sandbox)["error"]
    assert error.startswith("`git checkout -b bad..name` failed (exit "), error
    assert at("/work/demo") in error and "not registered" in error, error
    assert "home/.kluris/config.yml" not in v3.tree
    assert "demo/brain.md" in v3.tree, "the scaffold is left on disk, as the message says"


# --- register ---------------------------------------------------------------------------------------


def team_brain(sb: Sandbox, *, keep_yml: bool = True) -> Path:
    """``team`` created by v2 in ``brains/team`` (git), then unregistered."""
    brains = sb.work / "brains"
    brains.mkdir(exist_ok=True)
    oracle(sb, "create", "team", "--path", str(brains), "--description", "The team brain", "--json")
    oracle(sb, "remove", "team", "--json")
    if not keep_yml:
        (brains / "team" / "kluris.yml").unlink()
    return brains / "team"


def handmade_brain(sb: Sandbox, rel: str, brain_md: str) -> Path:
    path = sb.work / rel
    path.mkdir(parents=True)
    (path / "brain.md").write_text(brain_md, encoding="utf-8")
    return path


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_register_a_v2_brain(sandbox: Sandbox, json_flag):
    team_brain(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["register", "brains/team", *(["--json"] if json_flag else [])])
    assert_same_run(r, sandbox, json_out=json_flag)
    if not json_flag:
        assert r["v3"].stdout.startswith("Brain registered: team\n  Path: "), r["v3"].stdout


def test_register_writes_a_missing_kluris_yml(sandbox: Sandbox):
    team_brain(sandbox, keep_yml=False)
    sandbox.freeze()
    r = run_trees(sandbox, ["register", "brains/team", "--json"])
    assert_same_run(r, sandbox, yml={"brains/team/kluris.yml"})


@pytest.mark.parametrize("rel,brain_md", [
    ("brains/Some Folder", "---\nauto_generated: true\n---\n\n# team-x\n\nThe team brain.\n\n## Lobes\n"),
    ("brains/My Brain", "# Not Valid!\n\n## Lobes\n\nAfter lobes\n"),
    ("brains/plain", "no heading here\n"),
    ("brains/rawscan", "---\nkey: [unclosed\n---\n# raw-name\n\nraw desc\n"),
    ("brains/team-y", "\n\n#   spaced-name  \n\n  Indented description.  \n"),
    ("brains/team-z", "# first\n# second\n\n## Sub\n\nlater line\n"),
], ids=["h1", "fallback-lowered", "no-heading", "frontmatter-fails", "spaces", "two-h1"])
def test_register_identity_from_brain_md(sandbox: Sandbox, rel, brain_md):
    handmade_brain(sandbox, rel, brain_md)
    sandbox.freeze()
    r = run_trees(sandbox, ["register", rel, "--json"])
    yml = rel + "/kluris.yml"
    assert_same_run(r, sandbox, yml={yml})


def test_register_an_invalid_identity(sandbox: Sandbox):
    handmade_brain(sandbox, "brains/9bad", "# Also Bad\n")
    sandbox.freeze()
    for json_flag in (True, False):
        r = run_trees(sandbox, ["register", "brains/9bad", *(["--json"] if json_flag else [])])
        assert_same_error(r, sandbox)


@pytest.mark.parametrize("source", ["x.zip", "brains/X.ZIP", "~/b.Zip", "nope", "~/nope", "afile", "empty",
                                    "./nope//deeper/"])
def test_register_path_errors(sandbox: Sandbox, source):
    (sandbox.work / "afile").write_text("x", encoding="utf-8")
    (sandbox.work / "empty").mkdir()
    sandbox.freeze()
    for json_flag in (True, False):
        r = run_trees(sandbox, ["register", source, *(["--json"] if json_flag else [])])
        assert_same_error(r, sandbox)
    if source.lower().endswith(".zip"):
        assert ZIP_HINT in r["v3"].stderr


def test_register_the_same_brain_again(sandbox: Sandbox):
    """Same name and path: success with ``already_registered`` and no skill
    install."""
    team_brain(sandbox)
    oracle(sandbox, "register", str(sandbox.work / "brains" / "team"), "--json")
    sandbox.freeze()
    for json_flag in (True, False):
        r = run_trees(sandbox, ["register", "brains/team", *(["--json"] if json_flag else [])])
        assert r["v2"].code == r["v3"].code == 0
        if json_flag:
            assert_json_equal(r["v2"].stdout, r["v3"].stdout, sandbox.root)
        else:
            same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)
        assert r["v2"].tree == r["v3"].tree == pristine(sandbox)


def test_register_conflicts(sandbox: Sandbox):
    team = team_brain(sandbox)
    oracle(sandbox, "register", str(team), "--json")
    shutil.copytree(team, sandbox.work / "brains" / "copy", ignore=shutil.ignore_patterns(".git"))
    sandbox.freeze()
    # Same name, another path.
    r = run_trees(sandbox, ["register", "brains/copy", "--json"])
    assert_same_error(r, sandbox)
    assert "is already registered at" in r["v3"].stdout
    # Same path, another name.
    sandbox_state = sandbox.root / "pristine" / "brains" / "team" / "brain.md"
    sandbox_state.write_text(sandbox_state.read_text(encoding="utf-8").replace("# team", "# other"),
                             encoding="utf-8")
    r = run_trees(sandbox, ["register", "brains/team", "--json"])
    assert_same_error(r, sandbox)
    assert "is already registered as brain 'team'" in r["v3"].stdout


def test_register_a_brain_inside_another(sandbox: Sandbox):
    """v2 does not check this for register; neither does v3."""
    team = team_brain(sandbox)
    oracle(sandbox, "register", str(team), "--json")
    handmade_brain(sandbox, "brains/team/projects/sub", "# sub-brain\n\nInner.\n")
    sandbox.freeze()
    r = run_trees(sandbox, ["register", "brains/team/projects/sub", "--json"])
    assert_same_run(r, sandbox, yml={"brains/team/projects/sub/kluris.yml"})


def test_register_by_home_and_symlink(sandbox: Sandbox):
    team = team_brain(sandbox)
    (sandbox.home / "brains").mkdir()
    shutil.move(str(team), str(sandbox.home / "brains" / "team"))
    if not IS_WINDOWS:
        (sandbox.work / "link").symlink_to(sandbox.home / "brains" / "team")
    sandbox.freeze()
    r = run_trees(sandbox, ["register", "~/brains/team", "--json"])
    assert_same_run(r, sandbox)
    if not IS_WINDOWS:
        r = run_trees(sandbox, ["register", "link", "--json"])
        assert_same_run(r, sandbox)
        assert json_of(r["v3"], sandbox)["path"] == at("/work/home/brains/team")


def test_register_with_an_unreadable_kluris_yml(sandbox: Sandbox):
    team = team_brain(sandbox)
    (team / "kluris.yml").write_text("name: [broken\n", encoding="utf-8")
    sandbox.freeze()
    r = run_trees(sandbox, ["register", "brains/team", "--json"])
    for side in ("v2", "v3"):
        assert r[side].code == 1
        assert json_of(r[side], sandbox)["error"].startswith("Register failed: "), r[side].stdout
        assert "home/.kluris/config.yml" in r[side].tree  # written by v2's create, unchanged
        assert r[side].tree["home/.kluris/config.yml"] == pristine(sandbox)["home/.kluris/config.yml"]


@pty_only
def test_register_prompts_on_a_terminal(sandbox: Sandbox):
    team_brain(sandbox)
    sandbox.freeze()
    r = pty_both(sandbox, ["register"], ["", "brains/team", "6"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    assert normalize(wizard_transcript(v3.stdout), sandbox.root) == normalize(wizard_transcript(v2.stdout),
                                                                              sandbox.root)
    assert v3.stdout.startswith("\nRegister a brain\n\n  Path to brain directory:   Path to brain directory: \n")
    same_words(v2.stdout, v3.stdout, sandbox.root)
    assert_tree(v3.tree, expected_v3(v2.tree, {"brains/team/kluris.yml"}))


@pty_only
def test_register_end_of_input_at_the_picker(sandbox: Sandbox):
    """v2 catches Click's ``Abort`` as any other exception: ``Register
    failed: `` with nothing after it; the ``kluris.yml`` it wrote stays."""
    team_brain(sandbox, keep_yml=False)
    sandbox.freeze()
    r = pty_both(sandbox, ["register"], ["brains/team"])
    for side in ("v2", "v3"):
        assert r[side].code == 1 and r[side].stderr == "Error: Register failed: \n", r[side]
    assert r["v3"].stdout == r["v2"].stdout
    assert_tree(r["v3"].tree, expected_v3(r["v2"].tree, {"brains/team/kluris.yml"}))


@pty_only
def test_create_end_of_input_at_the_picker(sandbox: Sandbox):
    """``Aborted!`` after the scaffold: the files stay, nothing is
    registered or committed (v2 too)."""
    sandbox.freeze()
    r = pty_both(sandbox, ["create"], ["wiz", "d", "", "", "1", ""], git=("wiz",))
    for side in ("v2", "v3"):
        assert r[side].code == 1 and r[side].stderr == "Aborted!\n", r[side]
        assert "wiz/brain.md" in r[side].tree and "home/.kluris/config.yml" not in r[side].tree
    assert r["v3"].stdout == r["v2"].stdout
    assert_tree(r["v3"].tree, expected_v3(r["v2"].tree, {"wiz/kluris.yml"}))
    assert r["v3"].git == r["v2"].git


@pytest.mark.parametrize("json_flag", [False, True], ids=["text", "json"])
def test_register_without_a_path_and_no_terminal_is_an_error(sandbox: Sandbox, json_flag):
    """v2 prompts for the path even under ``--json`` and aborts on the empty
    pipe; v3 names the missing value (allowlisted)."""
    sandbox.freeze()
    r = run_trees(sandbox, ["register", *(["--json"] if json_flag else [])])
    assert r["v2"].code == 1 and r["v2"].stderr.endswith("Aborted!\n")
    v3 = r["v3"]
    assert v3.code == 1
    if json_flag:
        assert v3.stdout == json.dumps({"ok": False, "error": PATH_REQUIRED}) + "\n"
    else:
        assert v3.stderr == f"Error: {PATH_REQUIRED}\n"
    assert v3.tree == pristine(sandbox)


# --- remove -----------------------------------------------------------------------------------------


def two_brains(sb: Sandbox) -> None:
    brains = sb.work / "brains"
    brains.mkdir()
    oracle(sb, "create", "a", "--path", str(brains), "--json")
    oracle(sb, "create", "b", "--path", str(brains), "--no-git", "--json")


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_remove(sandbox: Sandbox, json_flag):
    two_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, ["remove", "a", *(["--json"] if json_flag else [])])
    assert_same_run(r, sandbox, json_out=json_flag)
    if not json_flag:
        assert r["v3"].stdout == r["v2"].stdout == "Unregistered: a (files preserved)\n"
    assert "brains/a/brain.md" in r["v3"].tree
    assert not any(k.startswith("home/") and "kluris-a/" in k for k in r["v3"].tree)


def test_remove_the_last_brain(sandbox: Sandbox):
    oracle(sandbox, "create", "only", "--json")
    sandbox.freeze()
    r = run_trees(sandbox, ["remove", "only", "--json"])
    assert_same_run(r, sandbox)
    assert r["v3"].tree["home/.kluris/config.yml"].replace(b"\r\n", b"\n") == b"brains: {}\n"
    assert not any("/kluris-" in k for k in r["v3"].tree if k.startswith("home/"))


@pytest.mark.parametrize("args,code", [
    (["remove", "nope"], 1),
    (["remove", "nope", "--json"], 1),
    (["remove"], 2),
    (["remove", "--json"], 1),
], ids=["unknown", "unknown-json", "missing", "missing-json"])
def test_remove_errors(sandbox: Sandbox, args, code):
    two_brains(sandbox)
    sandbox.freeze()
    r = run_trees(sandbox, args)
    assert_same_error(r, sandbox, code)


def test_root_help_lists_the_new_commands(sandbox: Sandbox):
    sandbox.freeze()
    import re

    r = run_trees(sandbox, ["--help"])
    for command in ("create", "register", "remove"):
        line = next(line for line in r["v2"].stdout.splitlines() if line.startswith(f"  {command} "))
        assert re.search(rf"^  {re.escape(command)} +{re.escape(line.split(maxsplit=1)[1])}$",
                         r["v3"].stdout, re.M), line
    for command, words in (("create", ["Create a new brain.", "--description", "--path", "--type", "--remote",
                                       "--branch", "--no-git", "--from-config", "--json"]),
                           ("register", ["Register an existing brain directory on disk.", "--json"]),
                           ("remove", ["Unregister a brain (does not delete files).", "--json"])):
        r = run_trees(sandbox, [command, "--help"])
        assert r["v2"].code == r["v3"].code == 0
        for word in words:
            assert word in r["v2"].stdout and word in r["v3"].stdout, (command, word)


# --- write safety (review T-103) -------------------------------------------------


@pytest.mark.skipif(IS_WINDOWS, reason="POSIX directory modes")
def test_create_into_a_directory_that_cannot_be_listed(sandbox: Sandbox):
    """PAR-068: v2 stops with a ``PermissionError`` traceback when it cannot
    list an existing target; v3 refuses with a clean error. Neither writes."""
    demo = sandbox.work / "brains" / "demo"
    demo.mkdir(parents=True)
    (demo / "README.md").write_text("my own readme\n", encoding="utf-8")
    sandbox.freeze()
    results = {}
    for side in ("v2", "v3"):
        sandbox.checkout()
        target = sandbox.work / "brains" / "demo"
        target.chmod(0o300)
        try:
            if os.access(target, os.R_OK):
                pytest.skip("running as root: every directory can be listed")
            proc = subprocess.run(
                [*_exe(side), "create", "demo", "--path", "brains", "--type", "blank", "--no-git", "--json"],
                env=_env(sandbox, {}), cwd=str(sandbox.work), stdin=subprocess.DEVNULL,
                capture_output=True, timeout=120,
            )
        finally:
            target.chmod(0o755)
        results[side] = (proc, snapshot_tree(sandbox.work))
        sandbox.stash(f"{side}-{time.monotonic_ns()}")
    (p2, t2), (p3, t3) = results["v2"], results["v3"]
    assert p2.returncode == 1 and p2.stdout == b"" and b"PermissionError" in p2.stderr
    doc = json.loads(p3.stdout)
    assert p3.returncode == 1 and doc["ok"] is False
    assert doc["error"].startswith(str(sandbox.work / "brains" / "demo") + " cannot be listed: ")
    assert [rel for rel in t3 if rel.startswith("brains/")] == ["brains/demo/README.md"]
    assert t3["brains/demo/README.md"] == t2["brains/demo/README.md"] == b"my own readme\n"
    assert not (sandbox.work / "home" / ".kluris" / "config.yml").exists()
