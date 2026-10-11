#!/bin/sh
# Rebuilds a Debian apt repository tree from .deb files already in pool/ (or passed in).
# Usage: scripts/apt-repo.sh <site-dir> [extra.deb ...]
# Requires: dpkg-dev, gnupg, gzip. Signs with the key available in GNUPGHOME / agent.
set -eu
if [ "$#" -lt 1 ]; then
    echo "usage: $0 <site-dir> [extra.deb ...]" >&2
    exit 1
fi
site=$(CDPATH= cd -- "$1" && pwd)
shift
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
pub="$root/packaging/linux/apt/ikterminal.gpg"
if [ ! -f "$pub" ]; then
    echo "missing public key: $pub (run scripts/generate-apt-key.sh)" >&2
    exit 1
fi
for tool in dpkg-scanpackages gpg dpkg-deb gzip sha256sum md5sum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$tool not found" >&2
        exit 1
    fi
done

mkdir -p "$site/pool/main"
install -m 644 "$pub" "$site/ikterminal.gpg"

for deb in "$@"; do
    [ -f "$deb" ] || {
        echo "missing deb: $deb" >&2
        exit 1
    }
    pkg=$(dpkg-deb -f "$deb" Package)
    ver=$(dpkg-deb -f "$deb" Version)
    arch=$(dpkg-deb -f "$deb" Architecture)
    dest="$site/pool/main/${pkg}_${ver}_${arch}.deb"
    install -m 644 "$deb" "$dest"
done

rm -rf "$site/dists"
arches=
for deb in "$site"/pool/main/*.deb; do
    [ -f "$deb" ] || continue
    arch=$(dpkg-deb -f "$deb" Architecture)
    case " $arches " in
        *" $arch "*) ;;
        *) arches="$arches $arch" ;;
    esac
done
arches=$(echo "$arches" | sed 's/^ *//')
if [ -z "$arches" ]; then
    echo "no .deb files in $site/pool/main" >&2
    exit 1
fi

scan=$(mktemp -d)
cleanup() { rm -rf "$scan"; }
trap cleanup EXIT

for arch in $arches; do
    bin="$site/dists/stable/main/binary-$arch"
    mkdir -p "$bin" "$scan/$arch/pool/main"
    for deb in "$site"/pool/main/*.deb; do
        [ -f "$deb" ] || continue
        if [ "$(dpkg-deb -f "$deb" Architecture)" = "$arch" ]; then
            ln -s "$deb" "$scan/$arch/pool/main/$(basename "$deb")"
        fi
    done
    (
        cd "$scan/$arch"
        dpkg-scanpackages -m pool/main /dev/null
    ) >"$bin/Packages"
    gzip -9n -c "$bin/Packages" >"$bin/Packages.gz"
    cat >"$bin/Release" <<EOF
Archive: stable
Component: main
Origin: IkTerminal
Label: IkTerminal
Architecture: $arch
EOF
done

release="$site/dists/stable/Release"
{
    cat <<EOF
Origin: IkTerminal
Label: IkTerminal
Suite: stable
Codename: stable
Architectures: $arches
Components: main
Description: IkTerminal apt repository
Date: $(date -Ru)
EOF
    echo "MD5Sum:"
    (
        cd "$site/dists/stable"
        find . -type f ! -path ./Release ! -path ./Release.gpg ! -path ./InRelease -printf '%P\n' | sort | while read -r rel; do
            size=$(wc -c <"$rel")
            sum=$(md5sum "$rel" | awk '{ print $1 }')
            printf " %s %8s %s\n" "$sum" "$size" "$rel"
        done
    )
    echo "SHA256:"
    (
        cd "$site/dists/stable"
        find . -type f ! -path ./Release ! -path ./Release.gpg ! -path ./InRelease -printf '%P\n' | sort | while read -r rel; do
            size=$(wc -c <"$rel")
            sum=$(sha256sum "$rel" | awk '{ print $1 }')
            printf " %s %8s %s\n" "$sum" "$size" "$rel"
        done
    )
} >"$release"

rm -f "$site/dists/stable/InRelease" "$site/dists/stable/Release.gpg"
gpg --batch --yes --clearsign -o "$site/dists/stable/InRelease" "$release"
gpg --batch --yes -abs -o "$site/dists/stable/Release.gpg" "$release"

cat >"$site/index.html" <<'EOF'
<!DOCTYPE html>
<html lang="ja">
<head>
  <meta charset="utf-8">
  <title>IkTerminal apt</title>
</head>
<body>
  <h1>IkTerminal apt repository</h1>
  <pre>
sudo mkdir -p /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/ikterminal.gpg \
  https://kenitp.github.io/IKTerminal/ikterminal.gpg
echo "deb [signed-by=/etc/apt/keyrings/ikterminal.gpg] https://kenitp.github.io/IKTerminal stable main" \
  | sudo tee /etc/apt/sources.list.d/ikterminal.list
sudo apt update
sudo apt install ikterminal
  </pre>
</body>
</html>
EOF

echo "$site"
