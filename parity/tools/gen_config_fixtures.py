"""Regenerate ``crates/kluris-core/tests/fixtures/config/`` with the v2.28.11 oracle.

Run from the repo root (the oracle venv must exist, e.g. after one parity run):

    parity/.oracle/venv/bin/python -I parity/tools/gen_config_fixtures.py

Every ``*.yml`` here is written by v2 itself (``create``, ``register``,
``companion add``, ``remove``) in an isolated HOME; the ``*-input.yml`` files
are the hand-edited files v2 was then given to read and rewrite. The sandbox
root is replaced by ``/kf`` so the fixtures carry no machine paths; the script
then checks, with v2's own ``kluris.core.config``, that reading and writing
each v2-written fixture gives back the same bytes, so the replacement did not
change anything v2 would write.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
VENV = REPO / "parity" / ".oracle" / "venv"
KLURIS = VENV / ("Scripts/kluris.exe" if sys.platform == "win32" else "bin/kluris")
OUT = REPO / "crates" / "kluris-core" / "tests" / "fixtures" / "config"
FAKE_ROOT = "/kf"

BETA_DESCRIPTION = (
    "Équipe données — connaissances partagées über alles, naïve café façade 日本語"
    " and a long tail to fold"
)


def main() -> None:
    root = Path(tempfile.mkdtemp(prefix="kluris-cfg-")).resolve()
    try:
        generate(root)
    finally:
        shutil.rmtree(root)
    verify()


def generate(root: Path) -> None:
    home = root / "home"
    brains = root / "brains"
    home.mkdir()
    brains.mkdir()
    config = home / ".kluris" / "config.yml"
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(home),
        "USERPROFILE": str(home),
        "KLURIS_CONFIG": str(config),
        "KLURIS_NO_PROMPT": "1",
        "GIT_CONFIG_GLOBAL": str(root / "gitconfig"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "PYTHONUTF8": "1",
        "TZ": "UTC",
    }
    (root / "gitconfig").write_text("", encoding="utf-8")

    def kluris(*args: str) -> None:
        proc = subprocess.run([str(KLURIS), *args, "--json"], env=env, capture_output=True, text=True)
        if proc.returncode != 0:
            raise SystemExit(f"oracle {args} failed: {proc.stdout} {proc.stderr}")

    def save(src: Path, name: str) -> None:
        text = src.read_text(encoding="utf-8").replace(str(root), FAKE_ROOT)
        (OUT / name).write_text(text, encoding="utf-8", newline="")

    def hand_write(path: Path, text: str, name: str) -> None:
        path.write_text(text, encoding="utf-8")
        save(path, name)

    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)

    kluris("create", "alpha", "--path", str(brains), "--no-git")
    kluris("create", "beta", "--path", str(brains), "--no-git", "--description", BETA_DESCRIPTION)
    gamma = brains / "gamma"
    gamma.mkdir()
    (gamma / "brain.md").write_text("---\nupdated: 2026-01-01\n---\n# gamma\n\nRegistered brain\n", encoding="utf-8")
    kluris("register", str(gamma))
    kluris("companion", "add", "specmint-tdd-html", "--brain", "beta")
    kluris("companion", "add", "specmint-core", "--brain", "beta")
    save(config, "global-three.yml")
    save(brains / "alpha" / "kluris.yml", "brain-created.yml")
    save(brains / "beta" / "kluris.yml", "brain-unicode-companions.yml")
    save(gamma / "kluris.yml", "brain-registered.yml")

    # Stale keys on brain entries and at the top level; v2 drops them on the next write.
    stale = config.read_text(encoding="utf-8")
    stale = stale.replace(
        "    description: alpha knowledge base\n",
        "    description: alpha knowledge base\n    type: product-group\n    repo: git@example.com:t/alpha.git\n",
    )
    hand_write(config, stale + "default_brain: alpha\n", "global-stale-input.yml")
    kluris("create", "delta", "--path", str(brains), "--no-git")
    save(config, "global-stale-rewritten.yml")

    # A legacy git: block and type: in kluris.yml.
    alpha_cfg = brains / "alpha" / "kluris.yml"
    hand_write(
        alpha_cfg,
        alpha_cfg.read_text(encoding="utf-8")
        + "type: product-group\ngit:\n  remote: git@example.com:t/alpha.git\n  branch: main\n",
        "brain-stale-input.yml",
    )
    kluris("companion", "add", "specmint-tdd", "--brain", "alpha")
    save(alpha_cfg, "brain-stale-rewritten.yml")

    # v3 agent names, an unknown one, and companions before name.
    delta_cfg = brains / "delta" / "kluris.yml"
    hand_write(
        delta_cfg,
        "companions: []\nagents:\n  commands_for:\n  - claude\n  - devin\n  - antigravity\n"
        "  - bogus-agent\ndescription: d\nname: delta\n",
        "brain-custom-agents-input.yml",
    )
    kluris("companion", "add", "specmint-core", "--brain", "delta")
    save(delta_cfg, "brain-custom-agents.yml")

    # The pre-Hermes default list (v2 migrates it to the current default).
    gamma_cfg = gamma / "kluris.yml"
    hand_write(
        gamma_cfg,
        "name: gamma\ndescription: Registered brain\nagents:\n  commands_for:\n"
        + "".join(f"  - {a}\n" for a in ("junie", "claude", "cursor", "windsurf", "copilot", "codex", "gemini", "kilocode"))
        + "companions: []\n",
        "brain-legacy-input.yml",
    )
    kluris("companion", "add", "specmint-core", "--brain", "gamma")
    save(gamma_cfg, "brain-legacy-rewritten.yml")

    # No companions key and no agents block; an empty description.
    hand_write(alpha_cfg, "name: alpha\ndescription: ''\n", "brain-minimal-input.yml")
    kluris("companion", "add", "specmint-core", "--brain", "alpha")
    save(alpha_cfg, "brain-minimal-rewritten.yml")

    # Removing every brain leaves an empty mapping.
    for name in ("alpha", "beta", "gamma", "delta"):
        kluris("remove", name)
    save(config, "global-empty.yml")


def verify() -> None:
    """v2 reads each v2-written fixture and writes back the same bytes."""
    code = r"""
import os, sys
from pathlib import Path
import tempfile
from kluris.core import config as c

out = Path(sys.argv[1])
bad = []
for f in sorted(out.glob("*.yml")):
    if f.name.endswith("-input.yml"):
        continue
    want = f.read_bytes()
    with tempfile.TemporaryDirectory() as d:
        if f.name.startswith("global-"):
            os.environ["KLURIS_CONFIG"] = str(Path(d) / "config.yml")
            Path(os.environ["KLURIS_CONFIG"]).write_bytes(want)
            c.write_global_config(c.read_global_config())
            got = Path(os.environ["KLURIS_CONFIG"]).read_bytes()
        else:
            (Path(d) / "kluris.yml").write_bytes(want)
            c.write_brain_config(c.read_brain_config(Path(d)), Path(d))
            got = (Path(d) / "kluris.yml").read_bytes()
    if got != want:
        bad.append(f.name)
if bad:
    raise SystemExit(f"v2 does not round-trip: {bad}")
print("verified", len(list(out.glob("*.yml"))), "fixtures")
"""
    python = VENV / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")
    subprocess.run([str(python), "-I", "-c", code, str(OUT)], check=True)


if __name__ == "__main__":
    main()
