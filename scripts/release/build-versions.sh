#!/bin/sh
# Build this tree as several releases that differ only in their version, for
# the life-cycle smoke (scripts/release/lifecycle-smoke.sh): each version a
# copy of the tree with `[workspace.package] version` changed, built by
# build-local.sh, its target/distrib copied to <out>/v<version>/.
#
#   scripts/release/build-versions.sh <out> <version>...
#
# The copies build into this tree's target folder (CARGO_TARGET_DIR, else
# target/) and use its tools (KLURIS_TOOLS_DIR, else target/tools), so the
# dependencies and dist are not built or fetched again.
set -eu

if [ $# -lt 2 ]; then
    echo "usage: build-versions.sh <out> <version>..." >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
shift
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export KLURIS_TOOLS_DIR="${KLURIS_TOOLS_DIR:-$root/target/tools}"
work="$root/target/release-versions"
rm -rf "${work:?}"
mkdir -p "$work"

for version in "$@"; do
    case "$version" in
        *[!0-9A-Za-z.+-]* | "") echo "build-versions: not a version: '$version'" >&2; exit 2 ;;
    esac
    copy="$work/kluris-$version"
    mkdir -p "$copy"
    # No pipe: the same two steps work with Windows' tar.exe.
    (cd "$root" && tar -cf "$work/tree.tar" --exclude=./target --exclude=./tmp --exclude=./.git \
        --exclude=./parity/.oracle --exclude=./parity/.venv .)
    tar -xf "$work/tree.tar" -C "$copy"
    rm -f "$work/tree.tar"
    sed "s/^version = \".*\"$/version = \"$version\"/" "$root/Cargo.toml" > "$copy/Cargo.toml"
    grep -q "^version = \"$version\"$" "$copy/Cargo.toml" || {
        echo "build-versions: no [workspace.package] version line in Cargo.toml" >&2
        exit 1
    }
    (cd "$copy" && cargo metadata --format-version 1 --offline > /dev/null)
    (cd "$copy" && GIT_CEILING_DIRECTORIES="$work" sh scripts/release/build-local.sh > /dev/null)
    rm -rf "${out:?}/v$version"
    mkdir -p "$out/v$version"
    for f in "$copy/target/distrib"/*; do
        [ -f "$f" ] && cp "$f" "$out/v$version/"
    done
    grep -q "^APP_VERSION=\"$version\"$" "$out/v$version/kluris-installer.sh" || {
        echo "build-versions: the installer of v$version names another version" >&2
        exit 1
    }
    echo "built v$version into $out/v$version"
done
rm -rf "${work:?}"
