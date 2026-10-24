#!/usr/bin/env python3
"""Sync the embedded specmint companion skill folders into kluris.

Each companion ships as a folder: ``SKILL.md`` plus ``references/`` and
``assets/`` when the source has them. Everything else in a source repo
(.git, README.md, LICENSE, AGENTS.md, CLAUDE.md, evals/, skills/, dotfiles)
stays behind. Files are copied verbatim, except ``SKILL.md``, which gets the
sanitizer rules below. Every rule must match: a renamed heading or reworded
sentence upstream aborts the sync instead of silently shipping plugin text.
Two checks always run on the result: the sanitized ``SKILL.md`` must not
mention any of ``SIDE_CAR_PATTERNS``, and every ``references/...`` or
``assets/...`` path a shipped markdown file mentions must exist in the folder.

All four companions are built and checked in memory first; nothing under
``src/kluris/vendored/`` is touched unless every one of them passes.
"""

from __future__ import annotations

import argparse
import difflib
import fnmatch
import re
import shutil
import sys
import textwrap
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CORE = (ROOT.parent / "specmint/specmint-core").resolve()
DEFAULT_TDD = (ROOT.parent / "specmint/specmint-tdd").resolve()
DEFAULT_CORE_HTML = (ROOT.parent / "specmint/specmint-core-html").resolve()
DEFAULT_TDD_HTML = (ROOT.parent / "specmint/specmint-tdd-html").resolve()
DEST_ROOT = ROOT / "src" / "kluris" / "vendored"
# Top-level entries copied from a source repo; nothing else ships.
ALLOWED_ENTRIES = ("SKILL.md", "references", "assets")
# Plugin/marketplace text that must not survive in a vendored SKILL.md.
SIDE_CAR_PATTERNS = (
    "commands/",
    "agents/",
    ".claude-plugin",
    "plugin.json",
    "npx skills",
    "/plugin marketplace",
    "~/.claude/skills",
)
# `references/foo.md`, `assets/x.js`, `references/*.md`; not `.specs/assets/...`.
_PATH_MENTION = re.compile(r"(?<![\w./-])((?:references|assets)/[A-Za-z0-9_.*-]+)")

Tree = dict[str, bytes]


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Copy specmint companion skill folders into kluris vendored data."
    )
    parser.add_argument("--core", type=Path, default=DEFAULT_CORE)
    parser.add_argument("--tdd", type=Path, default=DEFAULT_TDD)
    parser.add_argument("--core-html", type=Path, default=DEFAULT_CORE_HTML)
    parser.add_argument("--tdd-html", type=Path, default=DEFAULT_TDD_HTML)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Build and diff only; write nothing. Exit 1 when the vendored copy is stale.",
    )
    parser.add_argument(
        "--diff",
        action="store_true",
        help="Print the full unified diff of every changed file, not just the summary.",
    )
    return parser.parse_args()


# --- sanitizer rules -------------------------------------------------------


def _compat_section(name: str) -> str:
    home = f"~/.kluris/companions/{name}"
    if name.endswith("-html"):
        scope = (
            f"Paths in this file that start with `references/` or `assets/` "
            f"resolve against `{home}/`. `.specs/...` paths, and the relative "
            "`../assets/` links inside a `SPEC.html`, are project paths."
        )
        format_note = (
            "`SPEC.html` stays plain HTML with a JSON metadata blob: open it in "
            "any browser, edit it in any text editor, and keep it diff-friendly "
            "with the conventions in `references/spec-format.md`."
        )
    else:
        scope = (
            f"Paths in this file that start with `references/` resolve against "
            f"`{home}/`. `.specs/...` paths are project paths."
        )
        format_note = (
            "Specs remain plain markdown/YAML files that humans can edit and Git can diff."
        )
    body = (
        f"Kluris embeds this playbook as the `{name}` companion. Read and follow "
        f"it directly from `{home}/SKILL.md`. {scope} "
        f"Do not ask the user to install specmint separately. {format_note}"
    )
    # Wrap like the upstream files; never split a `code span` or a hyphenated name.
    wrapped = textwrap.fill(body, width=75, break_long_words=False, break_on_hyphens=False)
    return f"\n## Kluris Companion Compatibility\n\n{wrapped}\n"


def _rule_compat_section(name: str, content: str) -> tuple[str, int]:
    """v3 `## Cross-Tool Compatibility`: `npx skills add` install lines per tool."""
    pattern = r"\n## Cross-Tool Compatibility\n.*?(?=\n## Behavioral Notes\n)"
    return re.subn(pattern, lambda _m: _compat_section(name), content, flags=re.S)


def _rule_asset_location(name: str, content: str) -> tuple[str, int]:
    """v3 html Step 1: "... (e.g. `~/.claude/skills/<name>/assets/`)"."""
    pattern = (
        r"(?:AI tools\s+should know the skill install location; for|For)"
        r" skill installs it\s+is typically the installed\s+skill's assets/ directory"
        rf" \(e\.g\.\s+`~/\.claude/skills/{re.escape(name)}/assets/`\)\."
    )
    # Both matches sit in a numbered list item, so continuation lines indent 3.
    replacement = f"Under Kluris that directory is\n   `~/.kluris/companions/{name}/assets/`."
    return re.subn(pattern, lambda _m: replacement, content)


@dataclass(frozen=True)
class Rule:
    label: str
    applies: Callable[[str], bool]
    apply: Callable[[str, str], tuple[str, int]]


RULES = (
    Rule("Cross-Tool Compatibility section", lambda n: True, _rule_compat_section),
    Rule("~/.claude/skills asset location", lambda n: n.endswith("-html"), _rule_asset_location),
)


def _sanitize_for_kluris(name: str, content: str) -> str:
    """Apply every rule for ``name``; each must match exactly once."""
    for rule in RULES:
        if not rule.applies(name):
            continue
        content, count = rule.apply(name, content)
        if count != 1:
            raise SystemExit(
                f"{name}: sanitizer rule '{rule.label}' matched {count} times "
                "(expected 1). Specmint's SKILL.md changed; review the rule."
            )
    return content


def _forbidden_hits(content: str) -> list[str]:
    return [pattern for pattern in SIDE_CAR_PATTERNS if pattern in content]


# --- building and checking a companion tree --------------------------------


def _read_source(name: str, source: Path) -> Tree:
    """Return the allowlisted source files as ``{posix relpath: bytes}``."""
    if not (source / "SKILL.md").is_file():
        raise SystemExit(f"{name}: missing SKILL.md at {source / 'SKILL.md'}")
    tree: Tree = {}
    for entry in ALLOWED_ENTRIES:
        path = source / entry
        if not path.exists():
            continue
        if path.is_symlink():
            raise SystemExit(f"{name}: refusing symlink {path}")
        if entry == "SKILL.md":
            tree[entry] = path.read_bytes()
            continue
        if not path.is_dir():
            raise SystemExit(f"{name}: {path} must be a directory")
        for file in sorted(path.rglob("*")):
            rel = file.relative_to(source)
            if any(part.startswith(".") or part == "__pycache__" for part in rel.parts):
                continue
            if file.is_symlink():
                raise SystemExit(f"{name}: refusing symlink {file}")
            if file.is_file():
                tree[rel.as_posix()] = file.read_bytes()
    skipped = sorted(p.name for p in source.iterdir() if p.name not in ALLOWED_ENTRIES)
    if skipped:
        print(f"{name}: not shipped: {', '.join(skipped)}")
    return tree


def _missing_mentions(tree: Tree) -> list[str]:
    """Return references/assets paths mentioned in markdown that the tree lacks."""
    missing = []
    for rel, data in tree.items():
        if not rel.endswith(".md"):
            continue
        for mention in _PATH_MENTION.findall(data.decode("utf-8")):
            mention = mention.rstrip(".")
            if mention.endswith("/"):
                continue
            if "*" in mention:
                if not fnmatch.filter(tree, mention):
                    missing.append(f"{rel}: {mention}")
            elif mention not in tree:
                missing.append(f"{rel}: {mention}")
    return sorted(set(missing))


def _build(name: str, source: Path) -> Tree:
    source = source.expanduser().resolve()
    tree = _read_source(name, source)

    content = _sanitize_for_kluris(name, tree["SKILL.md"].decode("utf-8"))
    output_hits = _forbidden_hits(content)
    if output_hits:
        raise SystemExit(
            f"{name}: sanitized SKILL.md still mentions forbidden paths: "
            f"{', '.join(output_hits)}"
        )
    tree["SKILL.md"] = content.encode("utf-8")

    missing = _missing_mentions(tree)
    if missing:
        raise SystemExit(f"{name}: mentioned files are not shipped: {'; '.join(missing)}")
    return tree


def _read_tree(root: Path) -> Tree:
    if not root.is_dir():
        return {}
    return {
        p.relative_to(root).as_posix(): p.read_bytes()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


def _line_count(data: bytes) -> int:
    return len(data.decode("utf-8", errors="replace").splitlines())


def _report(name: str, old: Tree, new: Tree, *, full_diff: bool) -> bool:
    """Print a per-file diff summary; return True when anything differs."""
    changed = False
    for rel in sorted(set(old) | set(new)):
        before, after = old.get(rel), new.get(rel)
        if before == after:
            continue
        changed = True
        if before is None:
            print(f"  added    {rel} ({len(after)} bytes, {_line_count(after)} lines)")
            continue
        if after is None:
            print(f"  removed  {rel} ({len(before)} bytes, {_line_count(before)} lines)")
            continue
        a = before.decode("utf-8", errors="replace").splitlines(keepends=True)
        b = after.decode("utf-8", errors="replace").splitlines(keepends=True)
        diff = list(difflib.unified_diff(
            a, b, fromfile=f"old/{name}/{rel}", tofile=f"new/{name}/{rel}"
        ))
        plus = sum(1 for line in diff if line.startswith("+") and not line.startswith("+++"))
        minus = sum(1 for line in diff if line.startswith("-") and not line.startswith("---"))
        print(f"  changed  {rel} (+{plus} -{minus} lines; {len(before)} -> {len(after)} bytes)")
        if full_diff:
            sys.stdout.writelines(diff)
    if not changed:
        print("  in sync")
    return changed


def _write(name: str, tree: Tree) -> None:
    """Replace ``DEST_ROOT/name`` with ``tree`` via stage-then-rename."""
    dest = DEST_ROOT / name
    staging = DEST_ROOT / f".{name}.tmp"
    backup = DEST_ROOT / f".{name}.old"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(backup, ignore_errors=True)
    try:
        for rel, data in tree.items():
            path = staging / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        if _read_tree(staging) != tree:
            raise SystemExit(f"{name}: staged copy does not match the built tree")
        if dest.exists():
            dest.replace(backup)
        staging.replace(dest)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        if backup.exists() and not dest.exists():
            backup.replace(dest)
        raise
    finally:
        shutil.rmtree(backup, ignore_errors=True)
    if _read_tree(dest) != tree:
        raise SystemExit(f"{name}: written copy does not match the built tree")


def main() -> int:
    args = _parse_args()
    sources = {
        "specmint-core": args.core,
        "specmint-tdd": args.tdd,
        "specmint-core-html": args.core_html,
        "specmint-tdd-html": args.tdd_html,
    }
    built = {name: _build(name, source) for name, source in sources.items()}

    stale = []
    for name, tree in built.items():
        print(f"{name}:")
        if _report(name, _read_tree(DEST_ROOT / name), tree, full_diff=args.diff):
            stale.append(name)

    if args.check:
        return 1 if stale else 0
    for name in stale:
        _write(name, built[name])
        print(f"synced {name} -> {DEST_ROOT / name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
