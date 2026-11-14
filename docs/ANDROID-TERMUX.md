# Coiman IDE on Android, in Termux

Coiman IDE runs on an Android phone or tablet as a Linux program inside
[Termux](https://termux.dev). Its window shows in the
[Termux:X11](https://github.com/termux/termux-x11) app. The Linux it runs in is installed in
Termux with `proot-distro`, so the ARM64 Debian package from the releases page installs as
on any Debian. Its compilers, debuggers, git and language servers come from that Linux, with
`apt`.

## Install

You need the Termux and Termux:X11 apps, both from F-Droid or GitHub (not the Play Store).
Then, in Termux:

```
pkg install x11-repo proot-distro
pkg install termux-x11-nightly
termux-setup-storage
proot-distro install debian
```

`termux-setup-storage` asks Android to let Termux reach the phone's shared storage
(Download, Documents…). The File Manager then lists it under **Phone storage**.

Download `coiman-ide_<version>_arm64.deb` from the
[releases page](https://github.com/ManuelCoimbra/coiman-ide/releases) to the phone's Download
folder. In Termux, copy it into Debian's home folder:

```
cp ~/storage/shared/Download/coiman-ide_*_arm64.deb $PREFIX/var/lib/proot-distro/installed-rootfs/debian/root/
```

Then install it inside Debian, with the tools you'll use (for example git, gcc and Python):

```
proot-distro login debian --shared-tmp
apt update
apt install ./coiman-ide_*_arm64.deb git build-essential python3
```

## Run

In Termux, start the X server, then Coiman IDE inside Debian:

```
termux-x11 :1 &
proot-distro login debian --shared-tmp -- env DISPLAY=:1 coiman-ide
```

Then switch to the Termux:X11 app.

## On the phone

- **Touch sizes.** On a phone, buttons, rows and scroll bars are larger for fingers. The
  setting is **Touch sizes** (`workbench.touch`): auto, on or off.
- **The on-screen keyboard.** Termux:X11's own keyboard is shown and hidden with Android's
  Back button. Programs in it can't change that, so Coiman IDE has its own keyboard at the
  bottom of the window, made for code. It has letters, a **?123** layer with the brackets and
  symbols code needs, and Esc, Tab, Ctrl, Alt, Home, End and the arrows. Ctrl and Alt hold
  for the next key: Ctrl, then S saves. Ctrl+C, X and V use the phone's clipboard.
- **Its button** at the top right, beside Settings ("Keys: AUTO"), shows the mode and changes it:
  - **OFF**: never shown.
  - **ON**: shown while something takes typing.
  - **AUTO**: shown on a phone while something takes typing, and hidden for 30 seconds after a
    hardware keyboard is used.
  The mode is kept in settings (`workbench.soft_keyboard`), and **View: Cycle On-screen
  Keyboard** does the same as the button. The editor gives the keyboard its room, and keeps
  its cursor and text. Leave Termux:X11's keyboard hidden while using it.
- **The File Manager** lists **Phone storage** and, when it can be reached from the Linux it
  runs in, **Termux home**, before the usual places.

## What to expect

- The program and everything it runs are ARM64 builds. Most of Debian's packages exist for
  ARM64; a few tools, such as some debuggers, don't work under `proot`.
- `proot` makes file access slower than on a computer. Building large projects takes longer.
- Android may stop Termux in the background. Keep it in the foreground or use Termux's
  wake lock (`termux-wake-lock`) during long builds.
