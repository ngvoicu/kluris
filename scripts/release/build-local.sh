#!/bin/sh
# Build a local release for this machine the way release.yml does on GitHub:
# dist's local artifacts for one target (the archive and its .sha256), its
# manifest beside them, then the global ones (kluris-installer.sh with the
# archive's SHA-256 inside, kluris-installer.ps1, sha256.sum, the source
# tarball). They land in target/distrib, which the script prints last: dist
# writes there even when CARGO_TARGET_DIR sends the build elsewhere.
#
#   scripts/release/build-local.sh [target]   default: rustc's host target
#
# dist makes the source tarball with `git archive HEAD`, so a tree that is no
# git checkout (the unpacked tarball of `cargo xtask linux`) is made a
# one-commit repository of its own first.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
dist=$("$root/scripts/release/tool.sh" dist)
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
out="$root/target/distrib"
manifests="$root/target/dist-manifests"

cd "$root"
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git init -q
    git add -A
    git -c user.name=kluris-ci -c user.email=kluris-ci@localhost commit -q -m "release tree"
fi
rm -rf "${out:?}" "${manifests:?}"
mkdir -p "$manifests"
"$dist" build --output-format=json --artifacts=local --target="$target" > "$manifests/local.json"
cp "$manifests/local.json" "$out/$target-dist-manifest.json"
"$dist" build --output-format=json --artifacts=global > "$manifests/global.json"
ls "$out" >&2
echo "$out"
