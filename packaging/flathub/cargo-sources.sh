#!/bin/sh
# Lists every crate of Cargo.lock for Flathub's offline build:
#   cargo-sources.sh <output, such as packaging/flathub/cargo-sources.json>
# Uses flatpak-cargo-generator.py from flatpak-builder-tools (fetched here, with the Python
# modules it needs). Run it again whenever Cargo.lock changes.
set -eu

out=$1
root=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet aiohttp tomlkit
curl -fsSL -o "$work/flatpak-cargo-generator.py" \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
"$work/venv/bin/python" "$work/flatpak-cargo-generator.py" "$root/Cargo.lock" -o "$out"
echo "made $out ($(grep -c '"type": "archive"' "$out") crates)"
