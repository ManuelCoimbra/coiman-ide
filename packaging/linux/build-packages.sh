#!/bin/sh
# Makes the tar.gz, deb, rpm and AppImage of Coiman IDE:
#   build-packages.sh <coiman-ide binary> <version> <outdir>
# The machine's architecture (x86_64 or aarch64) is the packages'. Needs dpkg-deb for the
# deb, rpmbuild for the rpm, and APPIMAGETOOL (a path to appimagetool) for the AppImage; a
# format whose tool is missing is skipped with a note.
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
rpm_libs="libEGL.so.1()(64bit) libGL.so.1()(64bit) libxkbcommon.so.0()(64bit) libxkbcommon-x11.so.0()(64bit) libwayland-client.so.0()(64bit) libX11.so.6()(64bit) libXcursor.so.1()(64bit) libXrandr.so.2()(64bit) libXi.so.6()(64bit)"

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
    echo "made $(basename "$work"/rpmbuild/RPMS/*/coiman-ide-*.rpm)"
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
    cat > "$app/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/coiman-ide" "$@"
EOF
    chmod 755 "$app/AppRun"
    ARCH=$arch "$APPIMAGETOOL" --no-appstream "$app" "$out/Coiman_IDE-$version-$arch.AppImage" >/dev/null
    echo "made Coiman_IDE-$version-$arch.AppImage"
else
    echo "no APPIMAGETOOL: skipping the AppImage"
fi
