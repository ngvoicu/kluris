"""Tests of build-local.ps1's pinned dist fetch, run by a real PowerShell.

    python3 -I -m unittest discover -s scripts/release -p 'test_*.py'

Each case runs build-local.ps1 against a scratch KLURIS_TOOLS_DIR with the
network stubbed out: Invoke-WebRequest copies a fixture zip in place of the
download and Get-FileHash answers the pinned digest, so nothing leaves the
machine. The script stops right after the fetch -- the stubbed rustc names
no host target, so 'rustc -vV names no host target' in the error output is
what shows the fetch block was passed, and what it left behind is what each
test checks. Skipped without a powershell or pwsh on PATH (GitHub's runners
have pwsh).
"""

import base64
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "build-local.ps1")

# The dist.zip build-local.ps1 pins: its SHA-256 line, verbatim.
DIST_SHA256 = "79dfcce84e6a8e69210f11fc23d445c3aac7af6a29e0e981867f053c0a869316"
DIST_DIR = "dist-0.33.1-windows-x86_64"
DIST_EXE = os.path.join(DIST_DIR, "dist.exe")

# powershell is Windows PowerShell 5.1, the interpreter the workflow calls;
# pwsh is what non-Windows machines (GitHub's ubuntu and macOS runners among
# them) have.
PWSH = shutil.which("powershell") or shutil.which("pwsh")

# The harness's own words: stubs for the two cmdlets that would reach the
# network and for rustc, whose answer (no 'host: *' line) ends the script
# right after the fetch block, before it would run dist or touch target/.
HARNESS = """\
function Invoke-WebRequest {
    param($Uri, $OutFile, $TimeoutSec, [switch]$UseBasicParsing)
    Copy-Item -LiteralPath $env:KLURIS_TEST_ZIP -Destination $OutFile
}
function Get-FileHash {
    param($Algorithm, $LiteralPath)
    [pscustomobject]@{ Hash = '""" + DIST_SHA256.upper() + """' }
}
function rustc { 'rustc 1.99.0 (00000000 2026-01-01)' }
& $env:KLURIS_TEST_SCRIPT
"""


def zip_with(names):
    """A fixture zip in the temp dir holding the named flat entries."""
    fd, path = tempfile.mkstemp(suffix=".zip")
    with os.fdopen(fd, "wb") as f:
        with zipfile.ZipFile(f, "w") as z:
            for name in names:
                z.writestr(name, b"fake\n")
    return path


@unittest.skipUnless(PWSH, "needs powershell or pwsh on PATH")
class DistFetch(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tools = os.path.join(tmp.name, "tools")
        self.dist_dir = os.path.join(self.tools, DIST_DIR)
        self.dist_exe = os.path.join(self.tools, DIST_EXE)
        os.makedirs(self.tools)

    def run_script(self, fixture_zip):
        env = dict(
            os.environ,
            KLURIS_TOOLS_DIR=self.tools,
            KLURIS_TEST_ZIP=fixture_zip,
            KLURIS_TEST_SCRIPT=SCRIPT,
        )
        encoded = base64.b64encode(HARNESS.encode("utf-16-le")).decode("ascii")
        result = subprocess.run(
            [PWSH, "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
            capture_output=True,
            text=True,
            env=env,
        )
        return result, result.stdout + result.stderr

    def test_replaces_a_stale_tools_dir(self):
        # rust-cache can restore dist-0.33.1-windows-x86_64\ without
        # dist.exe; the fetched .part folder must replace it, not move
        # inside it (PowerShell moves a folder into an existing -Destination).
        os.makedirs(self.dist_dir)
        with open(os.path.join(self.dist_dir, "stale.txt"), "w", encoding="utf-8") as f:
            f.write("restored by rust-cache\n")
        result, output = self.run_script(zip_with(["dist.exe"]))
        self.assertIn("rustc -vV names no host target", output)
        self.assertTrue(os.path.isfile(self.dist_exe), output)
        self.assertFalse(os.path.exists(os.path.join(self.dist_dir, "stale.txt")))
        nested = os.path.join(self.dist_dir, DIST_DIR + ".part", "dist.exe")
        self.assertFalse(os.path.exists(nested), "the .part folder moved inside the stale one")

    def test_fetches_into_an_empty_tools_dir(self):
        result, output = self.run_script(zip_with(["dist.exe"]))
        self.assertIn("rustc -vV names no host target", output)
        self.assertTrue(os.path.isfile(self.dist_exe), output)

    def test_a_zip_without_dist_exe_fails_loudly(self):
        result, output = self.run_script(zip_with(["readme.txt"]))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dist.exe", output)
        self.assertNotIn("rustc -vV names no host target", output)

    def test_keeps_a_fetched_dist(self):
        os.makedirs(self.dist_dir)
        with open(self.dist_exe, "wb") as f:
            f.write(b"MZ fake\n")
        # A zip with no dist.exe: the fetch must not run at all, or this
        # would fail the way test_a_zip_without_dist_exe_fails_loudly does.
        result, output = self.run_script(zip_with(["readme.txt"]))
        self.assertIn("rustc -vV names no host target", output)


if __name__ == "__main__":
    unittest.main()
