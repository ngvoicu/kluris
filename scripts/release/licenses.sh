#!/bin/sh
# Generate THIRD_PARTY_LICENSES.md, the licenses of the crates built into
# `kluris`, with the pinned cargo-about (about.toml, about.hbs). It reads the
# crates' own files and, for crates clarified in about.toml, the license file
# at their published git commit, so it needs the network.
#
#   scripts/release/licenses.sh           write THIRD_PARTY_LICENSES.md
#   scripts/release/licenses.sh --check   exit 1 when it is not up to date
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
about=$("$root/scripts/release/tool.sh" cargo-about)
out="$root/target/tools/THIRD_PARTY_LICENSES.md"
mkdir -p "$root/target/tools"

"$about" generate \
    --manifest-path "$root/crates/kluris-cli/Cargo.toml" \
    --config "$root/about.toml" \
    --locked --fail \
    --output-file "$out" \
    "$root/about.hbs"

case "${1:-}" in
    --check)
        if ! cmp -s "$out" "$root/THIRD_PARTY_LICENSES.md"; then
            diff -u "$root/THIRD_PARTY_LICENSES.md" "$out" || true
            echo "THIRD_PARTY_LICENSES.md is out of date: run scripts/release/licenses.sh" >&2
            exit 1
        fi
        echo "THIRD_PARTY_LICENSES.md is up to date"
        ;;
    "")
        cp "$out" "$root/THIRD_PARTY_LICENSES.md"
        echo "wrote THIRD_PARTY_LICENSES.md"
        ;;
    *) echo "usage: scripts/release/licenses.sh [--check]" >&2; exit 2 ;;
esac
