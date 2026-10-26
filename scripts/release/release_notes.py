"""Print a release's GitHub body so it leads with the site's install commands.

    python3 -I scripts/release/release_notes.py plan.json > notes.md

plan.json is dist's manifest (`dist plan`/`dist host --output-format=json`).
The body is dist's own `announcement_github_body` with its "## Install
kluris <version>" section (the long installer commands from GitHub) replaced
by the two short commands from kluris.ngvoicu.dev and their KLURIS_VERSION
form for this release; the download table and the attestation notes stay as
dist wrote them. release-notes.yml puts it on the release with `gh release
edit --notes-file`.

Exits 1, printing nothing, when dist's body does not have the layout dist
0.33.1 writes: the release then keeps dist's body.
"""

import json
import sys

SITE = "https://kluris.ngvoicu.dev"


class LayoutError(Exception):
    pass


def install_section(version, prerelease):
    latest = "the newest stable release" if prerelease else "the newest release"
    lines = [
        f"## Install kluris {version}",
        "",
        "macOS and Linux:",
        "",
        "```sh",
        f"curl -fsSL {SITE}/install.sh | sh",
        "```",
        "",
        "Windows (PowerShell):",
        "",
        "```powershell",
        f"irm {SITE}/install.ps1 | iex",
        "```",
        "",
        f"Either one installs {latest} into `~/.kluris/bin` and adds it to PATH;"
        " open a new terminal, then run `kluris doctor`."
        " An installed kluris updates itself with `kluris self-update`.",
        "",
    ]
    if prerelease:
        lines += [f"{version} is a pre-release. To install it:", ""]
    else:
        lines += [f"To install {version} itself:", ""]
    lines += [
        "```sh",
        f"curl -fsSL {SITE}/install.sh | KLURIS_VERSION={version} sh",
        "```",
        "",
        "```powershell",
        f"$env:KLURIS_VERSION = '{version}'; irm {SITE}/install.ps1 | iex",
        "```",
        "",
    ]
    return "\n".join(lines) + "\n"


def body(plan):
    original = plan["announcement_github_body"]
    tag = plan["announcement_tag"]
    version = tag[1:] if tag.startswith("v") else tag
    install = original.find(f"## Install kluris {version}\n")
    download = original.find(f"## Download kluris {version}\n")
    if install < 0 or download < install:
        raise LayoutError(
            f"dist's body for {tag} has no '## Install kluris {version}' section"
            f" followed by '## Download kluris {version}'"
        )
    before = original[:install]
    return install_section(version, plan["announcement_is_prerelease"]) + before + original[download:]


def main(argv):
    if len(argv) != 2:
        print("usage: release_notes.py plan.json", file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as f:
        plan = json.load(f)
    try:
        text = body(plan)
    except LayoutError as error:
        print(f"release_notes: {error}", file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
