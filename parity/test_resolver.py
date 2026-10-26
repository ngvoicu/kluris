"""Resolver parity: every case of v2's ``_resolve_brains`` / ``_check_brain_paths``
/ ``_pick_brain_interactively`` (research-03 §1.0) against the v2.28.11 oracle.

Single-brain commands go through ``wake-up`` and the ``allow_all`` ones through
``status`` (a fan-out command), the same command on both sides: errors are
compared whole, a success by its whole ``--json`` output (and the brain names
it lists) or its text.

Stdin is a pipe unless a test says otherwise; ``test_null_stdin`` gives both
the null device, which v2 reads as a terminal on Windows (PAR-022).

The picker needs a terminal on stdin: those tests drive both binaries through
a pseudo-terminal (stdout and stderr stay pipes) and are skipped on Windows,
which has no ``pty``.
"""

from __future__ import annotations

import json
import re

import pytest

from harness import IS_WINDOWS, Sandbox, assert_json_equal, normalize
from runs import run_pair, run_pty, same_words, transcript, write_config

ALL_NOT_SUPPORTED = "--brain all is only supported on dream, status, mri, and companion commands."
NO_BRAINS = "No brains registered. Run 'kluris create <name>' to create one."


def unknown(name: str) -> str:
    return f"No brain named '{name}' is registered. Run 'kluris list' to see available brains."


def brains(sb: Sandbox, spec: list[tuple[str, str]]) -> dict[str, str]:
    """Register ``(name, kind)`` brains in order: ``dir`` makes the directory,
    ``file`` a file at the path, ``missing`` nothing. Returns name -> path."""
    root = sb.work / "brains"
    root.mkdir(exist_ok=True)
    paths = {}
    rows = []
    for name, kind in spec:
        path = root / f"brain-{len(paths)}"
        if kind == "dir":
            path.mkdir()
        elif kind == "file":
            path.write_text("x", encoding="utf-8")
        paths[name] = str(path)
        rows.append((name, str(path), f"{name} brain"))
    write_config(sb, rows)
    sb.freeze()
    return paths


def fan(*args: str) -> tuple[list[str], list[str]]:
    """An ``allow_all`` command: ``status`` on both sides."""
    return ["status", *args], ["status", *args]


def single(*args: str) -> tuple[list[str], list[str]]:
    return ["wake-up", *args], ["wake-up", *args]


def assert_same_error(sb: Sandbox, pair, message: str | None = None, **kw) -> None:
    """Both fail alike: exit 1 and, under ``--json``, the same envelope on
    stdout; otherwise ``Error: <message>`` on stderr and nothing on stdout."""
    v2_args, v3_args = pair
    r = run_pair(sb, v2_args, v3_args, **kw)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 1, f"v2: exit {v2.code} {v2.stdout} {v2.stderr}"
    assert v3.code == v2.code, f"v3: exit {v3.code} {v3.stdout} {v3.stderr}"
    if "--json" in v2_args:
        assert normalize(v3.stdout, sb.root) == normalize(v2.stdout, sb.root)
        if message is not None:
            envelope = json.loads(normalize(v2.stdout, sb.root))
            assert envelope == {"ok": False, "error": normalize(message, sb.root)}
    else:
        assert v3.stdout == v2.stdout == ""
        assert normalize(v3.stderr, sb.root) == normalize(v2.stderr, sb.root)
        if message is not None:
            assert normalize(v2.stderr, sb.root) == normalize(f"Error: {message}\n", sb.root)


def resolved_names(sb: Sandbox, pair, **kw) -> list[str]:
    """Both succeed with the same ``status --json``; the brain names it lists."""
    v2_args, v3_args = pair
    r = run_pair(sb, v2_args, v3_args, **kw)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == 0, f"v2: {v2.stdout} {v2.stderr}"
    assert v3.code == 0, f"v3: {v3.stdout} {v3.stderr}"
    assert_json_equal(v2.stdout, v3.stdout, sb.root)
    return [b["name"] for b in json.loads(v2.stdout)["brains"]]


def after_picker(stdout: str) -> str:
    """What a command printed after the picker's last prompt."""
    return stdout[len(transcript(stdout)):]


def status_names(text: str) -> list[str]:
    """The brain names ``status`` text prints, in order (each brain block
    opens with a blank line, then the name)."""
    return re.findall(r"^(\S.*)\n  Lobes: ", text, re.M)


def missing_message(rows: list[tuple[str, str]]) -> str:
    body = "\n".join(f"  - {n}: {p}" for n, p in rows)
    return (
        "The following registered brain(s) have missing or invalid paths:\n"
        f"{body}\nRun `kluris remove <name>` or fix the path in the global config."
    )


# --- non-interactive cases ------------------------------------------------------


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_brain_all_is_refused_on_single_brain_commands(sandbox, json_flag):
    brains(sandbox, [("a", "dir"), ("b", "dir")])
    flags = ["--json"] if json_flag else []
    assert_same_error(sandbox, single("--brain", "all", *flags), ALL_NOT_SUPPORTED)


def test_brain_all_is_refused_before_counting_brains(sandbox):
    brains(sandbox, [])
    assert_same_error(sandbox, single("--brain", "all", "--json"), ALL_NOT_SUPPORTED)


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_brain_all_with_zero_brains(sandbox, json_flag):
    brains(sandbox, [])
    flags = ["--json"] if json_flag else []
    assert_same_error(sandbox, fan("--brain", "all", *flags), NO_BRAINS)


def test_brain_all_is_every_brain_in_config_order(sandbox):
    brains(sandbox, [("zeta", "dir"), ("alpha", "dir"), ("mid", "dir")])
    assert resolved_names(sandbox, fan("--brain", "all", "--json")) == ["zeta", "alpha", "mid"]


def test_brain_all_with_a_missing_path(sandbox):
    paths = brains(sandbox, [("ok", "dir"), ("gone", "missing"), ("file", "file")])
    message = missing_message([("gone", paths["gone"]), ("file", paths["file"])])
    assert_same_error(sandbox, fan("--brain", "all", "--json"), message)
    assert_same_error(sandbox, fan("--brain", "all"), message)


@pytest.mark.parametrize("count", [0, 1, 2])
@pytest.mark.parametrize("kind", ["single", "fan"])
def test_unknown_brain_name(sandbox, count, kind):
    brains(sandbox, [("a", "dir"), ("b", "dir")][:count])
    make = single if kind == "single" else fan
    assert_same_error(sandbox, make("--brain", "nope", "--json"), unknown("nope"))
    assert_same_error(sandbox, make("--brain", "nope"), unknown("nope"))


def test_brain_names_match_exactly(sandbox):
    brains(sandbox, [("Demo", "dir")])
    assert_same_error(sandbox, single("--brain", "demo", "--json"), unknown("demo"))


def test_named_brain(sandbox):
    brains(sandbox, [("a", "dir"), ("b", "dir"), ("c", "dir")])
    assert resolved_names(sandbox, fan("--brain", "b", "--json")) == ["b"]
    r = run_pair(sandbox, *single("--brain", "b", "--json"))
    assert r["v2"].code == r["v3"].code == 0, (r["v2"].stderr, r["v3"].stderr)
    assert json.loads(r["v2"].stdout)["name"] == json.loads(r["v3"].stdout)["name"] == "b"


def test_the_last_brain_option_wins(sandbox):
    brains(sandbox, [("a", "dir"), ("b", "dir")])
    assert resolved_names(sandbox, fan("--brain", "a", "--brain=b", "--json", "--json")) == ["b"]
    r = run_pair(sandbox, *single("--brain", "nope", "--brain", "a", "--json"))
    assert r["v2"].code == r["v3"].code == 0, (r["v2"].stdout, r["v3"].stdout)


def test_an_empty_brain_name_is_no_name(sandbox):
    brains(sandbox, [("only", "dir")])
    assert resolved_names(sandbox, fan("--brain", "", "--json")) == ["only"]
    assert resolved_names(sandbox, fan("--brain=", "--json")) == ["only"]


def test_an_empty_brain_name_with_two_brains(sandbox):
    brains(sandbox, [("a", "dir"), ("b", "dir")])
    assert_same_error(
        sandbox, fan("--brain", "", "--json"),
        "Multiple brains registered. Pass --brain NAME or 'all'. Available: a, b",
    )


def test_a_brain_option_value_that_looks_like_an_option(sandbox):
    # Click takes the next token as the value, even `--json`; the envelope
    # still applies because `--json` is on the command line.
    brains(sandbox, [("a", "dir")])
    assert_same_error(sandbox, single("--brain", "--json"), unknown("--json"))


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_brain_option_without_a_value(sandbox, json_flag):
    brains(sandbox, [("a", "dir")])
    flags = ["--json"] if json_flag else []
    v2_args, v3_args = single(*flags, "--brain")
    r = run_pair(sandbox, v2_args, v3_args)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == v2.code == (1 if json_flag else 2), (v2.code, v3.code)
    if json_flag:
        assert json.loads(v2.stdout) == {"ok": False, "error": "Option '--brain' requires an argument."}
        assert v3.stdout == v2.stdout
    else:
        assert v2.stderr.endswith("Error: Option '--brain' requires an argument.\n")
        assert v3.stderr.endswith("Error: Option '--brain' requires an argument.\n"), v3.stderr


@pytest.mark.parametrize("json_flag", [True, False], ids=["json", "text"])
def test_zero_brains(sandbox, json_flag):
    brains(sandbox, [])
    flags = ["--json"] if json_flag else []
    assert_same_error(sandbox, single(*flags), NO_BRAINS)
    assert_same_error(sandbox, fan(*flags), NO_BRAINS)


def test_no_config_file_is_zero_brains(sandbox):
    sandbox.freeze()
    assert_same_error(sandbox, single("--json"), NO_BRAINS)


def test_exactly_one_brain_needs_no_picker(sandbox):
    brains(sandbox, [("solo", "dir")])
    # Even with stdin a pipe and prompting allowed.
    assert resolved_names(sandbox, fan("--json"), env={"KLURIS_NO_PROMPT": None}) == ["solo"]
    r = run_pair(sandbox, *single("--json"), env={"KLURIS_NO_PROMPT": None})
    assert r["v2"].code == r["v3"].code == 0


@pytest.mark.parametrize("kind", ["missing", "file"])
def test_one_brain_with_a_bad_path(sandbox, kind):
    paths = brains(sandbox, [("solo", kind)])
    message = missing_message([("solo", paths["solo"])])
    assert_same_error(sandbox, single("--json"), message)
    assert_same_error(sandbox, single(), message)
    assert_same_error(sandbox, fan("--brain", "solo", "--json"), message)


@pytest.mark.parametrize(
    "how",
    [
        {"args": ["--json"], "env": {}},
        {"args": ["--json"], "env": {"KLURIS_NO_PROMPT": None}},
        {"args": [], "env": {}},  # KLURIS_NO_PROMPT=1 (the harness default)
        {"args": [], "env": {"KLURIS_NO_PROMPT": None}},  # stdin is a pipe
        {"args": [], "env": {"KLURIS_NO_PROMPT": "true"}},
        {"args": [], "env": {"KLURIS_NO_PROMPT": "0"}},
    ],
    ids=["json", "json-prompt-allowed", "no-prompt", "pipe", "no-prompt-true", "no-prompt-0"],
)
@pytest.mark.parametrize("kind", ["single", "fan"])
def test_several_brains_non_interactive(sandbox, how, kind):
    brains(sandbox, [("b", "dir"), ("a", "missing"), ("c", "dir")])
    hint = "" if kind == "single" else " or 'all'"
    message = f"Multiple brains registered. Pass --brain NAME{hint}. Available: b, a, c"
    make = single if kind == "single" else fan
    assert_same_error(sandbox, make(*how["args"]), message, env=how["env"])


def test_unicode_names_and_paths(sandbox):
    root = sandbox.work / "brains"
    root.mkdir()
    (root / "café").mkdir()
    write_config(sandbox, [("café", str(root / "café"), "déjà"), ("b", str(root / "nope"), "")])
    sandbox.freeze()
    assert_same_error(
        sandbox, single("--json"), "Multiple brains registered. Pass --brain NAME. Available: café, b"
    )
    r = run_pair(sandbox, *single("--brain", "café", "--json"))
    assert r["v2"].code == r["v3"].code == 0
    assert normalize(r["v3"].stdout, sandbox.root) == normalize(r["v2"].stdout, sandbox.root)
    assert "\\u00e9" in r["v2"].stdout


@pytest.mark.parametrize("kind", ["single", "fan"])
def test_null_stdin(sandbox, kind):
    """Stdin is the null device (``/dev/null``, ``NUL`` on Windows), prompting
    allowed. Python's ``isatty()`` is true for ``NUL``, so v2 on Windows shows
    the picker and aborts at the end of input; v3 asks whether stdin is a
    console and prints the non-interactive error. Both exit 1 (PAR-022).
    Elsewhere both print the error."""
    paths = brains(sandbox, [("a", "dir"), ("b", "dir")])
    make = single if kind == "single" else fan
    hint = "" if kind == "single" else " or 'all'"
    message = f"Multiple brains registered. Pass --brain NAME{hint}. Available: a, b"
    r = run_pair(sandbox, *make(), env={"KLURIS_NO_PROMPT": None}, stdin=None)
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 1, (v2.code, v3.code, v2.stderr, v3.stderr)
    assert (v3.stdout, v3.stderr) == ("", f"Error: {message}\n")
    if IS_WINDOWS:
        expected = menu(paths, set(), allow_all=kind == "fan") + "Choice: "
        assert normalize(v2.stdout, sandbox.root).startswith(normalize(expected, sandbox.root)), v2.stdout
        assert v2.stderr == "Aborted!\n"
    else:
        assert (v2.stdout, v2.stderr) == (v3.stdout, v3.stderr)


# --- the picker (a terminal on stdin) ---------------------------------------------

pty_only = pytest.mark.skipif(IS_WINDOWS, reason="the picker needs a pseudo-terminal; Windows has no pty")


def pty_both(sb, pair, **kw):
    v2_args, v3_args = pair
    return run_pty(sb, "v2", v2_args, **kw), run_pty(sb, "v3", v3_args, **kw)


def menu(paths: dict[str, str], missing: set[str], allow_all: bool) -> str:
    lines = ["Multiple brains registered. Pick one:"]
    for i, (name, path) in enumerate(paths.items(), start=1):
        lines.append(f"  [{i}] {name}  ({path})" + ("  (missing)" if name in missing else ""))
    if allow_all:
        lines.append(f"  [{len(paths) + 1}] all")
    return "\n".join(lines) + "\n"


@pty_only
def test_picker_picks_a_brain_for_wake_up(sandbox):
    paths = brains(sandbox, [("a", "dir"), ("b", "dir")])
    v2, v3 = pty_both(sandbox, single(), lines=["2"])
    assert v2.code == 0, v2.stderr
    assert v3.code == 0, v3.stderr
    expected = menu(paths, set(), allow_all=False) + "Choice: "
    assert normalize(transcript(v2.stdout), sandbox.root) == normalize(expected, sandbox.root)
    assert normalize(transcript(v3.stdout), sandbox.root) == normalize(transcript(v2.stdout), sandbox.root)
    # Then wake-up's text for `b`.
    same_words(v2.stdout[len(transcript(v2.stdout)):], v3.stdout[len(transcript(v3.stdout)):], sandbox.root)
    assert "Brain: b" in v3.stdout


@pty_only
def test_picker_reprompts_on_invalid_input(sandbox):
    paths = brains(sandbox, [("a", "dir"), ("b", "missing"), ("c", "file")])
    inputs = ["abc", "", " 'x", "0", "9", "1_0", "1__0", " ", "１"]
    v2, v3 = pty_both(sandbox, fan(), lines=inputs)
    assert v2.code == 0, v2.stderr
    assert v3.code == 0, v3.stderr
    script = (
        menu(paths, {"b"}, allow_all=True)
        + "Choice: Error: 'abc' is not a valid integer range.\n"
        + "Choice: Choice: Error: \" 'x\" is not a valid integer range.\n"
        + "Choice: Error: 0 is not in the range 1<=x<=4.\n"
        + "Choice: Error: 9 is not in the range 1<=x<=4.\n"
        + "Choice: Error: 10 is not in the range 1<=x<=4.\n"
        + "Choice: Error: '1__0' is not a valid integer range.\n"
        + "Choice: Error: ' ' is not a valid integer range.\n"
        + "Choice: "
    )
    assert normalize(transcript(v2.stdout), sandbox.root) == normalize(script, sandbox.root)
    assert normalize(transcript(v3.stdout), sandbox.root) == normalize(transcript(v2.stdout), sandbox.root)
    same_words(after_picker(v2.stdout), after_picker(v3.stdout), sandbox.root)
    assert status_names(after_picker(v3.stdout)) == status_names(after_picker(v2.stdout)) == ["a"]


@pty_only
def test_picker_all_is_every_brain(sandbox):
    paths = brains(sandbox, [("a", "dir"), ("b", "dir")])
    v2, v3 = pty_both(sandbox, fan("--brain", ""), lines=["+3"])
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    assert transcript(v3.stdout) == transcript(v2.stdout) == menu(paths, set(), True) + "Choice: "
    same_words(after_picker(v2.stdout), after_picker(v3.stdout), sandbox.root)
    assert status_names(after_picker(v3.stdout)) == status_names(after_picker(v2.stdout)) == ["a", "b"]


@pty_only
@pytest.mark.parametrize("choice", ["2", "3"], ids=["missing", "all"])
def test_picker_choice_is_path_checked(sandbox, choice):
    paths = brains(sandbox, [("a", "dir"), ("b", "missing")])
    v2, v3 = pty_both(sandbox, fan(), lines=[choice])
    assert v2.code == v3.code == 1
    assert transcript(v3.stdout) == transcript(v2.stdout)
    expected = f"Error: {missing_message([('b', paths['b'])])}\n"
    assert normalize(v2.stderr, sandbox.root) == normalize(expected, sandbox.root)
    assert v3.stderr == v2.stderr


@pty_only
def test_picker_missing_brain_for_wake_up(sandbox):
    brains(sandbox, [("a", "dir"), ("b", "missing")])
    v2, v3 = pty_both(sandbox, single(), lines=["2"])
    assert v2.code == v3.code == 1
    assert transcript(v3.stdout) == transcript(v2.stdout)
    assert v3.stderr == v2.stderr


@pty_only
@pytest.mark.parametrize("how", ["eof", "eof-after-input", "sigint"])
def test_picker_abort(sandbox, how):
    paths = brains(sandbox, [("a", "dir"), ("b", "dir")])
    kw = {"sigint": True} if how == "sigint" else {"eof": True, "lines": ["x"] if how == "eof-after-input" else []}
    v2, v3 = pty_both(sandbox, single(), **kw)
    assert v2.code == 1, (v2.stdout, v2.stderr)
    assert v3.code == 1, (v3.stdout, v3.stderr)
    assert v2.stderr == "Aborted!\n"
    assert v3.stderr == v2.stderr
    assert v3.stdout == v2.stdout
    assert v2.stdout.startswith(menu(paths, set(), False))


@pty_only
def test_picker_prompting_allowed_by_any_other_no_prompt_value(sandbox):
    paths = brains(sandbox, [("a", "dir"), ("b", "dir")])
    v2, v3 = pty_both(sandbox, fan(), lines=["1"], env={"KLURIS_NO_PROMPT": "yes"})
    assert v2.code == v3.code == 0
    assert transcript(v3.stdout) == transcript(v2.stdout) == menu(paths, set(), True) + "Choice: "


@pty_only
@pytest.mark.parametrize(
    "how", [{"args": ["--json"], "env": {}}, {"args": [], "env": {"KLURIS_NO_PROMPT": "1"}}], ids=["json", "no-prompt"]
)
def test_terminal_without_prompting_lists_the_brains(sandbox, how):
    brains(sandbox, [("a", "dir"), ("b", "dir")])
    v2, v3 = pty_both(sandbox, fan(*how["args"]), env=how["env"])
    assert v2.code == v3.code == 1
    assert v3.stdout == v2.stdout
    assert v3.stderr == v2.stderr
    assert "Multiple brains registered. Pass --brain NAME or 'all'. Available: a, b" in v2.stdout + v2.stderr
