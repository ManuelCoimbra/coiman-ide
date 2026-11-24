#!/bin/sh
# Makes the tar.gz, deb, rpm and AppImage of Coiman IDE:
#   build-packages.sh <coiman-ide binary> <version> <outdir>
# The machine's architecture (x86_64 or aarch64) is the packages'. Needs dpkg-deb for the
# deb, rpmbuild for the rpm, and APPIMAGETOOL (a path to appimagetool) for the AppImage; a
# format whose tool is missing is skipped with a note.
# With GITHUB_RELEASES set to owner/repo, the AppImage says where its updates are (the latest
# GitHub release of that repository) and a .zsync file is made beside it, for AppImageUpdate;
# without it, the AppImage has no update information.
set -eu

bin=$1
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
arch=$(uname -m)
case $arch in
    x86_64) deb_arch=amd64 ;;
    aarch64) deb_arch=arm64 ;;
    *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
id=io.github.manuelcoimbra.CoimanIde

# Libraries the window loads at run time (not linked, so no tool finds them on its own):
# OpenGL through EGL or GLX, and X11 or Wayland with the keyboard map.
deb_libs="libc6, libgl1, libegl1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-6, libxcursor1, libxrandr2, libxi6"

# Portable archive: the tree under no prefix, with install.sh and uninstall.sh.
name=coiman-ide-$version-linux-$arch
"$here/stage.sh" "$bin" "$work/tar/$name" ""
install -m755 "$here/install.sh" "$here/uninstall.sh" "$work/tar/$name/"
tar -C "$work/tar" -czf "$out/$name.tar.gz" "$name"
echo "made $name.tar.gz"

# Debian package.
if command -v dpkg-deb >/dev/null; then
    root=$work/deb
    "$here/stage.sh" "$bin" "$root" /usr
    mkdir -p "$root/DEBIAN"
    size=$(du -sk "$root/usr" | cut -f1)
    cat > "$root/DEBIAN/control" <<EOF
Package: coiman-ide
Version: $version
Architecture: $deb_arch
Maintainer: Manuel Pereira <noreply@github.com>
Installed-Size: $size
Depends: $deb_libs
Recommends: git, xdg-desktop-portal
Section: devel
Priority: optional
Homepage: https://github.com/ManuelCoimbra/coiman-ide
Description: Code editor and IDE written in Rust
 Edits, runs, tests and debugs code, and works with Git. No AI features,
 and no network connections of its own.
EOF
    dpkg-deb --root-owner-group --build "$root" "$out/coiman-ide_${version}_$deb_arch.deb" >/dev/null
    echo "made coiman-ide_${version}_$deb_arch.deb"
else
    echo "no dpkg-deb: skipping the deb"
fi

# RPM package (Fedora, openSUSE, RHEL): library names, so it fits each distribution's packages.
if command -v rpmbuild >/dev/null; then
    "$here/build-rpm.sh" "$bin" "$version" "$out"
else
    echo "no rpmbuild: skipping the rpm"
fi

# AppImage: one file that runs in place.
if [ -n "${APPIMAGETOOL:-}" ]; then
    app=$work/AppDir
    "$here/stage.sh" "$bin" "$app" /usr
    cp "$here/$id.desktop" "$app/$id.desktop"
    cp "$app/usr/share/icons/hicolor/256x256/apps/$id.png" "$app/$id.png"
    ln -s "$id.png" "$app/.DirIcon"
    # Not every system has the X11 keyboard-map library the window loads: it travels along.
    mkdir -p "$app/usr/lib"
    for lib in libxkbcommon-x11.so.0 libxcb-xkb.so.1; do
        found=$(ldconfig -p | awk -v l="$lib" '$1 == l { print $NF; exit }')
        if [ -n "$found" ]; then cp -L "$found" "$app/usr/lib/"; else echo "AppImage: no $lib to bundle" >&2; exit 1; fi
    done
    cat > "$app/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/coiman-ide" "$@"
EOF
    chmod 755 "$app/AppRun"
    appimage=Coiman_IDE-$version-$arch.AppImage
    if [ -n "${GITHUB_RELEASES:-}" ]; then
        owner=${GITHUB_RELEASES%%/*}
        repo=${GITHUB_RELEASES#*/}
        update="gh-releases-zsync|$owner|$repo|latest|Coiman_IDE-*-$arch.AppImage.zsync"
        ARCH=$arch "$APPIMAGETOOL" --no-appstream -u "$update" "$app" "$out/$appimage" >/dev/null
        if [ ! -f "$out/$appimage.zsync" ]; then echo "AppImage: no $appimage.zsync made" >&2; exit 1; fi
        echo "made $appimage and $appimage.zsync (updates: $update)"
    else
        ARCH=$arch "$APPIMAGETOOL" --no-appstream "$app" "$out/$appimage" >/dev/null
        echo "made $appimage (no update information)"
    fi
else
    echo "no APPIMAGETOOL: skipping the AppImage"
fi
