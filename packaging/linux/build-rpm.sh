#!/bin/sh
# Builds the Coiman IDE RPM for this machine's architecture:
#   build-rpm.sh <coiman-ide binary> <version> <outdir>
# The RPM installs under /usr and carries the desktop entry, icon and metadata.
set -eu

bin=$1
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
arch=$(uname -m)
case $arch in
    x86_64 | aarch64) ;;
    *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac
id=io.github.manuelcoimbra.CoimanIde
# Libraries the window loads at run time (not linked, so no tool finds them on its own):
# OpenGL through EGL or GLX, and X11 or Wayland with the keyboard map.
rpm_libs="libEGL.so.1()(64bit) libGL.so.1()(64bit) libxkbcommon.so.0()(64bit) libxkbcommon-x11.so.0()(64bit) libwayland-client.so.0()(64bit) libX11.so.6()(64bit) libXcursor.so.1()(64bit) libXrandr.so.2()(64bit) libXi.so.6()(64bit)"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

root=$work/rpmroot
"$here/stage.sh" "$bin" "$root" /usr
{
    echo "Name: coiman-ide"
    echo "Version: $version"
    echo "Release: 1"
    echo "Summary: Code editor and IDE written in Rust"
    echo "License: MIT"
    echo "URL: https://github.com/ManuelCoimbra/coiman-ide"
    for lib in $rpm_libs; do echo "Requires: $lib"; done
    echo "Recommends: git"
    echo
    echo "%description"
    echo "Edits, runs, tests and debugs code, and works with Git. No AI features,"
    echo "and no network connections of its own."
    echo
    echo "%install"
    echo "cp -a $root/. %{buildroot}/"
    echo
    echo "%files"
    echo "/usr/bin/coiman-ide"
    echo "/usr/share/applications/$id.desktop"
    echo "/usr/share/metainfo/$id.metainfo.xml"
    echo "/usr/share/icons/hicolor/*/apps/$id.*"
    echo "%doc /usr/share/doc/coiman-ide"
    echo "%license /usr/share/licenses/coiman-ide"
} > "$work/coiman-ide.spec"

rpmbuild -bb --quiet \
    --define "_topdir $work/rpmbuild" \
    --define "_build_id_links none" \
    --define "debug_package %{nil}" \
    --define "__strip /bin/true" \
    "$work/coiman-ide.spec"

cp "$work"/rpmbuild/RPMS/*/coiman-ide-*.rpm "$out/"
echo "made $(basename "$work"/rpmbuild/RPMS/*/coiman-ide-*.rpm) in $out"