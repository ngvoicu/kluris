#!/bin/sh
# Install a release PUBLISHED on GitHub through the LIVE site wrapper, on
# macOS or Linux, in a sandbox HOME (never the real one), check it and
# uninstall it:
#
#   scripts/release/published-smoke.sh [--unpinned] <version> [<site install.sh URL>]
#
# The site URL defaults to https://kluris.ngvoicu.dev/install.sh. The wrapper
# runs the documented way, `curl -fsSL <url> | sh`, with KLURIS_VERSION set
# to <version>, so it fetches github.com/ngvoicu/kluris/releases/download/
# v<version>/kluris-installer.sh, which downloads this machine's archive.
# With --unpinned it runs without KLURIS_VERSION, as a user's first install
# does, so it installs GitHub's latest release; every check below still
# expects <version>.
#
# Checks: the installer verified the archive's SHA-256 and printed its
# success message; the receipt names ~/.kluris/bin and <version>; a fresh
# login shell finds kluris there; `kluris --version` is `kluris <version>`;
# `kluris doctor --json` is ok with this binary first on PATH; `kluris
# self-update --check --json` against the real GitHub release listing says
# no update is available (prereleases are skipped, v2 releases never win);
# `kluris self-uninstall --yes --json` removes exactly the installer's files
# and brings the sandbox's own rc files back byte for byte; the real HOME's
# rc files and kluris files are the same before and after.
#
# The JSON checks run on KLURIS_SMOKE_PYTHON (default python3). Exits 0 when
# every check passes, 1 at the first failure, 2 for a bad argument.
#
# The rc-file lines and the messages name $HOME and ~ as text, unexpanded.
# shellcheck disable=SC2016,SC2088
set -eu

pinned=1
if [ "${1:-}" = "--unpinned" ]; then
    pinned=0
    shift
fi
if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: published-smoke.sh [--unpinned] <version> [<site install.sh URL>]" >&2
    exit 2
fi
version=${1#v}
site=${2:-https://kluris.ngvoicu.dev/install.sh}
case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "published-smoke: <version> is a release number such as 3.0.0-rc.1, not '$1'" >&2; exit 2 ;;
esac
case "$version" in
    *[!0-9A-Za-z.-]*) echo "published-smoke: <version> is a release number such as 3.0.0-rc.1, not '$1'" >&2; exit 2 ;;
esac

fail() {
    echo "published-smoke: FAIL: $*" >&2
    exit 1
}
step() {
    echo "published-smoke: $*"
}

python="${KLURIS_SMOKE_PYTHON:-python3}"
"$python" -c "" 2>/dev/null || fail "$python (KLURIS_SMOKE_PYTHON) is needed for the JSON checks"

work=$(mktemp -d "${TMPDIR:-/tmp}/kluris-published-smoke.XXXXXX")
cleanup() {
    rm -rf "${work:?}"
}
trap cleanup EXIT INT TERM

sandbox="$work/home"
mkdir -p "$sandbox"
base_path=/usr/bin:/bin:/usr/sbin:/sbin

# The real HOME's files an installer or an uninstall would touch, as they are now.
real_home="${HOME:-}"
snapshot() {
    for f in .profile .bashrc .bash_profile .bash_login .zshrc .zshenv \
        .config/fish/conf.d/kluris.env.fish .config/kluris/kluris-receipt.json \
        .config/kluris/env.sh .config/kluris/env.fish .kluris/bin/kluris; do
        if [ -n "$real_home" ] && [ -f "$real_home/$f" ]; then
            printf '%s %s\n' "$f" "$(cksum < "$real_home/$f")"
        else
            printf '%s absent\n' "$f"
        fi
    done
}
snapshot > "$work/real-home.before"

# The sandbox user's own rc files: each must come back byte for byte.
printf 'export EDITOR=vi\n' > "$sandbox/.profile"
printf 'setopt no_beep\n' > "$sandbox/.zshrc"

# Every file under the sandbox HOME with its SHA-256.
inventory() {
    "$python" -I - "$sandbox" <<'PY'
import hashlib, os, sys
root = sys.argv[1]
for folder, dirs, files in os.walk(root):
    dirs.sort()
    for name in sorted(files):
        path = os.path.join(folder, name)
        if os.path.islink(path):
            digest = "link:" + os.readlink(path)
        else:
            with open(path, "rb") as handle:
                digest = hashlib.sha256(handle.read()).hexdigest()
        print(os.path.relpath(path, root), digest)
PY
}
inventory > "$work/before-install"

# A clean environment: the sandbox HOME, a system PATH, no ZDOTDIR,
# XDG_CONFIG_HOME, GITHUB_PATH, KLURIS_GITHUB_API_URL or token from here.
in_sandbox() {
    env -i HOME="$sandbox" PATH="$base_path" USER="${USER:-kluris}" LANG=C TERM=dumb \
        KLURIS_NO_PROMPT=1 NO_COLOR=1 "$@"
}
json_check() {
    file=$1
    shift
    "$python" -I - "$file" "$@" || { cat "$file"; fail "$(basename "$file") is not what it should be"; }
}

# The install: `curl -fsSL <site> | sh`, pinned to the release, or unpinned
# (GitHub's latest, which must be <version>).
if [ "$pinned" -eq 1 ]; then
    step "installing kluris $version through $site"
    in_sandbox KLURIS_VERSION="$version" sh -c 'curl -fsSL "$0" | sh' "$site" > "$work/install.out" 2>&1 \
        || { cat "$work/install.out"; fail "the site wrapper's install failed"; }
else
    step "installing GitHub's latest kluris (expecting $version) through $site, no KLURIS_VERSION"
    in_sandbox sh -c 'curl -fsSL "$0" | sh' "$site" > "$work/install.out" 2>&1 \
        || { cat "$work/install.out"; fail "the site wrapper's install failed"; }
fi
cat "$work/install.out"
grep -q "no checksums to verify" "$work/install.out" && fail "the installer verified no checksum"
grep -q "skipping sha256 checksum verification" "$work/install.out" && fail "the installer skipped the SHA-256 check"
grep -q "kluris installed. Open a new terminal, then run: kluris doctor" "$work/install.out" \
    || fail "the installer did not print the success message"

bin="$sandbox/.kluris/bin"
config="$sandbox/.config/kluris"
receipt="$config/kluris-receipt.json"
[ -x "$bin/kluris" ] || fail "no executable $bin/kluris"
[ -f "$receipt" ] || fail "no receipt at $receipt"
json_check "$receipt" "$bin" "$version" <<'PY'
import json, sys
receipt = json.load(open(sys.argv[1], encoding="utf-8"))
assert receipt["install_prefix"] == sys.argv[2], receipt["install_prefix"]
assert receipt["version"] == sys.argv[3], receipt["version"]
assert receipt["modify_path"] is True, receipt["modify_path"]
PY
step "installed: $bin/kluris; the receipt: $(cat "$receipt")"
inventory > "$work/after-install"

login_shell=""
for shell in zsh bash; do
    if command -v "$shell" >/dev/null 2>&1; then
        case "$shell" in
            zsh) login_shell="$(command -v zsh) -l -i -c" ;;
            bash) login_shell="$(command -v bash) -l -c" ;;
        esac
        break
    fi
done
[ -n "$login_shell" ] || fail "no zsh or bash for a login shell"
# A command in a fresh login shell of the sandbox.
fresh() {
    # shellcheck disable=SC2086
    in_sandbox $login_shell "$1"
}

found=$(fresh 'command -v kluris' 2>/dev/null | tail -n 1)
[ "$found" = "$bin/kluris" ] || fail "a fresh login shell ($login_shell) finds kluris at '$found'"
step "a fresh login shell finds $found"
got=$(fresh 'kluris --version' 2>&1 | tail -n 1)
[ "$got" = "kluris $version" ] || fail "kluris --version printed '$got'"
step "kluris --version: $got"

fresh 'kluris doctor --json' > "$work/doctor.json" 2> "$work/doctor.err" \
    || { cat "$work/doctor.json" "$work/doctor.err"; fail "kluris doctor --json failed"; }
json_check "$work/doctor.json" "$bin/kluris" <<'PY'
import json, sys
lines = [line for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
report = json.loads(lines[-1])
assert report["ok"] is True, report
checks = {check["name"]: check for check in report["checks"]}
path = checks["path"]
assert path["passed"] is True and not path.get("warning"), path
assert path["detail"] == sys.argv[2], path
print("published-smoke: kluris doctor --json: ok; " + ", ".join(
    f"{name} {'passed' if check['passed'] else 'FAILED'}" for name, check in checks.items()))
PY

fresh 'kluris self-update --check --json' > "$work/check.json" 2> "$work/check.err" \
    || { cat "$work/check.json" "$work/check.err"; fail "kluris self-update --check --json failed"; }
json_check "$work/check.json" "$version" <<'PY'
import json, sys
report = json.loads([l for l in open(sys.argv[1], encoding="utf-8") if l.strip()][-1])
assert report.get("ok") is True and report.get("current") == sys.argv[2], report
assert report.get("update_available") is False, report
PY
step "self-update --check --json: $(tail -n 1 "$work/check.json")"

inventory > "$work/before-uninstall"
fresh 'kluris self-uninstall --yes --json' > "$work/uninstall.json" 2>&1 \
    || { cat "$work/uninstall.json"; fail "kluris self-uninstall --yes --json failed"; }
step "self-uninstall --yes --json: $(tail -n 1 "$work/uninstall.json" | cut -c1-400)"
inventory > "$work/after-uninstall"
"$python" -I - "$work" <<'PY' || fail "self-uninstall did not leave exactly what it should"
import sys
work = sys.argv[1]
def load(name):
    entries = {}
    for line in open(f"{work}/{name}", encoding="utf-8"):
        path, digest = line.rstrip("\n").rsplit(" ", 1)
        entries[path] = digest
    return entries
start, installed, before, after = (load(n) for n in ("before-install", "after-install", "before-uninstall", "after-uninstall"))
ours = {".kluris/bin/kluris", ".config/kluris/env.sh", ".config/kluris/env.fish",
        ".config/kluris/kluris-receipt.json", ".config/fish/conf.d/kluris.env.fish"}
problems = []
if not ours <= set(installed) - set(start):
    problems.append(f"the installer did not write {sorted(ours - set(installed))}")
for path in sorted(ours):
    if path in after:
        problems.append(f"still there: {path}")
for path in sorted(start):
    if after.get(path) != start[path]:
        problems.append(f"not byte-identical to before the install: {path}")
for path in sorted(set(before) - ours):
    if path not in start and after.get(path) != before[path]:
        problems.append(f"removed or changed but not the installer's: {path}")
for path in sorted(set(after) - set(before)):
    problems.append(f"new after the uninstall: {path}")
for p in problems:
    print("published-smoke:", p, file=sys.stderr)
kept = sorted(set(after) - set(start))
print(f"published-smoke: the installer's {len(ours)} files are gone; {len(start)} rc files byte-identical "
      f"to before the install; kept (doctor's own): {', '.join(kept) or 'nothing'}")
sys.exit(1 if problems else 0)
PY

snapshot > "$work/real-home.after"
diff -u "$work/real-home.before" "$work/real-home.after" || fail "the real HOME's rc or kluris files changed"
step "the real HOME's rc and kluris files are unchanged"
if [ "$pinned" -eq 1 ]; then
    step "ok ($(uname -s) $(uname -m), kluris $version from GitHub through $site)"
else
    step "ok ($(uname -s) $(uname -m), kluris $version, GitHub's latest, from GitHub through $site, unpinned)"
fi
