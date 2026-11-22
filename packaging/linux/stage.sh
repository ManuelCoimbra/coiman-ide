#!/bin/sh
# Lays out Coiman IDE as installed under a prefix, inside DESTDIR:
#   stage.sh <coiman-ide binary> <destdir> [prefix, default /usr]
# Every Linux package (tar.gz, deb, rpm, AppImage, Flatpak, Snap, Arch) is made from this tree.
set -eu

bin=$1
dest=$2
# An empty prefix (the portable archive) is kept as empty.
prefix=${3-/usr}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
id=io.github.manuelcoimbra.CoimanIde
out=$dest$prefix

install -Dm755 "$bin" "$out/bin/coiman-ide"
install -Dm644 "$here/$id.desktop" "$out/share/applications/$id.desktop"
install -Dm644 "$here/$id.metainfo.xml" "$out/share/metainfo/$id.metainfo.xml"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "$root/assets/icon/coiman-$size.png" "$out/share/icons/hicolor/${size}x${size}/apps/$id.png"
done
install -Dm644 "$root/assets/icon/coiman.svg" "$out/share/icons/hicolor/scalable/apps/$id.svg"
for doc in README.md CHANGELOG.md PRIVACY.md; do
    install -Dm644 "$root/$doc" "$out/share/doc/coiman-ide/$doc"
done
install -Dm644 "$root/LICENSE" "$out/share/licenses/coiman-ide/LICENSE"
# Extensions that come with Coiman, installed only when the user asks (Extensions view).
for ext in "$root"/plugins/extensions/*/; do
    name=$(basename "$ext")
    install -Dm644 "$ext/plugin.toml" "$out/share/coiman-ide/extensions/$name/plugin.toml"
    install -Dm644 "$ext/plugin.wasm" "$out/share/coiman-ide/extensions/$name/plugin.wasm"
done
