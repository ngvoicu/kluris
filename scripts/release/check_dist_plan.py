"""Check `dist plan --output-format=json` against what a Kluris 3 release ships.

    dist plan --output-format=json > plan.json
    python3 -I scripts/release/check_dist_plan.py plan.json

Prints one line per problem and exits 1 when there is any; prints the release
it checked and exits 0 otherwise. The rules are SPEC TEST-K3-37/IMPL-K3-38
and research-02 section 1: the eight targets, tar.gz on Unix and zip on
Windows, a sha256 per archive and one sha256.sum, the shell and PowerShell
installers named after `kluris` (so their variables are KLURIS_*), no
updater, attestations, and LICENSE, THIRD_PARTY_NOTICES.md and
THIRD_PARTY_LICENSES.md beside the binary in every archive.
"""

import json
import sys

APP = "kluris"

# Each target and the GitHub runner that builds it: arm builds on arm
# runners, and no runner label GitHub has deprecated (macos-14).
RUNNERS = {
    "aarch64-apple-darwin": "macos-15",
    "x86_64-apple-darwin": "macos-15-intel",
    "x86_64-unknown-linux-gnu": "ubuntu-22.04",
    "x86_64-unknown-linux-musl": "ubuntu-22.04",
    "aarch64-unknown-linux-gnu": "ubuntu-22.04-arm",
    "aarch64-unknown-linux-musl": "ubuntu-22.04-arm",
    "x86_64-pc-windows-msvc": "windows-2025",
    "aarch64-pc-windows-msvc": "windows-11-arm",
}

NOTICES = ("LICENSE", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.md")

ENV = {
    "install_dir_env_var": "KLURIS_INSTALL_DIR",
    "download_url_env_var": "KLURIS_DOWNLOAD_URL",
    "no_modify_path_env_var": "KLURIS_NO_MODIFY_PATH",
    "unmanaged_dir_env_var": "KLURIS_UNMANAGED_INSTALL",
}


def problems(plan: dict) -> list[str]:
    found: list[str] = []
    releases = plan.get("releases") or []
    if [r.get("app_name") for r in releases] != [APP]:
        found.append(f"releases: expected one release of {APP!r}, got {[r.get('app_name') for r in releases]}")
        return found
    release = releases[0]
    for key, want in ENV.items():
        got = release.get("env", {}).get(key)
        if got != want:
            found.append(f"env {key}: expected {want}, got {got}")

    artifacts = plan.get("artifacts") or {}
    names = set(release.get("artifacts") or [])
    for name in (f"{APP}-installer.sh", f"{APP}-installer.ps1", "sha256.sum"):
        if name not in names:
            found.append(f"missing artifact {name}")
    for name in names:
        if "update" in name:
            found.append(f"unexpected updater artifact {name}")

    archives = {
        name: art
        for name, art in artifacts.items()
        if name in names and art.get("kind") == "executable-zip"
    }
    by_target = {}
    for name, art in archives.items():
        for triple in art.get("target_triples") or []:
            by_target.setdefault(triple, []).append(name)
    if sorted(by_target) != sorted(RUNNERS):
        found.append(f"archive targets: expected {sorted(RUNNERS)}, got {sorted(by_target)}")
    for triple, names_for in sorted(by_target.items()):
        windows = "windows" in triple
        want = f"{APP}-{triple}.{'zip' if windows else 'tar.gz'}"
        if names_for != [want]:
            found.append(f"{triple}: expected archive {want}, got {names_for}")
            continue
        art = archives[want]
        if art.get("checksum") != f"{want}.sha256":
            found.append(f"{want}: expected checksum {want}.sha256, got {art.get('checksum')}")
        assets = [a.get("path") for a in art.get("assets") or []]
        exe = "kluris.exe" if windows else "kluris"
        if exe not in assets:
            found.append(f"{want}: no {exe}")
        for notice in NOTICES:
            count = assets.count(notice)
            if count != 1:
                found.append(f"{want}: {notice} {count} times, expected once")

    if plan.get("github_attestations") is not True:
        found.append(f"github_attestations: expected true, got {plan.get('github_attestations')}")

    matrix = (((plan.get("ci") or {}).get("github") or {}).get("artifacts_matrix") or {}).get("include") or []
    runners = {}
    for job in matrix:
        for triple in job.get("targets") or []:
            runners[triple] = job.get("runner")
    for triple, want in RUNNERS.items():
        if runners.get(triple) != want:
            found.append(f"runner for {triple}: expected {want}, got {runners.get(triple)}")
    return found


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as handle:
        plan = json.load(handle)
    found = problems(plan)
    for line in found:
        print(f"dist plan: {line}")
    if found:
        return 1
    release = plan["releases"][0]
    print(f"dist plan: ok, {release['app_name']} {release['app_version']}, {len(RUNNERS)} targets")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
