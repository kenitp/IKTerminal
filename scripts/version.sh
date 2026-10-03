#!/bin/sh
# Prints the package version. The only source is Cargo.toml.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -n 1
