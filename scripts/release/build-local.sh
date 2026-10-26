#!/bin/sh
# Build a local release for this machine into target/distrib/ the way
# release.yml does on GitHub: dist's local artifacts for one target (the
# archive and its .sha256), its manifest beside them, then the global ones
# (kluris-installer.sh with the archive's SHA-256 inside, kluris-installer.ps1,
# sha256.sum).
#
#   scripts/release/build-local.sh [target]   default: rustc's host target
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
dist=$("$root/scripts/release/tool.sh" dist)
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
out="$root/target/distrib"

cd "$root"
rm -rf "${out:?}"
mkdir -p "$root/target/tools"
"$dist" build --output-format=json --artifacts=local --target="$target" > "$root/target/tools/local-dist-manifest.json"
cp "$root/target/tools/local-dist-manifest.json" "$out/$target-dist-manifest.json"
"$dist" build --output-format=json --artifacts=global > "$root/target/tools/global-dist-manifest.json"
echo "built $target into $out:"
ls "$out"
