"""Tests of release_notes.py, against the body dist 0.33.1 really writes.

    python3 -I -m unittest discover -s scripts/release -p 'test_*.py'

fixtures/plan-3.0.0-rc.1.json is `dist plan --tag=v3.0.0-rc.1
--output-format=json` cut to the announcement fields.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import release_notes  # noqa: E402

SH = "curl -fsSL https://kluris.ngvoicu.dev/install.sh | sh"
PS = "irm https://kluris.ngvoicu.dev/install.ps1 | iex"


def plan(version="3.0.0-rc.1", prerelease=True):
    with open(os.path.join(HERE, "fixtures", "plan-3.0.0-rc.1.json"), encoding="utf-8") as f:
        text = f.read()
    data = json.loads(text.replace("3.0.0-rc.1", version))
    data["announcement_is_prerelease"] = prerelease
    return data


class Body(unittest.TestCase):
    def test_leads_with_the_site_commands(self):
        body = release_notes.body(plan())
        first = body.split("\n## ", 1)[0]
        self.assertTrue(body.startswith("## Install kluris 3.0.0-rc.1\n"), body[:80])
        self.assertIn(SH, first)
        self.assertIn(PS, first)
        self.assertLess(first.index(SH), first.index(PS))

    def test_pins_this_release(self):
        body = release_notes.body(plan())
        self.assertIn("curl -fsSL https://kluris.ngvoicu.dev/install.sh | KLURIS_VERSION=3.0.0-rc.1 sh", body)
        self.assertIn("$env:KLURIS_VERSION = '3.0.0-rc.1'; " + PS, body)

    def test_drops_dists_long_install_section(self):
        body = release_notes.body(plan())
        self.assertNotIn("Install prebuilt binaries via shell script", body)
        self.assertNotIn("kluris-installer.sh", body)
        self.assertNotIn("-ExecutionPolicy Bypass", body)
        self.assertEqual(body.count("## Install kluris"), 1)

    def test_keeps_downloads_and_attestations(self):
        original = plan()["announcement_github_body"]
        body = release_notes.body(plan())
        tail = original[original.index("## Download kluris"):]
        self.assertTrue(body.endswith(tail))
        self.assertIn("gh attestation verify", body)

    def test_prerelease_says_the_plain_command_installs_the_newest_stable(self):
        self.assertIn("pre-release", release_notes.body(plan()))
        self.assertNotIn("pre-release", release_notes.body(plan("3.0.0", prerelease=False)))

    def test_stable_release(self):
        body = release_notes.body(plan("3.0.0", prerelease=False))
        self.assertTrue(body.startswith("## Install kluris 3.0.0\n"))
        self.assertIn("KLURIS_VERSION=3.0.0 sh", body)

    def test_unknown_layout_is_an_error(self):
        data = plan()
        data["announcement_github_body"] = "## Something else\n"
        with self.assertRaises(release_notes.LayoutError):
            release_notes.body(data)


class Command(unittest.TestCase):
    def run_script(self, data):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "plan.json")
            with open(path, "w", encoding="utf-8") as f:
                json.dump(data, f)
            return subprocess.run(
                [sys.executable, "-I", os.path.join(HERE, "release_notes.py"), path],
                capture_output=True,
                text=True,
            )

    def test_prints_the_body(self):
        result = self.run_script(plan())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, release_notes.body(plan()))

    def test_unknown_layout_exits_1(self):
        data = plan()
        data["announcement_github_body"] = "## Something else\n"
        result = self.run_script(data)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("release_notes", result.stderr)


if __name__ == "__main__":
    unittest.main()
