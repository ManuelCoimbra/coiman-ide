# Installing Coiman IDE on Linux

Each release has Linux packages for x86_64 and ARM64 (aarch64) on its
[releases page](https://github.com/ManuelCoimbra/coiman-ide/releases). Pick the one your
distribution uses. `<version>` below is the release's version, such as `0.4.0`, and `<arch>`
is `x86_64` or `aarch64` (Debian packages say `amd64` or `arm64`).

Coiman IDE never updates itself and downloads nothing: updates come the way the package
came, when you ask for them.

Uninstalling never removes your settings (`~/.config/coiman-ide`) or your projects. Delete
that folder yourself if you want the settings gone too.

The program needs OpenGL and X11 or Wayland, which every desktop has. Packages name these
libraries, so the package manager brings any that are missing. The tools the IDE runs (git,
compilers, debuggers, language servers) are the ones you install yourself.

## Debian, Ubuntu, Linux Mint (.deb)

Install, or update to a newer release:

```
sudo apt install ./coiman-ide_<version>_amd64.deb
```

Remove:

```
sudo apt remove coiman-ide
```

## Fedora, RHEL, openSUSE (.rpm)

Install, or update to a newer release:

```
sudo dnf install ./coiman-ide-<version>-1.<arch>.rpm
sudo zypper install --allow-unsigned-rpm ./coiman-ide-<version>-1.<arch>.rpm
```

Use the first line on Fedora and RHEL, the second on openSUSE. Remove with
`sudo dnf remove coiman-ide` or `sudo zypper remove coiman-ide`.

## AppImage (any distribution)

One file that runs in place:

```
chmod +x Coiman_IDE-<version>-<arch>.AppImage
./Coiman_IDE-<version>-<arch>.AppImage
```

If it says FUSE is missing, install `libfuse2` (Ubuntu 24.04: `libfuse2t64`), or run it with
`--appimage-extract-and-run`. To update, download the newer AppImage. To remove, delete the
file.

## Flatpak

```
flatpak install --user ./coiman-ide-<arch>.flatpak
flatpak run io.github.manuelcoimbra.CoimanIde
```

To update, install the newer bundle the same way. To remove:

```
flatpak uninstall io.github.manuelcoimbra.CoimanIde
```

Inside Flatpak the IDE runs the tools of the Freedesktop SDK (git, gcc, make, Python). To
reach a tool that is installed on the computer itself, run it through
`flatpak-spawn --host`, for example as the command of a run configuration or in the
terminal.

## Snap

```
sudo snap install --dangerous --classic ./coiman-ide-<arch>.snap
```

`--classic` lets the IDE run the computer's own compilers and git, as other IDE snaps do;
`--dangerous` is needed for a snap that doesn't come from the Snap Store. To update, install
the newer snap the same way. Remove with `sudo snap remove coiman-ide`.

## Arch Linux

Each release attaches two PKGBUILDs with the version and checksums filled in:
`PKGBUILD-coiman-ide` builds from source, `PKGBUILD-coiman-ide-bin` uses the prebuilt
archive. Put one in an empty folder as `PKGBUILD`, then:

```
makepkg -si
```

To update, do the same with the newer release's PKGBUILD. Remove with
`sudo pacman -R coiman-ide` (or `coiman-ide-bin`).

## Portable archive (.tar.gz)

```
tar -xzf coiman-ide-<version>-linux-<arch>.tar.gz
cd coiman-ide-<version>-linux-<arch>
./install.sh
```

The archive brings no libraries: besides OpenGL it needs `libxkbcommon-x11` (Debian and
Ubuntu: `libxkbcommon-x11-0`), which desktops normally have already.

This installs for you alone, under `~/.local`, with a launcher in your applications menu.
Run `sudo ./install.sh --system` instead to install for everyone under `/usr/local`. To
update, do the same with the newer archive. To remove, run `./uninstall.sh` (or
`sudo ./uninstall.sh --system`) from the unpacked folder.

You can also run `bin/coiman-ide` from the unpacked folder without installing anything.

## How the packages are made and tested

`.github/workflows/linux.yml` builds the program on Ubuntu 22.04, whose C library is older
than any of the distributions below, so one build runs on all of them. It then makes every
format from the same files (`packaging/linux/stage.sh`). Before a release gets them, CI
installs each package and opens the IDE on a virtual screen:

- the deb on Debian 12, Ubuntu 22.04 and Ubuntu 24.04;
- the rpm on Fedora 41 and openSUSE Tumbleweed;
- the AppImage and the archive on Ubuntu 24.04;
- both PKGBUILDs on Arch Linux;
- the Flatpak in the Freedesktop runtime;
- the Snap on Ubuntu 24.04.

Each of these runs on x86_64 and ARM64, except Arch Linux, which is x86_64 only. CI also
checks that uninstalling leaves the settings in place.
