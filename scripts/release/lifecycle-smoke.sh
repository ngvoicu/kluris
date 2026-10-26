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
# 6. KLURIS_NO_MODIFY_PATH=1 over the user's own ~/.config/kluris/env.sh and
#    the profile line sourcing it (T-111 finding 4): the receipt says
#    modify_path false, and self-uninstall removes the binary and the
#    receipt and leaves both of the user's files byte-identical.
# 7. An rc file the uninstall cannot rewrite (~/.profile a link into a
#    read-only dotfiles folder, T-111 finding 5): self-uninstall fails and
#    keeps the binary, the receipt and the env scripts; once the folder is
#    writable again a rerun finishes, the profile byte-identical to before.
# 8. `self-uninstall --purge` with a read-only folder in
#    ~/.kluris/companions (T-114): it fails and keeps the binary and the
#    receipt; once the folder is writable a rerun purges and finishes.
# 9. A read-only ~/.config/fish/conf.d (T-114): the uninstall fails, keeping
#    conf.d/kluris.env.fish and the env.fish it sources, the binary and the
#    receipt; a rerun finishes.
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

# 6. No PATH setup by the installer: the user's own is kept.
sandbox="$work/home-no-modify-path"
mkdir -p "$sandbox/.config/kluris"
printf 'export USER_SENTINEL=1\n' > "$sandbox/.config/kluris/env.sh"
# shellcheck disable=SC2016 # the line names $HOME as text, as a profile does
printf 'export EDITOR=vi\n. "$HOME/.config/kluris/env.sh"\n' > "$sandbox/.profile"
cp "$sandbox/.config/kluris/env.sh" "$work/user-env.sh"
cp "$sandbox/.profile" "$work/user-profile"
in_sandbox KLURIS_NO_MODIFY_PATH=1 KLURIS_DOWNLOAD_URL="$api/ngvoicu/kluris/releases/download/v$old" \
    sh "$releases_dir/v$old/kluris-installer.sh" > "$work/nomodify-install.out" 2>&1 \
    || { cat "$work/nomodify-install.out"; fail "KLURIS_NO_MODIFY_PATH=1 install failed"; }
grep -q '"modify_path":false' "$sandbox/.config/kluris/kluris-receipt.json" \
    || fail "the KLURIS_NO_MODIFY_PATH=1 receipt does not say modify_path false"
cmp -s "$sandbox/.config/kluris/env.sh" "$work/user-env.sh" || fail "the installer changed the user's env.sh"
cmp -s "$sandbox/.profile" "$work/user-profile" || fail "the installer changed the user's .profile"
in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --yes --json > "$work/nomodify-uninstall.json" 2>&1 \
    || { cat "$work/nomodify-uninstall.json"; fail "self-uninstall after a KLURIS_NO_MODIFY_PATH=1 install failed"; }
cmp -s "$sandbox/.config/kluris/env.sh" "$work/user-env.sh" || fail "self-uninstall changed the user's own env.sh"
cmp -s "$sandbox/.profile" "$work/user-profile" || fail "self-uninstall changed the user's own .profile line"
[ ! -e "$sandbox/.kluris/bin/kluris" ] && [ ! -e "$sandbox/.config/kluris/kluris-receipt.json" ] \
    || fail "self-uninstall left the binary or the receipt of a KLURIS_NO_MODIFY_PATH=1 install"
step "KLURIS_NO_MODIFY_PATH=1: modify_path false; the uninstall removed the binary and the receipt, the user's env.sh and .profile byte-identical"

# 7. An rc file the uninstall cannot rewrite, then a rerun.
sandbox="$work/home-failed-rc"
mkdir -p "$sandbox"
printf 'export EDITOR=vi\n' > "$sandbox/.profile"
cp "$sandbox/.profile" "$work/failed-rc-profile"
in_sandbox KLURIS_DOWNLOAD_URL="$api/ngvoicu/kluris/releases/download/v$old" \
    sh "$releases_dir/v$old/kluris-installer.sh" > "$work/failed-rc-install.out" 2>&1 \
    || { cat "$work/failed-rc-install.out"; fail "the install for the failed-rc case failed"; }
mkdir -p "$sandbox/dotfiles"
mv "$sandbox/.profile" "$sandbox/dotfiles/profile"
ln -s "$sandbox/dotfiles/profile" "$sandbox/.profile"
chmod 500 "$sandbox/dotfiles"
if in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --yes --json > "$work/failed-rc-1.json" 2>&1; then
    chmod 700 "$sandbox/dotfiles"
    cat "$work/failed-rc-1.json"
    fail "self-uninstall succeeded though ~/.profile could not be rewritten"
fi
chmod 700 "$sandbox/dotfiles"
for kept in .kluris/bin/kluris .config/kluris/kluris-receipt.json .config/kluris/env.sh .config/kluris/env.fish; do
    [ -e "$sandbox/$kept" ] || { cat "$work/failed-rc-1.json"; fail "a failed rc edit removed ~/$kept, which a retry needs"; }
done
step "an rc file that cannot be rewritten: self-uninstall exit 1, the binary, receipt and env scripts kept: $(tail -n 1 "$work/failed-rc-1.json" | sed 's/.*"failed": //' | cut -c1-160)"
in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --yes --json > "$work/failed-rc-2.json" 2>&1 \
    || { cat "$work/failed-rc-2.json"; fail "the rerun of self-uninstall failed"; }
for gone in .kluris/bin/kluris .config/kluris/kluris-receipt.json .config/kluris/env.sh .config/kluris/env.fish .config/fish/conf.d/kluris.env.fish; do
    [ ! -e "$sandbox/$gone" ] || fail "the rerun left ~/$gone"
done
cmp -s "$sandbox/dotfiles/profile" "$work/failed-rc-profile" || fail "the profile is not byte-identical after the rerun"
[ -L "$sandbox/.profile" ] || fail "the rerun replaced the ~/.profile link"
step "the rerun finished: the installer's files gone, the profile byte-identical and still a link"


# A fresh install of the old release into $sandbox.
fresh_install() {
    mkdir -p "$sandbox"
    in_sandbox KLURIS_DOWNLOAD_URL="$api/ngvoicu/kluris/releases/download/v$old" \
        sh "$releases_dir/v$old/kluris-installer.sh" > "$work/$1-install.out" 2>&1 \
        || { cat "$work/$1-install.out"; fail "the install for the $1 case failed"; }
}
# A failed uninstall kept what a retry needs.
assert_retry_possible() {
    for kept in .kluris/bin/kluris .config/kluris/kluris-receipt.json "$@"; do
        [ -e "$sandbox/$kept" ] || { cat "$work/$case-1.json"; fail "$case: a failed uninstall removed ~/$kept, which a retry needs"; }
    done
}

# 8. A purge that cannot clear ~/.kluris, then a rerun.
case=failed-purge
sandbox="$work/home-$case"
fresh_install "$case"
mkdir -p "$sandbox/.kluris/companions/protected"
printf 'remaining data\n' > "$sandbox/.kluris/companions/protected/notes.txt"
chmod 500 "$sandbox/.kluris/companions/protected"
if in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --purge --yes --json > "$work/$case-1.json" 2>&1; then
    chmod 700 "$sandbox/.kluris/companions/protected"
    cat "$work/$case-1.json"
    fail "a purge succeeded though ~/.kluris could not be cleared"
fi
chmod 700 "$sandbox/.kluris/companions/protected"
assert_retry_possible
step "a purge that cannot clear ~/.kluris: exit 1, the binary and receipt kept: $(tail -n 1 "$work/$case-1.json" | sed 's/.*"failed": //' | cut -c1-140)"
in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --purge --yes --json > "$work/$case-2.json" 2>&1 \
    || { cat "$work/$case-2.json"; fail "the rerun of the purge failed"; }
for gone in .kluris .config/kluris/kluris-receipt.json .config/kluris/env.sh .config/fish/conf.d/kluris.env.fish; do
    [ ! -e "$sandbox/$gone" ] || fail "the purge rerun left ~/$gone"
done
step "the purge rerun finished: ~/.kluris and the installer's files gone"

# 9. A fish conf.d that cannot be cleaned, then a rerun.
case=failed-fish
sandbox="$work/home-$case"
fresh_install "$case"
chmod 500 "$sandbox/.config/fish/conf.d"
if in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --yes --json > "$work/$case-1.json" 2>&1; then
    chmod 700 "$sandbox/.config/fish/conf.d"
    cat "$work/$case-1.json"
    fail "self-uninstall succeeded though fish's conf.d could not be cleaned"
fi
chmod 700 "$sandbox/.config/fish/conf.d"
assert_retry_possible .config/fish/conf.d/kluris.env.fish .config/kluris/env.fish
step "a fish conf.d that cannot be cleaned: exit 1, conf.d/kluris.env.fish, the env.fish it sources, the binary and receipt kept"
in_sandbox "$sandbox/.kluris/bin/kluris" self-uninstall --yes --json > "$work/$case-2.json" 2>&1 \
    || { cat "$work/$case-2.json"; fail "the rerun after the fish failure failed"; }
for gone in .kluris/bin/kluris .config/kluris/kluris-receipt.json .config/kluris/env.sh .config/kluris/env.fish .config/fish/conf.d/kluris.env.fish; do
    [ ! -e "$sandbox/$gone" ] || fail "the rerun after the fish failure left ~/$gone"
done
step "the rerun after the fish failure finished"

step "ok ($(uname -s) $(uname -m), v$old -> v$new -> uninstalled; no-modify-path, failed-rc, failed-purge and failed-fish)"
