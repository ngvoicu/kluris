"""check_site.py's parse and rules, on small pages and a canned CLI."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_site  # noqa: E402

CLI = check_site.Cli(
    commands={
        "sync": {"--brain", "--no-verify", "--json", "--help"},
        "resolve": {"--brain", "--ours", "--theirs", "--file", "--merge", "--json", "--help"},
        "commit": {"--brain", "--paths", "-m", "--message", "--json", "--help"},
        "companion": {"--help"},
    },
    subcommands={"companion": {"add": {"--brain", "--json", "--help"}, "list": {"--brain", "--help"}}},
    root_flags={"--help", "--version"},
)

# The retired words, put together here so this file holds none of them.
MAJOR = "Kluris " + "3"
WORD = "v" + "2"


def problems_in_html(text):
    out = []
    for snippet in check_site.html_snippets("p.html", text):
        for line, command, sub, flags in check_site.commands_in(snippet):
            out += [(line, p) for p in CLI.problems(command, sub, flags)]
    return out


class Parse(unittest.TestCase):
    def test_nested_spans_are_one_command_and_the_line_is_kept(self):
        page = '<p>x</p>\n<pre><span class="prompt">$</span> <span class="cmd">kluris sync\n</span>' \
               '<span class="cmd">kluris sync <span class="w">--brain acme</span> --watch</span></pre>'
        found = problems_in_html(page)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0][0], 3)
        self.assertIn("--watch is not in `kluris sync --help`", found[0][1])

    def test_skill_names_files_and_output_prefixes_are_not_commands(self):
        page = '<pre>/kluris-acme learn\nkluris.yml\nkluris: acme: pushed\nkluris-acme</pre>'
        snippet = check_site.html_snippets("p.html", page)[0]
        self.assertEqual(list(check_site.commands_in(snippet)), [])

    def test_alternatives_split_and_separators_end_a_command(self):
        page = '<pre>kluris resolve --brain acme a.md --ours|--theirs|--file FILE --merge 1a2b' \
               ' · kluris sync --brain acme # --nope\nkluris commit --paths a.md | grep --nope</pre>'
        self.assertEqual(problems_in_html(page), [])

    def test_font_mono_spans_and_entities_are_read(self):
        page = '<p>Run <span class="font-mono">kluris resolve &lt;path&gt; --abort</span>.</p>'
        self.assertIn("--abort is not in", problems_in_html(page)[0][1])

    def test_unknown_command_and_subcommand(self):
        page = '<code>kluris pack</code> <code>kluris companion remove x</code> <code>kluris companion add x --brain all</code>'
        found = [p for _, p in problems_in_html(page)]
        self.assertEqual(len(found), 2)
        self.assertIn("`kluris pack`: no such command", found[0])
        self.assertIn("`kluris companion remove`: no such subcommand", found[1])

    def test_markdown_reads_shell_fences_and_inline_code_only(self):
        text = "Run `kluris sync --nope`.\n\n```sh\nkluris commit -m x --nope\n```\n\n```zone\nCNAME  kluris  host\n```\n"
        found = []
        for snippet in check_site.markdown_snippets("README.md", text):
            for line, command, sub, flags in check_site.commands_in(snippet):
                found += [(line, p) for p in CLI.problems(command, sub, flags)]
        self.assertEqual([line for line, _ in found], [1, 4])

    def test_loose_flags_are_reported_not_checked(self):
        snippet = check_site.html_snippets("p.html", '<span class="font-mono">--brain all</span>')[0]
        self.assertEqual(check_site.loose_flags(snippet), {"--brain"})


class Rules(unittest.TestCase):
    def site(self, files):
        root = Path(tempfile.mkdtemp())
        for name, text in files.items():
            (root / name).write_text(text, encoding="utf-8")
        return root

    def test_retired_words_on_pages_only(self):
        root = self.site({
            "index.html": f"<p>{MAJOR} ships</p>\n<p>see {WORD} and PyPI</p>\n<p>pipx install</p>",
            "README.md": "run kluris sync &#45;-watch\n",
            "CLAUDE.md": f"{MAJOR}\n",
            "AGENTS.md": f"{MAJOR}\n",
        })
        hits, _ = check_site.check(root, CLI)
        self.assertEqual(len(hits), 5, hits)
        self.assertTrue(any(h.startswith("README.md:1:") for h in hits))
        self.assertFalse(any("CLAUDE.md" in h for h in hits))

    def test_a_version_after_the_name_is_caught_but_a_bare_one_is_not(self):
        root = self.site({"index.html": f"<p>{MAJOR}.2.0</p><p>KLURIS_VERSION=3.2.0</p>"})
        hits, _ = check_site.check(root, CLI)
        self.assertEqual(len(hits), 1, hits)

    def test_agents_must_match_claude(self):
        root = self.site({"index.html": "<p>ok</p>", "CLAUDE.md": "# a\nsame\n", "AGENTS.md": "# b\nsame\n"})
        hits, _ = check_site.check(root, CLI)
        self.assertEqual(len(hits), 1)
        self.assertTrue(hits[0].startswith("AGENTS.md: differs from CLAUDE.md"))

    def test_help_parsing(self):
        text = "Usage: kluris commit [OPTIONS]\n\nOptions:\n      --brain <BRAIN>\n  -m, --message <MSG>\n      --help\n"
        self.assertEqual(check_site.help_flags(text), {"--brain", "-m", "--message", "--help"})
        self.assertEqual(
            check_site.subcommand_names("Usage: x\n\nCommands:\n  list    a\n  add     b\n  help    c\n\nOptions:\n"),
            ["list", "add"],
        )


if __name__ == "__main__":
    unittest.main()
