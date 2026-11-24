#!/bin/sh
# Writes the files of the Flathub repository (github.com/flathub/io.github.manuelcoimbra.CoimanIde)
# for one release:
#   for-flathub.sh <tag, such as v0.7.0> <its commit> <output folder>
# The manifest's source becomes the release's git tag and commit, and cargo-sources.json
# (made by cargo-sources.sh beside this script) goes along. The release attaches the result as
# flathub-<version>.tar.gz; its files replace those in the Flathub repository.
set -eu

tag=$1
commit=$2
out=$3
here=$(cd "$(dirname "$0")" && pwd)
id=io.github.manuelcoimbra.CoimanIde
test -f "$here/cargo-sources.json" || { echo "no cargo-sources.json: run cargo-sources.sh first" >&2; exit 1; }
mkdir -p "$out"

awk -v tag="$tag" -v commit="$commit" '
    /^      - type: dir$/ {
        print "      - type: git"
        print "        url: https://github.com/ManuelCoimbra/coiman-ide.git"
        print "        tag: " tag
        print "        commit: " commit
        print "        x-checker-data:"
        print "          type: git"
        print "          tag-pattern: ^v([\\d.]+)$"
        skipping = 1
        next
    }
    skipping && /^      - / { skipping = 0 }
    skipping { next }
    { print }
' "$here/$id.yml" | grep -v "for-flathub.sh replaces this" > "$out/$id.yml"
cp "$here/cargo-sources.json" "$out/"
# Flathub builds for both architectures.
printf '{\n  "only-arches": ["x86_64", "aarch64"]\n}\n' > "$out/flathub.json"
grep -q "tag: $tag" "$out/$id.yml"
echo "made $out/$id.yml for $tag ($commit), with cargo-sources.json and flathub.json"
