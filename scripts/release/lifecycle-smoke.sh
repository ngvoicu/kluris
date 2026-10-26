#!/bin/sh
# The installed life cycle on macOS or Linux, with dist's real output: install
# an old release with its kluris-installer.sh, `kluris self-update` to a new
# one through a fake GitHub release API, then `kluris self-uninstall`, in a
# sandbox HOME (never the real one):
#
#   scripts/release/lifecycle-smoke.sh <releases> <old version> <new version>
#
# <releases> holds v<old>/ and v<new>/, each a release folder as
# scripts/release/build-versions.sh writes them (this machine's archive,
# kluris-installer.sh, sha256.sum), served by scripts/release/serve.py with
# its release API (`KLURIS_GITHUB_API_URL` points there; plain http is
# allowed to loopback).
#
# Checks, each from a fresh login shell of the sandbox:
# 1. the old release installs into ~/.kluris/bin; `kluris --version` names it;
# 2. a brain, a companion and the agent skills are made (`kluris create`,
#    `kluris companion add`), to be kept by the uninstall;
# 3. `kluris self-update --check --json` reports the new version available;
# 4. `kluris self-update --json` replaces the binary with the new release's
#    (its bytes are the archive's), rewrites only the receipt's `version`, and
#    carries the new binary's `doctor` report; `kluris --version` names it;
# 5. `kluris self-uninstall --yes --json` removes the binary, env.sh,
#    env.fish, the fish conf.d file, the receipt and the rc lines, and nothing
#    else: every other file is byte-identical to before it (config,
#    companions, skills, the brain), and the rc files the sandbox started with
#    are byte-identical to before the install.
#
# The local server and the JSON checks run on KLURIS_SMOKE_PYTHON (default
# python3). Exits 0 when every check passes, 1 at the first failure.
set -eu

if [ $# -ne 3 ]; then
    echo "usage: lifecycle-smoke.sh <releases> <old version> <new version>" >&2
    exit 2
fi
releases_dir=$(cd "$1" && pwd)
old=$2
new=$3
here=$(cd "$(dirname "$0")" && pwd)
python="${KLURIS_SMOKE_PYTHON:-python3}"

fail() {
    echo "lifecycle-smoke: FAIL: $*" >&2
    exit 1
}
step() {
    echo "lifecycle-smoke: $*"
}

for v in "$old" "$new"; do
    [ -f "$releases_dir/v$v/kluris-installer.sh" ] || fail "no v$v/kluris-installer.sh in $releases_dir"
done
"$python" -c "" 2>/dev/null || fail "$python (KLURIS_SMOKE_PYTHON) is needed for the local server and the JSON checks"

work=$(mktemp -d "${TMPDIR:-/tmp}/kluris-lifecycle-smoke.XXXXXX")
server_pid=""
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
    rm -rf "${work:?}"
}
trap cleanup EXIT INT TERM

sandbox="$work/home"
mkdir -p "$sandbox"
base_path=/usr/bin:/bin:/usr/sbin:/sbin

# The rc files a user has: each must come back byte for byte.
printf 'export EDITOR=vi\n' > "$sandbox/.profile"
printf "alias ll='ls -l'\n# keep me\n" > "$sandbox/.bashrc"
printf 'setopt no_beep\n' > "$sandbox/.zshrc"

# A list of every file under the sandbox HOME with its SHA-256.
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

mkdir -p "$work/served"
cp -R "$releases_dir/v$old" "$releases_dir/v$new" "$work/served/"
(cd "$work" && exec "$python" -I "$here/serve.py" "$work/served" "$work/port" "$work/requests.log") &
server_pid=$!
tries=0
while [ ! -s "$work/port" ]; do
    tries=$((tries + 1))
    [ "$tries" -le 100 ] || fail "the local server did not start"
    kill -0 "$server_pid" 2>/dev/null || fail "the local server exited"
    sleep 0.1
done
port=$(cat "$work/port")
api="http://127.0.0.1:$port"
step "releases v$old and v$new and their release API served at $api"

# The sandbox: its HOME, a system PATH, the fake API, no variable of this
# process (ZDOTDIR, XDG_CONFIG_HOME, GITHUB_PATH, KLURIS_GITHUB_TOKEN).
in_sandbox() {
    env -i HOME="$sandbox" PATH="$base_path" USER="${USER:-kluris}" LANG=C TERM=dumb \
        KLURIS_GITHUB_API_URL="$api" KLURIS_NO_PROMPT=1 NO_COLOR=1 "$@"
}
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
json_check() {
    file=$1
    shift
    "$python" -I - "$file" "$@" || { cat "$file"; fail "$(basename "$file") is not what it should be"; }
}

# 1. The old release, with its own installer.
in_sandbox KLURIS_DOWNLOAD_URL="$api/ngvoicu/kluris/releases/download/v$old" \
    sh "$releases_dir/v$old/kluris-installer.sh" > "$work/install.out" 2>&1 \
    || { cat "$work/install.out"; fail "kluris-installer.sh of v$old failed"; }
grep -q "skipping sha256\|no checksums to verify" "$work/install.out" && fail "the installer verified no checksum"
bin="$sandbox/.kluris/bin/kluris"
config="$sandbox/.config/kluris"
receipt="$config/kluris-receipt.json"
[ -x "$bin" ] || fail "no $bin"
got=$(fresh 'kluris --version' 2>&1 | tail -n 1)
[ "$got" = "kluris $old" ] || fail "after the install kluris --version printed '$got'"
step "installed v$old: $got"
step "the receipt as dist writes it: $(cat "$receipt")"
inventory > "$work/after-install"

# 2. What the uninstall must keep.
mkdir -p "$sandbox/brains"
# shellcheck disable=SC2016 # $HOME is the sandbox shell's
fresh 'kluris create lc-brain --type personal --path "$HOME/brains" --description "Life-cycle smoke" --json' \
    > "$work/create.json" 2>&1 || { cat "$work/create.json"; fail "kluris create failed"; }
fresh 'kluris companion add specmint-core --brain lc-brain --json' > "$work/companion.json" 2>&1 \
    || { cat "$work/companion.json"; fail "kluris companion add failed"; }
[ -f "$sandbox/.kluris/config.yml" ] && [ -f "$sandbox/.agents/skills/kluris-lc-brain/SKILL.md" ] \
    && [ -f "$sandbox/.kluris/companions/specmint-core/SKILL.md" ] && [ -f "$sandbox/brains/lc-brain/brain.md" ] \
    || fail "no config, skill, companion or brain after kluris create and companion add"
step "made a brain, a companion and the agent skills"

# 3. The check.
fresh 'kluris self-update --check --json' > "$work/check.json" 2>&1 || { cat "$work/check.json"; fail "self-update --check failed"; }
json_check "$work/check.json" "$old" "$new" <<'PY'
import json, sys
report = json.loads([l for l in open(sys.argv[1], encoding="utf-8") if l.strip()][-1])
want = {"ok": True, "current": sys.argv[2], "latest": sys.argv[3], "update_available": True}
assert report == want, report
PY
step "self-update --check --json: $(tail -n 1 "$work/check.json")"

# 4. The update.
cp "$receipt" "$work/receipt-before-update"
fresh 'kluris self-update --json' > "$work/update.json" 2> "$work/update.err" \
    || { cat "$work/update.json" "$work/update.err"; fail "self-update failed"; }
json_check "$work/update.json" <<'PY'
import json, sys
report = json.loads([l for l in open(sys.argv[1], encoding="utf-8") if l.strip()][-1])
assert report.get("ok") is True, report
assert "doctor" in json.dumps(report) or "checks" in json.dumps(report), report
PY
step "self-update --json: $(tail -n 1 "$work/update.json" | cut -c1-200)"
"$python" -I - "$releases_dir/v$new" "$bin" <<'PY' || fail "the installed binary is not v$new's"
import glob, sys, tarfile
archive = [p for p in glob.glob(sys.argv[1] + "/kluris-*.tar.gz")]
assert len(archive) == 1, archive
with tarfile.open(archive[0]) as tar:
    member = [m for m in tar.getmembers() if m.name.endswith("/kluris")]
    assert len(member) == 1, [m.name for m in tar.getmembers()]
    want = tar.extractfile(member[0]).read()
assert open(sys.argv[2], "rb").read() == want, "bytes differ"
PY
expected=$(sed "s/\"version\":\"$old\"/\"version\":\"$new\"/" "$work/receipt-before-update")
[ "$(cat "$receipt")" = "$expected" ] || { cat "$receipt"; fail "the receipt changed other than its version"; }
got=$(fresh 'kluris --version' 2>&1 | tail -n 1)
[ "$got" = "kluris $new" ] || fail "after self-update kluris --version printed '$got'"
step "updated: $got; the binary is v$new's archive's; the receipt's version alone changed"

# 5. The uninstall.
inventory > "$work/before-uninstall"
fresh 'kluris self-uninstall --yes --json' > "$work/uninstall.json" 2>&1 \
    || { cat "$work/uninstall.json"; fail "self-uninstall failed"; }
step "self-uninstall --yes --json: $(tail -n 1 "$work/uninstall.json" | cut -c1-300)"
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
installer = {p for p in installed if p not in start}
ours = {".kluris/bin/kluris", ".config/kluris/env.sh", ".config/kluris/env.fish",
        ".config/kluris/kluris-receipt.json", ".config/fish/conf.d/kluris.env.fish"}
problems = []
if not ours <= installer:
    problems.append(f"the installer did not write {sorted(ours - installer)}")
for path in sorted(set(before) - ours):
    if path in start:
        continue
    if path not in after:
        problems.append(f"removed but not the installer's: {path}")
    elif after[path] != before[path]:
        problems.append(f"changed: {path}")
for path in sorted(ours):
    if path in after:
        problems.append(f"still there: {path}")
for path in sorted(start):
    if after.get(path) != start[path]:
        problems.append(f"not byte-identical to before the install: {path}")
for path in sorted(set(after) - set(before)):
    problems.append(f"new after the uninstall: {path}")
for p in problems:
    print("lifecycle-smoke:", p, file=sys.stderr)
kept = sorted(set(after) - set(start))
print(f"lifecycle-smoke: kept {len(kept)} files the uninstall must keep (config, companion, skills, brain); "
      f"{len(start)} rc files byte-identical to before the install")
sys.exit(1 if problems else 0)
PY
for d in .config/fish/conf.d .config/kluris .kluris/bin; do
    if [ -d "$sandbox/$d" ]; then step "left folder: ~/$d ($(find "$sandbox/$d" -mindepth 1 | wc -l | tr -d ' ') entries)"; fi
done
step "ok ($(uname -s) $(uname -m), v$old -> v$new -> uninstalled)"
