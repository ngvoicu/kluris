#!/bin/sh
# Smoke-test the release installer on macOS or Linux, in a sandbox HOME
# (never the real one), against a local release folder served the way GitHub
# Releases serves it (scripts/release/serve.py):
#
#   scripts/release/installer-smoke.sh [--wrapper <site install.sh>] <release folder>
#
# The release folder is what scripts/release/build-local.sh writes
# (target/distrib): this machine's archive, kluris-installer.sh, sha256.sum.
#
# Checks: the installer verifies the archive's SHA-256 and puts `kluris` in
# the default ~/.kluris/bin (KLURIS_INSTALL_DIR unset); its env scripts
# (dist 0.33.1 keeps them beside the receipt, ~/.config/kluris/env.sh and
# env.fish, out of the bin folder), the `. "$HOME/.config/kluris/env.sh"`
# line in ~/.profile and ~/.zshrc, fish's conf.d file and the receipt land in
# the sandbox HOME and nowhere else (the real HOME's rc files are compared
# before and after); a fresh login shell
# (sh, bash, zsh, fish: each one installed) finds `kluris` there; `kluris
# --version` names the release and `kluris doctor --json` is ok with this
# binary first on PATH.
#
# With --wrapper, the site's install.sh is the entry point instead, served
# beside the release and run the way the site says, `curl -fsSL <url> | sh`:
# with KLURIS_VERSION=<v> (and v<v>) it must fetch
# releases/download/v<v>/kluris-installer.sh, without it
# releases/latest/download/kluris-installer.sh, and it must refuse a
# malformed KLURIS_VERSION before fetching anything. Without `sha256sum` on
# PATH (macOS 14 and older; dist's installer then skips its archive check)
# the archive must still be verified through `shasum -a 256`, a corrupted
# archive refused, and with neither tool the wrapper must stop before it
# downloads anything.
#
# The local server and the JSON checks run on KLURIS_SMOKE_PYTHON (default
# python3). Exits 0 when every check passes, 1 at the first failure.
#
# The rc-file lines and the messages name $HOME and ~ as text, unexpanded.
# shellcheck disable=SC2016,SC2088
set -eu

wrapper=""
if [ "${1:-}" = "--wrapper" ]; then
    wrapper=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
    shift 2
fi
if [ $# -ne 1 ]; then
    echo "usage: installer-smoke.sh [--wrapper <install.sh>] <release folder>" >&2
    exit 2
fi
release=$(cd "$1" && pwd)
here=$(cd "$(dirname "$0")" && pwd)

fail() {
    echo "installer-smoke: FAIL: $*" >&2
    exit 1
}
step() {
    echo "installer-smoke: $*"
}

[ -f "$release/kluris-installer.sh" ] || fail "no kluris-installer.sh in $release"
[ -f "$release/sha256.sum" ] || fail "no sha256.sum in $release"
version=$(sed -n 's/^APP_VERSION="\(.*\)"$/\1/p' "$release/kluris-installer.sh" | head -n 1)
[ -n "$version" ] || fail "kluris-installer.sh names no APP_VERSION"
python="${KLURIS_SMOKE_PYTHON:-python3}"
"$python" -c "" 2>/dev/null || fail "$python (KLURIS_SMOKE_PYTHON) is needed for the local server and the JSON checks"

work=$(mktemp -d "${TMPDIR:-/tmp}/kluris-installer-smoke.XXXXXX")
server_pid=""
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
    rm -rf "${work:?}"
}
trap cleanup EXIT INT TERM

sandbox="$work/home"
mkdir -p "$sandbox"
base_path=/usr/bin:/bin:/usr/sbin:/sbin

# The real HOME's files an installer would edit, as they are now.
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

# The local "GitHub": a copy of the release (and the wrapper), served from
# the work folder.
mkdir -p "$work/served"
for f in "$release"/*; do
    [ -f "$f" ] && cp "$f" "$work/served/"
done
[ -z "$wrapper" ] || cp "$wrapper" "$work/served/install.sh"
(cd "$work" && exec "$python" -I "$here/serve.py" "$work/served" "$work/port" "$work/requests.log") &
server_pid=$!
tries=0
while [ ! -s "$work/port" ]; do
    tries=$((tries + 1))
    [ "$tries" -le 600 ] || fail "the local server did not start in 60 s"
    kill -0 "$server_pid" 2>/dev/null || fail "the local server exited"
    sleep 0.1
done
port=$(cat "$work/port")
releases="http://127.0.0.1:$port/ngvoicu/kluris/releases"
download="$releases/download/v$version"
site="http://127.0.0.1:$port/install.sh"
step "release $version served at $releases"

# A clean environment: the sandbox HOME, a system PATH, no ZDOTDIR,
# XDG_CONFIG_HOME or GITHUB_PATH from this process.
in_sandbox() {
    env -i HOME="$sandbox" PATH="$base_path" USER="${USER:-kluris}" LANG=C TERM=dumb "$@"
}

# `curl -fsSL <site>/install.sh | sh`, in the sandbox, with these variables.
piped() {
    in_sandbox "$@" sh -c 'curl -fsSL "$0" | sh' "$site"
}
# The requests after the wrapper's own download.
after_wrapper() {
    grep -v '^/install.sh$' "$work/requests.log" || true
}

if [ -n "$wrapper" ]; then
    : > "$work/requests.log"
    if piped KLURIS_RELEASES_URL="$releases" KLURIS_VERSION='3.0.0/../../evil' > "$work/bad.out" 2>&1; then
        fail "the wrapper ran with a malformed KLURIS_VERSION"
    fi
    [ -z "$(after_wrapper)" ] || fail "the wrapper fetched something for a malformed KLURIS_VERSION: $(after_wrapper)"
    [ ! -e "$sandbox/.kluris" ] || fail "the wrapper wrote ~/.kluris for a malformed KLURIS_VERSION"
    step "wrapper refuses a malformed KLURIS_VERSION: $(head -n 1 "$work/bad.out")"

    for pin in "$version" "v$version"; do
        : > "$work/requests.log"
        piped KLURIS_RELEASES_URL="$releases" KLURIS_VERSION="$pin" KLURIS_DOWNLOAD_URL="$download" \
            KLURIS_NO_MODIFY_PATH=1 KLURIS_INSTALL_DIR="$work/pinned" > "$work/pinned.out" 2>&1 \
            || { cat "$work/pinned.out"; fail "the wrapper failed with KLURIS_VERSION=$pin"; }
        [ -x "$work/pinned/kluris" ] || fail "KLURIS_VERSION=$pin installed nothing into KLURIS_INSTALL_DIR"
        first=$(after_wrapper | head -n 1)
        [ "$first" = "/ngvoicu/kluris/releases/download/v$version/kluris-installer.sh" ] \
            || fail "KLURIS_VERSION=$pin fetched $first"
        step "wrapper with KLURIS_VERSION=$pin fetched $first"
    done
    rm -rf "${work:?}/pinned" "${sandbox:?}/.config"

    # A PATH like the sandbox's without the commands named: each system
    # folder that has one is replaced by links to everything else in it.
    path_without() {
        new=""
        n=0
        for dir in $(echo "$base_path" | tr ':' ' '); do
            n=$((n + 1))
            hit=""
            for cmd in "$@"; do [ -e "$dir/$cmd" ] && hit=1; done
            if [ -n "$hit" ]; then
                farm="$work/path-$n-$(echo "$*" | tr ' ' '-')"
                rm -rf "${farm:?}"
                mkdir -p "$farm"
                for entry in "$dir"/*; do
                    name=$(basename "$entry")
                    skip=""
                    for cmd in "$@"; do [ "$name" = "$cmd" ] && skip=1; done
                    [ -n "$skip" ] || ln -s "$entry" "$farm/$name"
                done
                dir=$farm
            fi
            new="${new:+$new:}$dir"
        done
        echo "$new"
    }
    nosum_path=$(path_without sha256sum)
    # This Mac's sha256sum is /sbin's: the reviewer's way is /sbin off PATH.
    if [ "$(uname -s)" = Darwin ] && [ -e /sbin/sha256sum ] && [ ! -e /usr/bin/sha256sum ]; then
        nosum_path=/usr/bin:/bin:/usr/sbin
    fi
    env -i PATH="$nosum_path" sh -c 'command -v sha256sum' >/dev/null 2>&1 && fail "sha256sum is still on $nosum_path"
    env -i PATH="$nosum_path" sh -c 'command -v shasum' >/dev/null 2>&1 || fail "no shasum on $nosum_path"

    : > "$work/requests.log"
    in_sandbox PATH="$nosum_path" KLURIS_RELEASES_URL="$releases" KLURIS_VERSION="$version" KLURIS_DOWNLOAD_URL="$download" \
        KLURIS_NO_MODIFY_PATH=1 KLURIS_INSTALL_DIR="$work/nosum" sh -c 'curl -fsSL "$0" | sh' "$site" > "$work/nosum.out" 2>&1 \
        || { cat "$work/nosum.out"; fail "the wrapper failed without sha256sum on PATH"; }
    grep -q "skipping sha256\|no checksums to verify" "$work/nosum.out" \
        && { cat "$work/nosum.out"; fail "without sha256sum on PATH the archive was not verified"; }
    [ -x "$work/nosum/kluris" ] || fail "nothing installed without sha256sum on PATH"
    step "wrapper without sha256sum on PATH ($nosum_path): the archive verified through shasum"

    # The same with the archive corrupted: refused, nothing installed.
    mkdir -p "$work/corrupt"
    for f in "$work/served"/*; do [ -f "$f" ] && cp "$f" "$work/corrupt/"; done
    for f in "$work/corrupt"/kluris-*.tar.gz; do printf 'corrupt' >> "$f"; done
    (cd "$work" && exec "$python" -I "$here/serve.py" "$work/corrupt" "$work/port2" "$work/requests2.log") &
    corrupt_pid=$!
    tries=0
    while [ ! -s "$work/port2" ]; do
        tries=$((tries + 1))
        [ "$tries" -le 600 ] || fail "the second local server did not start in 60 s"
        sleep 0.1
    done
    bad="http://127.0.0.1:$(cat "$work/port2")"
    if in_sandbox PATH="$nosum_path" KLURIS_RELEASES_URL="$bad/ngvoicu/kluris/releases" KLURIS_VERSION="$version" \
        KLURIS_DOWNLOAD_URL="$bad/ngvoicu/kluris/releases/download/v$version" KLURIS_NO_MODIFY_PATH=1 \
        KLURIS_INSTALL_DIR="$work/corrupted" sh -c 'curl -fsSL "$0" | sh' "$site" > "$work/corrupt.out" 2>&1; then
        kill "$corrupt_pid" 2>/dev/null || true
        cat "$work/corrupt.out"
        fail "a corrupted archive installed without sha256sum on PATH"
    fi
    kill "$corrupt_pid" 2>/dev/null || true
    grep -q "checksum mismatch" "$work/corrupt.out" || { cat "$work/corrupt.out"; fail "the corrupted archive was not refused by its checksum"; }
    [ ! -e "$work/corrupted/kluris" ] || fail "a corrupted archive left a kluris"
    step "wrapper without sha256sum on PATH refuses a corrupted archive: $(grep -m 1 'checksum mismatch' "$work/corrupt.out")"

    # Neither sha256sum nor shasum: a clear refusal, nothing downloaded.
    nothing_path=$(path_without sha256sum shasum)
    : > "$work/requests.log"
    if in_sandbox PATH="$nothing_path" KLURIS_RELEASES_URL="$releases" KLURIS_VERSION="$version" \
        KLURIS_DOWNLOAD_URL="$download" KLURIS_NO_MODIFY_PATH=1 KLURIS_INSTALL_DIR="$work/nothing" \
        sh -c 'curl -fsSL "$0" | sh' "$site" > "$work/nothing.out" 2>&1; then
        fail "the wrapper installed with neither sha256sum nor shasum"
    fi
    [ -z "$(after_wrapper)" ] || fail "with neither sha256sum nor shasum the wrapper fetched $(after_wrapper)"
    [ ! -e "$work/nothing" ] || fail "with neither sha256sum nor shasum something was installed"
    step "wrapper with neither sha256sum nor shasum: $(head -n 1 "$work/nothing.out")"
    rm -rf "${sandbox:?}/.config"
    : > "$work/requests.log"
fi

# The install itself, into the default ~/.kluris/bin.
if [ -n "$wrapper" ]; then
    piped KLURIS_RELEASES_URL="$releases" KLURIS_DOWNLOAD_URL="$download" > "$work/install.out" 2>&1 \
        || { cat "$work/install.out"; fail "the wrapper's install failed"; }
    first=$(after_wrapper | head -n 1)
    [ "$first" = "/ngvoicu/kluris/releases/latest/download/kluris-installer.sh" ] \
        || fail "the wrapper without KLURIS_VERSION fetched $first"
    step "wrapper without KLURIS_VERSION fetched $first"
else
    in_sandbox KLURIS_DOWNLOAD_URL="$download" sh "$release/kluris-installer.sh" > "$work/install.out" 2>&1 \
        || { cat "$work/install.out"; fail "kluris-installer.sh failed"; }
fi
cat "$work/install.out"
grep -q "no checksums to verify" "$work/install.out" && fail "the installer verified no checksum"
grep -q "skipping sha256 checksum verification" "$work/install.out" && fail "the installer skipped the SHA-256 check"
grep -q "kluris installed. Open a new terminal, then run: kluris doctor" "$work/install.out" \
    || fail "the installer did not print the success message"

bin="$sandbox/.kluris/bin"
config="$sandbox/.config/kluris"
[ -x "$bin/kluris" ] || fail "no executable $bin/kluris"
[ -f "$config/env.sh" ] && [ -f "$config/env.fish" ] || fail "no env scripts in $config"
grep -qF '"$HOME/.kluris/bin"' "$config/env.sh" || fail "env.sh does not put \$HOME/.kluris/bin on PATH"
line='. "$HOME/.config/kluris/env.sh"'
for rc in .profile .zshrc; do
    grep -qxF "$line" "$sandbox/$rc" || fail "~/$rc has no line $line"
done
grep -qxF 'source "$HOME/.config/kluris/env.fish"' "$sandbox/.config/fish/conf.d/kluris.env.fish" \
    || fail "fish's conf.d/kluris.env.fish does not source ~/.config/kluris/env.fish"
receipt="$config/kluris-receipt.json"
[ -f "$receipt" ] || fail "no receipt at $receipt"
"$python" -I - "$receipt" "$bin" "$version" <<'PY' || fail "the receipt does not record this install"
import json, sys
receipt = json.load(open(sys.argv[1], encoding="utf-8"))
assert receipt["install_prefix"] == sys.argv[2], receipt["install_prefix"]
assert receipt["version"] == sys.argv[3], receipt["version"]
assert receipt["modify_path"] is True, receipt["modify_path"]
PY
step "installed: $bin/kluris, env scripts, ~/.profile and ~/.zshrc lines, fish conf.d, receipt"

# What the installer wrote, and nothing else.
(cd "$sandbox" && find . -type f | sort) > "$work/written"
cat > "$work/expected" <<EOF
./.config/fish/conf.d/kluris.env.fish
./.config/kluris/env.fish
./.config/kluris/env.sh
./.config/kluris/kluris-receipt.json
./.kluris/bin/kluris
./.profile
./.zshrc
EOF
diff -u "$work/expected" "$work/written" || fail "the installer wrote other files into HOME"
snapshot > "$work/real-home.after"
diff -u "$work/real-home.before" "$work/real-home.after" || fail "the real HOME's rc files changed"
step "the sandbox HOME holds exactly the installer's files; the real HOME is unchanged"

# A new terminal: each login shell this machine has finds kluris.
login_shell=""
for shell in sh bash zsh fish; do
    command -v "$shell" >/dev/null 2>&1 || { step "$shell: not installed, skipped"; continue; }
    case "$shell" in
        zsh) flags="-l -i -c" ;;
        *) flags="-l -c" ;;
    esac
    # shellcheck disable=SC2086
    found=$(in_sandbox "$(command -v "$shell")" $flags 'command -v kluris' 2>/dev/null | tail -n 1)
    [ "$found" = "$bin/kluris" ] || fail "a fresh $shell login shell finds kluris at '$found'"
    step "a fresh $shell login shell finds $found"
    case "$shell" in
        zsh | bash) [ -n "$login_shell" ] || login_shell="$(command -v "$shell") $flags" ;;
    esac
done
[ -n "$login_shell" ] || login_shell="$(command -v sh) -l -c"

# shellcheck disable=SC2086
got=$(in_sandbox $login_shell 'kluris --version' 2>&1 | tail -n 1)
[ "$got" = "kluris $version" ] || fail "kluris --version printed '$got'"
step "kluris --version: $got"
# shellcheck disable=SC2086
in_sandbox $login_shell 'kluris doctor --json' > "$work/doctor.json" 2> "$work/doctor.err" \
    || { cat "$work/doctor.json" "$work/doctor.err"; fail "kluris doctor --json failed"; }
"$python" -I - "$work/doctor.json" "$bin/kluris" <<'PY' || { cat "$work/doctor.json"; fail "kluris doctor --json is not ok with this binary first on PATH"; }
import json, sys
lines = [line for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
report = json.loads(lines[-1])
assert report["ok"] is True, report
checks = {check["name"]: check for check in report["checks"]}
path = checks["path"]
assert path["passed"] is True and not path.get("warning"), path
assert path["detail"] == sys.argv[2], path
print("installer-smoke: kluris doctor --json: ok; " + ", ".join(
    f"{name} {'passed' if check['passed'] else 'FAILED'}" for name, check in checks.items()))
PY

step "ok ($(uname -s) $(uname -m), kluris $version${wrapper:+, through the site wrapper})"
