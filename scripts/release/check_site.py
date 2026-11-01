"""Check that the website says only what this kluris binary does.

    python3 -I scripts/release/check_site.py --bin <kluris> <site dir>

A hit fails the check (exit 1), each named as `<file>:<line>: <what>`:

- a page (every `*.html` at the site's top level, and `README.md`) names
  a removed or retired thing: `--watch`, `install-service`, the product
  name followed by a major version number, `v` and that number as a word,
  `PyPI` or `pipx`;
- the site's `AGENTS.md` differs from its `CLAUDE.md`;
- a page shows `kluris <command> [--flag ...]` where the command is not in
  `kluris help --json` (for `companion`, its subcommand is not one of
  `kluris companion --help`), or a flag is not in that command's `--help`.

Where commands are read: in HTML, the text of `<pre>`, `<code>` and every
element whose class has `font-mono`, `cmd`, `miniterm` or
`surface-example` (nested tags stripped, entities decoded, `<br>` a line
break); in `README.md`, inline backticks and fenced code blocks with no
language or a shell or text one (a `zone` or `yaml` block is data). A command
is `kluris` as a word (not `/kluris-acme`, `kluris.yml` or `kluris:`), a
space, then a lowercase word. Its flags are the words that start with `-`
up to the end of the line or a separator (`;`, `&&`, `|` between words,
`#`, `·`, `→`, `—`, `)`, the next `kluris`); `--a|--b` is two flags. What
the parse misses, and the check prints: flags shown without `kluris
<command>` before them in the same code element (`--brain all` on its
own), commands in prose outside code, Markdown fences in other
languages, the rows of `.cmd-table` tables
(the CLI reference names commands without `kluris`), and screenshots.
"""

from __future__ import annotations

import argparse
import difflib
import html
import html.parser
import json
import re
import subprocess
import sys
from pathlib import Path

# The words a page must not hold. Written so this file does not hold them.
TERMS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"--watch\b"), "the watch mode"),
    (re.compile(r"install-service"), "the sync service"),
    (re.compile(r"\bKluris [23]\b"), "a major version in the product name"),
    (re.compile(r"\bv[2]\b"), "a major version as a word"),
    (re.compile(r"PyPI", re.IGNORECASE), "the Python package index"),
    (re.compile(r"\bpipx\b", re.IGNORECASE), "pipx"),
]

CODE_TAGS = {"pre", "code"}
CODE_CLASSES = {"font-mono", "cmd", "miniterm", "surface-example"}
VOID_TAGS = {
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link",
    "meta", "source", "track", "wbr",
}

COMMAND = re.compile(r"(?<![\w/.\-])kluris[ \t]+([a-z][a-z-]*|--?[a-z][a-z-]*)")
FLAG = re.compile(r"^(--?[A-Za-z][A-Za-z0-9-]*)")
STOP_WORDS = {"#", "|", "||", "&&", ";", "·", "→", "—", "kluris"}
STOP_CHARS = (";", "#", ")", "·", "→", "—")
HELP_FLAG = re.compile(r"^\s+(?:(-[A-Za-z]),\s+)?(--[a-z][a-z0-9-]*)", re.MULTILINE)


class Snippet:
    """A piece of code text from a page, and the line each offset is on."""

    def __init__(self, path: str) -> None:
        self.path = path
        self.parts: list[tuple[int, str]] = []

    def add(self, line: int, text: str) -> None:
        self.parts.append((line, text))

    @property
    def text(self) -> str:
        return "".join(t for _, t in self.parts)

    def line_at(self, offset: int) -> int:
        seen = 0
        for line, text in self.parts:
            if offset < seen + len(text):
                return line + text.count("\n", 0, offset - seen)
            seen += len(text)
        return self.parts[-1][0] if self.parts else 1


class CodeParser(html.parser.HTMLParser):
    """Collects the text of the code elements of one HTML page."""

    def __init__(self, path: str) -> None:
        super().__init__(convert_charrefs=True)
        self.path = path
        self.stack: list[tuple[str, bool]] = []
        self.depth = 0
        self.current: Snippet | None = None
        self.snippets: list[Snippet] = []

    def handle_starttag(self, tag, attrs):
        if tag == "br":
            if self.current is not None:
                self.current.add(self.getpos()[0], "\n")
            return
        if tag in VOID_TAGS:
            return
        classes = set((dict(attrs).get("class") or "").split())
        is_code = tag in CODE_TAGS or bool(classes & CODE_CLASSES)
        if is_code:
            if self.depth == 0:
                self.current = Snippet(self.path)
            self.depth += 1
        self.stack.append((tag, is_code))

    def handle_endtag(self, tag):
        if not any(t == tag for t, _ in self.stack):
            return
        while self.stack:
            open_tag, is_code = self.stack.pop()
            if is_code:
                self.depth -= 1
                if self.depth == 0 and self.current is not None:
                    self.snippets.append(self.current)
                    self.current = None
            if open_tag == tag:
                break

    def handle_data(self, data):
        if self.current is not None:
            self.current.add(self.getpos()[0], data)


def html_snippets(path: str, text: str) -> list[Snippet]:
    parser = CodeParser(path)
    parser.feed(text)
    parser.close()
    return parser.snippets


INLINE = re.compile(r"(`+)(.+?)\1")


# Fence languages read for commands; a block in any other (`zone`, `yaml`,
# `json`) is data, not a command line.
SHELL_FENCES = {"", "sh", "bash", "zsh", "fish", "shell", "console", "text", "powershell", "ps1", "cmd"}


def markdown_snippets(path: str, text: str) -> list[Snippet]:
    snippets: list[Snippet] = []
    fence: Snippet | None = None
    skipping = False
    for number, line in enumerate(text.splitlines(keepends=True), start=1):
        stripped = line.lstrip()
        if stripped.startswith("```"):
            if fence is None and not skipping:
                if stripped[3:].strip().lower() in SHELL_FENCES:
                    fence = Snippet(path)
                else:
                    skipping = True
            elif skipping:
                skipping = False
            else:
                snippets.append(fence)
                fence = None
            continue
        if skipping:
            continue
        if fence is not None:
            fence.add(number, line)
            continue
        for match in INLINE.finditer(line):
            snippet = Snippet(path)
            snippet.add(number, match.group(2))
            snippets.append(snippet)
    if fence is not None:
        snippets.append(fence)
    return snippets


def normalize(text: str) -> str:
    """Hyphens a page may print instead of `-` read as `-`."""
    return text.replace("‐", "-").replace("‑", "-")


def commands_in(snippet: Snippet):
    """Yield (line, command, flags) for each `kluris <command>`."""
    text = normalize(snippet.text)
    for match in COMMAND.finditer(text):
        command = match.group(1)
        rest = text[match.end():].split("\n", 1)[0]
        words: list[str] = []
        for word in rest.split():
            if word in STOP_WORDS:
                break
            cut = min((word.find(c) for c in STOP_CHARS if c in word), default=-1)
            if cut == 0:
                break
            if cut > 0:
                words.append(word[:cut])
                break
            words.append(word)
        flags = []
        for word in words:
            for piece in re.split(r"[|\[\]]", word):
                found = FLAG.match(piece)
                if found and not re.match(r"^-\d", piece):
                    flags.append(found.group(1))
        sub = next((w for w in words if re.fullmatch(r"[a-z][a-z-]*", w)), None)
        yield snippet.line_at(match.start()), command, sub, flags


def loose_flags(snippet: Snippet) -> set[str]:
    """The flags of a snippet with no `kluris <command>` in it."""
    text = normalize(snippet.text)
    if COMMAND.search(text):
        return set()
    return {m.group(1) for m in re.finditer(r"(?<![\w-])(--[a-z][a-z0-9-]*)", text)}


class Cli:
    """What the binary says it takes: commands, subcommands, flags."""

    def __init__(self, commands: dict[str, set[str]], subcommands: dict[str, dict[str, set[str]]], root_flags: set[str]):
        self.commands = commands
        self.subcommands = subcommands
        self.root_flags = root_flags

    @classmethod
    def from_binary(cls, binary: str) -> "Cli":
        def run(*args: str) -> str:
            done = subprocess.run(
                [binary, *args], capture_output=True, text=True, timeout=60,
                env={"KLURIS_NO_PROMPT": "1", "PATH": "/usr/bin:/bin", "HOME": "/nonexistent"},
            )
            if done.returncode != 0:
                raise SystemExit(f"check_site: `kluris {' '.join(args)}` exited {done.returncode}: {done.stderr.strip()}")
            return done.stdout

        names = [c["name"] for c in json.loads(run("help", "--json"))["commands"]]
        commands = {name: help_flags(run(name, "--help")) for name in names}
        subcommands: dict[str, dict[str, set[str]]] = {}
        for name in names:
            listed = subcommand_names(run(name, "--help"))
            if listed:
                subcommands[name] = {sub: help_flags(run(name, sub, "--help")) for sub in listed}
        return cls(commands, subcommands, help_flags(run("--help")) | {"--version", "--help"})

    def problems(self, command: str, sub: str | None, flags: list[str]) -> list[str]:
        if command.startswith("-"):
            return [] if command in self.root_flags else [f"`kluris {command}`: kluris takes no {command}"]
        if command not in self.commands:
            return [f"`kluris {command}`: no such command in `kluris help --json`"]
        known = self.commands[command]
        label = command
        if command in self.subcommands and sub is not None:
            if sub not in self.subcommands[command]:
                return [f"`kluris {command} {sub}`: no such subcommand in `kluris {command} --help`"]
            known = self.subcommands[command][sub]
            label = f"{command} {sub}"
        out = []
        for flag in flags:
            if flag not in known:
                out.append(f"`kluris {label} {flag}`: {flag} is not in `kluris {label} --help`")
        return out


def help_flags(text: str) -> set[str]:
    flags: set[str] = set()
    for short, long in HELP_FLAG.findall(text):
        flags.add(long)
        if short:
            flags.add(short)
    return flags


def subcommand_names(text: str) -> list[str]:
    match = re.search(r"^Commands:\n((?:  \S.*\n?)+)", text, re.MULTILINE)
    if not match:
        return []
    return [line.split()[0] for line in match.group(1).splitlines() if line.split() and line.split()[0] != "help"]


def pages(site: Path) -> list[Path]:
    found = sorted(site.glob("*.html"))
    readme = site / "README.md"
    if readme.is_file():
        found.append(readme)
    return found


def check(site: Path, cli: Cli) -> tuple[list[str], dict]:
    hits: list[str] = []
    stats = {"pages": 0, "commands": 0, "loose": set()}
    for page in pages(site):
        stats["pages"] += 1
        rel = page.name
        text = page.read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), start=1):
            plain = normalize(html.unescape(line))
            for pattern, what in TERMS:
                for found in {m.group(0) for m in pattern.finditer(plain)}:
                    hits.append(f"{rel}:{number}: `{found}` ({what})")
        snippets = markdown_snippets(rel, text) if page.suffix == ".md" else html_snippets(rel, text)
        for snippet in snippets:
            stats["loose"] |= loose_flags(snippet)
            for number, command, sub, flags in commands_in(snippet):
                stats["commands"] += 1
                for problem in cli.problems(command, sub, flags):
                    hits.append(f"{rel}:{number}: {problem}")
    claude, agents = site / "CLAUDE.md", site / "AGENTS.md"
    if claude.is_file() or agents.is_file():
        a = claude.read_bytes() if claude.is_file() else b""
        b = agents.read_bytes() if agents.is_file() else b""
        if a != b:
            diff = list(difflib.unified_diff(
                a.decode("utf-8", "replace").splitlines(), b.decode("utf-8", "replace").splitlines(),
                "CLAUDE.md", "AGENTS.md", n=0, lineterm=""))
            changed = sum(1 for d in diff if d[:1] in "+-" and not d.startswith(("+++", "---")))
            hits.append(f"AGENTS.md: differs from CLAUDE.md ({changed} lines); first: {next((d for d in diff[2:] if d[:1] in '+-'), '').strip()[:120]}")
    return hits, stats


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", required=True, help="the kluris binary to ask")
    parser.add_argument("site", type=Path)
    args = parser.parse_args(argv)
    if not args.site.is_dir():
        print(f"check_site: {args.site} is not a folder", file=sys.stderr)
        return 2
    cli = Cli.from_binary(args.bin)
    hits, stats = check(args.site, cli)
    if stats["commands"] == 0:
        hits.append("no `kluris <command>` found on any page: the parse is broken")
    for hit in hits:
        print(f"check_site: {hit}", file=sys.stderr)
    loose = sorted(stats["loose"])
    print(
        f"check_site: {stats['pages']} pages, {stats['commands']} `kluris <command>` checked "
        f"against {len(cli.commands)} commands."
    )
    print(
        "check_site: not checked: commands in prose outside code, Markdown fences in other languages, "
        "`.cmd-table` rows, screenshots, "
        f"and flags with no `kluris <command>` in their code element ({', '.join(loose) or 'none'})."
    )
    if hits:
        print(f"check_site: {len(hits)} problem(s)", file=sys.stderr)
        return 1
    print("check_site: the site matches this binary")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
