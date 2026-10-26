"""Regenerate the v1/v2 skill shapes Kluris 3 recognises as its own.

Run from the repo root with any Python 3.10+ (each release's ``agents.py``
imports only ``pathlib``, and the tags are in this repository):

    python3 -I parity/tools/gen_legacy_skills.py

For every release tag from v1.0.0 to v2.28.11 it runs that release's own
``kluris/core/agents.py`` and groups the releases by what their SKILL.md
renderings share: the description template (``{brain_name}`` the only
slot), the frontmatter style (quoted, or v1.0's folded one with
``allowed-tools``), the fixed headings of ``SKILL_BODY`` in order, and
whether the release wrote Windsurf workflows. It writes:

- ``crates/kluris-core/src/agents/legacy.rs``: one ``Era`` per group, the
  table ``render::is_generated_skill`` / ``is_generated_workflow`` match;
- ``crates/kluris-core/tests/fixtures/skills/legacy/<era>.md`` (and
  ``.workflow.md``, and ``-bare`` copies named ``kluris`` for the v2.0
  single-brain world): real renderings of each group's first release, for
  ``tests/agents_render.rs``.

Every group is checked against its own renderings before anything is
written.
"""

from __future__ import annotations

import re
import subprocess
import sys
import types
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RUST_OUT = REPO / "crates" / "kluris-core" / "src" / "agents" / "legacy.rs"
FIXTURES = REPO / "crates" / "kluris-core" / "tests" / "fixtures" / "skills" / "legacy"
LAST = "v2.28.11"
HEADING = re.compile(r"^#{1,3} ")
PLACEHOLDER = re.compile(r"\{[a-z_]+\}")
SAMPLE_BRAIN = "acme"
V1_BRAIN_INFO = "- **acme** (team): ~/brains/acme"


def tags() -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(REPO), "tag", "--list", "v*", "--sort=v:refname"],
        check=True, capture_output=True, text=True,
    ).stdout.split()
    picked = []
    for tag in out:
        major = int(tag[1:].split(".")[0])
        if major in (1, 2):
            picked.append(tag)
        if tag == LAST:
            break
    return picked


def load(tag: str) -> types.ModuleType | None:
    src = subprocess.run(
        ["git", "-C", str(REPO), "show", f"{tag}:src/kluris/core/agents.py"],
        capture_output=True, text=True,
    )
    if src.returncode != 0:
        return None
    mod = types.ModuleType(f"agents_{tag.replace('.', '_')}")
    exec(compile(src.stdout, f"{tag}/agents.py", "exec"), mod.__dict__)
    return mod


def is_v2(mod: types.ModuleType) -> bool:
    return "brain_name" in mod.render_skill.__code__.co_varnames


def render(mod, skill_name: str, workflow: bool = False) -> str:
    fn = mod._render_workflow if workflow else mod.render_skill
    if not is_v2(mod):
        return fn(V1_BRAIN_INFO)
    return fn(
        skill_name=skill_name, brain_name=SAMPLE_BRAIN, brain_path="/home/dev/brains/acme",
        has_git=True, brain_description="Acme platform knowledge",
    )


def frontmatter(desc_template: str, style: str, skill: str, brain: str) -> str:
    desc = desc_template.replace("{brain_name}", brain)
    if style == "Folded":
        return (f"---\nname: {skill}\ndescription: >\n  {desc}\n"
                "allowed-tools: Read, Write, Bash, Glob, Grep\n---\n\n")
    if style == "Plain":
        return f"---\nname: {skill}\ndescription: {desc}\n---\n\n"
    return f'---\nname: {skill}\ndescription: "{desc.replace(chr(34), chr(92) + chr(34))}"\n---\n\n'


def workflow_frontmatter(desc_template: str, quoted: bool, brain: str) -> str:
    desc = desc_template.replace("{brain_name}", brain)[:200]
    if not quoted:
        return f"---\ndescription: {desc}\n---\n\n"
    return f'---\ndescription: "{desc.replace(chr(34), chr(92) + chr(34))}"\n---\n\n'


def headings_in_order(text: str, headings: list[str], brain: str) -> bool:
    lines = iter(text.split("\n"))
    return all(any(line == h.replace("{brain_name}", brain) for line in lines) for h in headings)


def era_of(tag: str, mod) -> tuple:
    desc = mod.SKILL_DESCRIPTION
    assert set(PLACEHOLDER.findall(desc)) <= {"{brain_name}"}, tag
    headings = [line for line in mod.SKILL_BODY.split("\n") if HEADING.match(line)]
    for h in headings:
        assert set(PLACEHOLDER.findall(h)) <= {"{brain_name}"}, (tag, h)
    v2 = is_v2(mod)
    skill = f"kluris-{SAMPLE_BRAIN}" if v2 else "kluris"
    text = render(mod, skill)
    styles = [st for st in ("Folded", "Plain", "Quoted")
              if text.startswith(frontmatter(desc, st, skill, SAMPLE_BRAIN))]
    assert len(styles) == 1, (tag, styles)
    style = styles[0]
    assert headings_in_order(text, headings, SAMPLE_BRAIN), tag
    workflow = "None"
    if hasattr(mod, "_render_workflow"):
        wf = render(mod, skill, workflow=True)
        quoted = wf.startswith('---\ndescription: "')
        workflow = "Quoted" if quoted else "Plain"
        assert wf.startswith(workflow_frontmatter(desc, quoted, SAMPLE_BRAIN)), tag
        assert headings_in_order(wf, headings, SAMPLE_BRAIN), tag
    return (desc, style, tuple(headings), workflow, v2)


def rust_str(s: str) -> str:
    assert all(ord(c) >= 0x20 for c in s), s
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main() -> None:
    eras: dict[tuple, list[str]] = {}
    mods: dict[str, types.ModuleType] = {}
    for tag in tags():
        mod = load(tag)
        if mod is None or not hasattr(mod, "SKILL_DESCRIPTION"):
            continue
        key = era_of(tag, mod)
        eras.setdefault(key, []).append(tag)
        mods.setdefault(tag, mod)
    if not eras:
        sys.exit("no releases found")

    FIXTURES.mkdir(parents=True, exist_ok=True)
    for old in FIXTURES.glob("*.md"):
        old.unlink()
    rows = []
    for (desc, style, headings, workflow, v2), releases in eras.items():
        first, last = releases[0], releases[-1]
        mod = mods[first]
        names = [f"kluris-{SAMPLE_BRAIN}", "kluris"] if v2 else ["kluris"]
        for skill in names:
            stem = first if skill != "kluris" or not v2 else f"{first}-bare"
            (FIXTURES / f"{stem}.md").write_bytes(render(mod, skill).encode("utf-8"))
            if workflow != "None":
                (FIXTURES / f"{stem}.workflow.md").write_bytes(
                    render(mod, skill, workflow=True).encode("utf-8"))
        heading_lines = "".join(f"            {rust_str(h)},\n" for h in headings)
        rows.append(
            "    Era {\n"
            f"        releases: {rust_str(first if first == last else f'{first}..={last}')},\n"
            f"        description: {rust_str(desc)},\n"
            f"        style: Style::{style},\n"
            f"        headings: &[\n{heading_lines}        ],\n"
            f"        workflow: Workflow::{workflow},\n"
            "    },\n"
        )
    RUST_OUT.write_text(
        "//! The SKILL.md shapes Kluris v1.0.0 to v2.28.11 wrote, one [`Era`] per\n"
        "//! group of releases that render alike. Generated by\n"
        "//! `parity/tools/gen_legacy_skills.py` from each release's own\n"
        "//! `agents.py`; do not edit.\n\n"
        "/// What every SKILL.md (and Windsurf workflow) of a group of releases\n"
        "/// shares.\n"
        "pub(super) struct Era {\n"
        "    /// The releases, `first..=last`.\n"
        "    #[allow(dead_code)]\n"
        "    pub releases: &'static str,\n"
        "    /// `SKILL_DESCRIPTION`; `{brain_name}` is the only slot.\n"
        "    pub description: &'static str,\n"
        "    /// How the SKILL.md frontmatter writes the description.\n"
        "    pub style: Style,\n"
        "    /// The heading lines (`#` to `###`) of `SKILL_BODY`, in order.\n"
        "    pub headings: &'static [&'static str],\n"
        "    /// How the release wrote Windsurf workflows.\n"
        "    pub workflow: Workflow,\n"
        "}\n\n"
        "/// The frontmatter of an era's SKILL.md: `name: <folder>` and the\n"
        "/// description.\n"
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n"
        "pub(super) enum Style {\n"
        "    /// `description: >` folded, then `allowed-tools: ...` (v1.0).\n"
        "    Folded,\n"
        "    /// `description: <text>` unquoted.\n"
        "    Plain,\n"
        "    /// `description: \"<text>\"`, `\"` escaped.\n"
        "    Quoted,\n"
        "}\n\n"
        "/// The frontmatter of an era's Windsurf workflow: `description:` and the\n"
        "/// first 200 characters of the description.\n"
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n"
        "pub(super) enum Workflow {\n"
        "    /// The release wrote none.\n"
        "    None,\n"
        "    /// Unquoted (v1.0).\n"
        "    Plain,\n"
        "    /// Quoted, `\"` escaped.\n"
        "    Quoted,\n"
        "}\n\n"
        "pub(super) const ERAS: &[Era] = &[\n" + "".join(rows) + "];\n",
        encoding="utf-8",
    )
    for key, releases in eras.items():
        print(f"{releases[0]}..{releases[-1]}: {len(key[2])} headings, workflow={key[3]}")


if __name__ == "__main__":
    main()
