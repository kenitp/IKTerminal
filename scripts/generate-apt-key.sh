#!/bin/sh
# Generates a signing key for the GitHub Pages apt repository.
# Writes the public key into the repo and prints the private key for a GitHub secret.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out="$root/packaging/linux/apt"
mkdir -p "$out"
name=${APT_KEY_NAME:-IkTerminal Apt}
email=${APT_KEY_EMAIL:-apt@ikterminal.local}
priv="$out/ikterminal-private.asc"
pub="$out/ikterminal.gpg"

if [ -f "$pub" ] && [ "${1:-}" != "--force" ]; then
    echo "public key already exists: $pub" >&2
    echo "re-run with --force to replace it" >&2
    exit 1
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
chmod 700 "$tmpdir"
export GNUPGHOME="$tmpdir"

gpg --batch --pinentry-mode loopback --passphrase '' --quick-generate-key \
    "$name <$email>" default default never

fingerprint=$(gpg --list-keys --with-colons | awk -F: '/^fpr:/ { print $10; exit }')
gpg --batch --armor --export-secret-keys "$fingerprint" >"$priv"
gpg --batch --export "$fingerprint" >"$pub"
chmod 600 "$priv"
chmod 644 "$pub"

echo "public key:  $pub  (commit this file)"
echo "private key: $priv (do NOT commit; add as GitHub secret APT_GPG_PRIVATE_KEY)"
echo
echo "gh secret set APT_GPG_PRIVATE_KEY < \"$priv\""
echo
echo "Then enable GitHub Pages: Settings > Pages > Deploy from branch > gh-pages / (root)"
