#!/bin/sh
# Builds the release binary, tar.gz, and Ubuntu/Debian .deb.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
version=$("$root/scripts/version.sh")
if [ -z "$version" ]; then
    echo "version not found in Cargo.toml" >&2
    exit 1
fi
cargo build --release
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$root/$target_dir" ;;
esac
release_dir="$target_dir/release"
if [ ! -f "$release_dir/ikterminal" ]; then
    echo "missing release binary: $release_dir/ikterminal" >&2
    exit 1
fi
arch=$(uname -m)
case "$arch" in
    x86_64) deb_arch=amd64 ;;
    aarch64) deb_arch=arm64 ;;
    arm64)
        arch=aarch64
        deb_arch=arm64
        ;;
    *)
        echo "unsupported architecture: $arch" >&2
        exit 1
        ;;
esac
out="$root/target/dist"
mkdir -p "$out"
archive="$out/IkTerminal-${version}-linux-${arch}.tar.gz"
tar -C "$release_dir" -czf "$archive" ikterminal
echo "$archive"

if ! command -v dpkg-deb >/dev/null 2>&1; then
    echo "dpkg-deb not found (install dpkg-dev)" >&2
    exit 1
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
pkg="$stage/ikterminal"
mkdir -p \
    "$pkg/DEBIAN" \
    "$pkg/usr/bin" \
    "$pkg/usr/share/applications" \
    "$pkg/usr/share/xdg-terminals" \
    "$pkg/usr/share/icons/hicolor/256x256/apps" \
    "$pkg/usr/share/icons/hicolor/64x64/apps" \
    "$pkg/usr/share/doc/ikterminal"

install -m 755 "$release_dir/ikterminal" "$pkg/usr/bin/ikterminal"
ln -s ikterminal "$pkg/usr/bin/ikt"
install -m 644 "$root/packaging/linux/ikterminal.desktop" "$pkg/usr/share/applications/ikterminal.desktop"
install -m 644 "$root/packaging/linux/ikterminal.desktop" "$pkg/usr/share/xdg-terminals/ikterminal.desktop"
install -m 755 "$root/packaging/linux/postinst" "$pkg/DEBIAN/postinst"
install -m 755 "$root/packaging/linux/prerm" "$pkg/DEBIAN/prerm"
install -m 644 "$root/assets/icon-256.png" \
    "$pkg/usr/share/icons/hicolor/256x256/apps/ikterminal.png"
install -m 644 "$root/assets/icon-64.png" \
    "$pkg/usr/share/icons/hicolor/64x64/apps/ikterminal.png"

printf '%s\n' \
    'IkTerminal' \
    '' \
    'Lightweight terminal with SSH/SFTP.' \
    "https://github.com/kenitp/IKTerminal" \
    >"$pkg/usr/share/doc/ikterminal/README"

size=$(du -sk "$pkg" | cut -f1)
cat >"$pkg/DEBIAN/control" <<EOF
Package: ikterminal
Version: ${version}
Section: utils
Priority: optional
Architecture: ${deb_arch}
Installed-Size: ${size}
Depends: libxcb1, libxkbcommon0, libxkbcommon-x11-0, libgl1
Maintainer: IkTerminal <https://github.com/kenitp/IKTerminal>
Description: Lightweight terminal with SSH/SFTP
 Tabbed terminal emulator with local shells, SSH, and SFTP.
EOF

deb="$out/IkTerminal-${version}-linux-${arch}.deb"
dpkg-deb --root-owner-group --build "$pkg" "$deb"
echo "$deb"
