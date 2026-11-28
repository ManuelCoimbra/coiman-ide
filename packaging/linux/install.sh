#!/bin/sh
# Installs Coiman IDE from this folder: for you alone under ~/.local, or with --system for
# everyone under /usr/local (run that with sudo). Nothing is downloaded.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
if [ "${1:-}" = "--system" ]; then
    prefix=/usr/local
else
    prefix=${XDG_DATA_HOME:+$(dirname "$XDG_DATA_HOME")}
    prefix=${prefix:-$HOME/.local}
fi

mkdir -p "$prefix"
cp -R "$here/bin" "$here/share" "$prefix/"
# The launcher's Exec names the program; ~/.local/bin isn't always on PATH, so give its place.
desktop=$prefix/share/applications/io.github.manuelcoimbra.CoimanIde.desktop
sed -i "s|^Exec=coiman-ide|Exec=$prefix/bin/coiman-ide|" "$desktop"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "Coiman IDE is installed in $prefix. Run it from your applications menu or with:"
echo "  $prefix/bin/coiman-ide"
