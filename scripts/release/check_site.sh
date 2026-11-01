#!/bin/sh
# Check that the website says only what kluris does: no retired words on a
# page, the site's AGENTS.md the same as its CLAUDE.md, and every
# `kluris <command> --flag` on a page known to the binary. The rules and
# what the parse misses are in check_site.py.
#
#   scripts/release/check_site.sh <site dir>
#
# The binary is $KLURIS_BIN, else this tree's release build (built first).
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/release/check_site.sh <site dir>" >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
site=$1

if [ -n "${KLURIS_BIN:-}" ]; then
    bin=$KLURIS_BIN
else
    (cd "$root" && cargo build --release --quiet -p kluris)
    bin="${CARGO_TARGET_DIR:-$root/target}/release/kluris"
fi
if [ ! -x "$bin" ]; then
    echo "check_site: no kluris binary at $bin" >&2
    exit 2
fi

exec python3 -I "$root/scripts/release/check_site.py" --bin "$bin" "$site"
