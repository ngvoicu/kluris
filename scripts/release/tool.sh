#!/bin/sh
# Print the path of a pinned release tool, fetching it first when it is not
# there yet: the prebuilt binary for this machine, its archive checked against
# the SHA-256 below, unpacked into its own folder under target/tools/
# (KLURIS_TOOLS_DIR overrides). Nothing is installed anywhere else.
#
#   scripts/release/tool.sh dist|cargo-about|actionlint
#
# The hashes are the GitHub release assets' own digests.
set -eu

tool="${1:-}"
root=$(cd "$(dirname "$0")/../.." && pwd)
tools="${KLURIS_TOOLS_DIR:-$root/target/tools}"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) platform=darwin-arm64 ;;
    Darwin-x86_64) platform=darwin-x86_64 ;;
    Linux-x86_64) platform=linux-x86_64 ;;
    Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
    *) echo "tool.sh: no pinned tools for $(uname -s) $(uname -m)" >&2; exit 2 ;;
esac

case "$tool-$platform" in
    dist-darwin-arm64)
        version=0.33.1; asset=cargo-dist-aarch64-apple-darwin.tar.xz
        sha=b8bf041a38e9e54ecf41eee2574ca6644043796620bb0ee8340e9c0acbaaae8f ;;
    dist-darwin-x86_64)
        version=0.33.1; asset=cargo-dist-x86_64-apple-darwin.tar.xz
        sha=e9dc303035fbb2f44295948e0a076f1d1d218a0328e9b84e16955863bba92bf4 ;;
    dist-linux-x86_64)
        version=0.33.1; asset=cargo-dist-x86_64-unknown-linux-gnu.tar.xz
        sha=da68e159d5fed1ee61b00284da444e719927dbd0b7d0e2c34e57a04414453cf9 ;;
    dist-linux-arm64)
        version=0.33.1; asset=cargo-dist-aarch64-unknown-linux-gnu.tar.xz
        sha=e883f4c6c17f2458d6ced950d978c04b45423ed3a5fdbebcb8f8de568389e1c1 ;;
    cargo-about-darwin-arm64)
        version=0.9.2; asset=cargo-about-0.9.2-aarch64-apple-darwin.tar.gz
        sha=ae72f0df0c399a1e96336f696fa55b1b28679fd725632eba8cf8e4568467cc3e ;;
    cargo-about-linux-x86_64)
        version=0.9.2; asset=cargo-about-0.9.2-x86_64-unknown-linux-musl.tar.gz
        sha=9099a59e820c38a68b9d65f300662a567d56562f9a10f6aa4c7e86c17c2566af ;;
    cargo-about-linux-arm64)
        version=0.9.2; asset=cargo-about-0.9.2-aarch64-unknown-linux-musl.tar.gz
        sha=af5169282fb6f84e13471493f405437e43ac517744c9ae12fbe2cdf0a6f0e5a8 ;;
    actionlint-darwin-arm64)
        version=1.7.12; asset=actionlint_1.7.12_darwin_arm64.tar.gz
        sha=aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f ;;
    actionlint-linux-x86_64)
        version=1.7.12; asset=actionlint_1.7.12_linux_amd64.tar.gz
        sha=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
    actionlint-linux-arm64)
        version=1.7.12; asset=actionlint_1.7.12_linux_arm64.tar.gz
        sha=325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6 ;;
    *) echo "tool.sh: no pinned '$tool' for $platform (dist, cargo-about, actionlint)" >&2; exit 2 ;;
esac

case "$tool" in
    dist) url="https://github.com/axodotdev/cargo-dist/releases/download/v$version/$asset"; exe=dist ;;
    cargo-about) url="https://github.com/EmbarkStudios/cargo-about/releases/download/$version/$asset"; exe=cargo-about ;;
    actionlint) url="https://github.com/rhysd/actionlint/releases/download/v$version/$asset"; exe=actionlint ;;
esac

dir="$tools/$tool-$version-$platform"
found=$(find "$dir" -type f -name "$exe" 2>/dev/null | head -n 1)
if [ -n "$found" ] && [ -x "$found" ]; then
    echo "$found"
    exit 0
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

download="$tools/download-$tool-$$"
rm -rf "${download:?}"
mkdir -p "$download"
trap 'rm -rf "${download:?}"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$download/$asset"
got=$(sha256 "$download/$asset")
if [ "$got" != "$sha" ]; then
    echo "tool.sh: $asset has SHA-256 $got, expected $sha" >&2
    exit 1
fi
mkdir -p "$download/unpacked"
case "$asset" in
    *.tar.xz) tar -xJf "$download/$asset" -C "$download/unpacked" ;;
    *.tar.gz) tar -xzf "$download/$asset" -C "$download/unpacked" ;;
esac
rm -rf "${dir:?}"
mv "$download/unpacked" "$dir"
found=$(find "$dir" -type f -name "$exe" | head -n 1)
if [ -z "$found" ]; then
    echo "tool.sh: $asset holds no $exe" >&2
    exit 1
fi
echo "$found"
