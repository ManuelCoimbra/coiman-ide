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

| Format | How it updates |
|---|---|
| AppImage | AppImageUpdate, or download the newer file |
| deb, rpm, Flatpak and Snap files, PKGBUILDs, tar.gz | Install the newer release's file the same way |
| APT or RPM repository (later) | With the system: `apt upgrade`, `dnf upgrade`, `zypper update` |
| Flathub, Snap Store (later) | With the system: `flatpak update`, snapd on its own |
| AUR (later) | AUR helpers such as `yay` or `paru` |

The repositories, Flathub, the Snap Store and the AUR are ready in the release workflow but
not online yet; they come with a later release. Their sections below say how they will work.

## Debian, Ubuntu, Linux Mint: the APT repository (later)

Add the repository once; after that, Coiman updates with the rest of the system.

```
sudo curl -fsSLo /usr/share/keyrings/coiman-ide.asc https://manuelcoimbra.github.io/coiman-ide/coiman-ide.asc
sudo curl -fsSLo /etc/apt/sources.list.d/coiman-ide.sources https://manuelcoimbra.github.io/coiman-ide/coiman-ide.sources
sudo apt update
sudo apt install coiman-ide
```

The first line gets the key that signs the repository; apt refuses anything it hasn't signed.
To stop getting updates from it, delete those two files. Remove Coiman with
`sudo apt remove coiman-ide`.

## Fedora, RHEL, openSUSE: the RPM repository (later)

Add the repository once; after that, Coiman updates with the rest of the system.

```
sudo curl -fsSLo /etc/yum.repos.d/coiman-ide.repo https://manuelcoimbra.github.io/coiman-ide/coiman-ide.repo
sudo dnf install coiman-ide
```

On openSUSE:

```
sudo zypper addrepo https://manuelcoimbra.github.io/coiman-ide/coiman-ide.repo
sudo zypper install coiman-ide
```

dnf and zypper ask you to accept the repository's key the first time. The repository and each
package in it are signed. To stop getting updates from it, run
`sudo rm /etc/yum.repos.d/coiman-ide.repo` or `sudo zypper removerepo coiman-ide`.

## Debian, Ubuntu, Linux Mint (.deb file)

Install, or update to a newer release:

```
sudo apt install ./coiman-ide_<version>_amd64.deb
```

Remove:

```
sudo apt remove coiman-ide
```

## Fedora, RHEL, openSUSE (.rpm file)

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
`--appimage-extract-and-run`. To remove, delete the file.

To update, download the newer AppImage, or let
[AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) do it: the AppImage says
where its updates are (the latest release on GitHub), and each release has a
`Coiman_IDE-<version>-<arch>.AppImage.zsync` beside the AppImage, so only the changed parts are
downloaded. Coiman itself never checks for updates.

Run this way (the AppImage, or `bin/coiman-ide` from the archive), the IDE adds its own
launcher entry and icons under `~/.local/share` the first time it starts, so that Wayland
desktops show its icon in the taskbar: Wayland finds a window's icon only through the
launcher named after it. It writes nothing when a package already installed the launcher.
To remove them, delete `~/.local/share/applications/io.github.manuelcoimbra.CoimanIde.desktop`
and `~/.local/share/icons/hicolor/*/apps/io.github.manuelcoimbra.CoimanIde.*`.

## Flatpak

Once Coiman is on Flathub, install it from there; `flatpak update` (or your software centre)
then brings each new version:

```
flatpak install flathub io.github.manuelcoimbra.CoimanIde
```

Or install the bundle from the release, which doesn't update on its own:

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

Once Coiman is in the Snap Store, install it from there; snapd then updates it by itself:

```
sudo snap install --classic coiman-ide
```

Or install the snap from the release, which doesn't update on its own:

```
sudo snap install --dangerous --classic ./coiman-ide-<arch>.snap
```

`--classic` lets the IDE run the computer's own compilers and git, as other IDE snaps do;
`--dangerous` is needed for a snap that doesn't come from the Snap Store. To update, install
the newer snap the same way. Remove with `sudo snap remove coiman-ide`.

## Arch Linux

Once Coiman is on the AUR, an AUR helper installs it and updates it with the system:

```
yay -S coiman-ide-bin
```

`coiman-ide-bin` is the prebuilt program; `coiman-ide` builds it from source.

Each release also attaches two PKGBUILDs with the version and checksums filled in:
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
update, do the same with the newer archive: no package manager stands behind the archive, so
nothing tells you a newer one exists. Use the repositories above to get updates. To remove, run `./uninstall.sh` (or
`sudo ./uninstall.sh --system`) from the unpacked folder.

You can also run `bin/coiman-ide` from the unpacked folder without installing anything; it
then adds its launcher entry as the AppImage does.

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
- the Snap on Ubuntu 24.04;
- the APT and RPM repositories on Debian 12, Ubuntu 24.04, Fedora 41 and openSUSE Tumbleweed,
  added as described above and signed with a throwaway key, so apt, dnf and zypper check
  every signature;
- the Flathub manifest, built from source without a network as Flathub builds it.

Each of these runs on x86_64 and ARM64, except Arch Linux, the repositories and the Flathub
manifest, which are x86_64 only. CI also checks that uninstalling leaves the settings in place.

`docs/PUBLISHING.md` explains how a release reaches the repositories and the stores.
