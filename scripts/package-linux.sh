#!/bin/sh
# Builds the release binary and a tar.gz named with the Cargo.toml version.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
version=$("$root/scripts/version.sh")
if [ -z "$version" ]; then
    echo "version not found in Cargo.toml" >&2
    exit 1
fi
cargo build --release
arch=$(uname -m)
case "$arch" in
    x86_64 | aarch64) ;;
    arm64) arch=aarch64 ;;
    *)
        echo "unsupported architecture: $arch" >&2
        exit 1
        ;;
esac
out="$root/target/dist"
mkdir -p "$out"
archive="$out/IkTerminal-${version}-linux-${arch}.tar.gz"
tar -C "$root/target/release" -czf "$archive" ikterminal
echo "$archive"
