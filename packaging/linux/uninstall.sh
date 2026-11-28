#!/bin/sh
# Removes what install.sh put in place (add --system if it was installed that way). Your
# settings (~/.config/coiman-ide) and your projects stay.
set -eu

if [ "${1:-}" = "--system" ]; then
    prefix=/usr/local
else
    prefix=${XDG_DATA_HOME:+$(dirname "$XDG_DATA_HOME")}
    prefix=${prefix:-$HOME/.local}
fi
id=io.github.manuelcoimbra.CoimanIde

rm -f "$prefix/bin/coiman-ide"
rm -f "$prefix/share/applications/$id.desktop" "$prefix/share/metainfo/$id.metainfo.xml"
rm -f "$prefix"/share/icons/hicolor/*/apps/$id.png "$prefix/share/icons/hicolor/scalable/apps/$id.svg"
rm -rf "$prefix/share/doc/coiman-ide" "$prefix/share/licenses/coiman-ide"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$prefix/share/applications" || true
echo "Coiman IDE is removed from $prefix. Settings in ~/.config/coiman-ide are kept."
