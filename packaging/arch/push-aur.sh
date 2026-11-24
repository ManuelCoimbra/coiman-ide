#!/bin/sh
# Sends a release's PKGBUILDs to the AUR, as root on Arch Linux (the release workflow does it):
#   push-aur.sh <folder with PKGBUILD-coiman-ide and PKGBUILD-coiman-ide-bin> <version>
# AUR_SSH_KEY is the private SSH key whose public half is in the AUR account. Pushing a package
# that isn't on the AUR yet makes it. makepkg doesn't run as root, so a "builder" user does it.
set -eu

pkgs=$(cd "$1" && pwd)
version=$2
: "${AUR_SSH_KEY:?set AUR_SSH_KEY to the AUR account's private SSH key}"

id builder >/dev/null 2>&1 || useradd -m builder
home=/home/builder
install -d -m 700 -o builder "$home/.ssh"
printf '%s\n' "$AUR_SSH_KEY" > "$home/.ssh/aur"
ssh-keyscan -t ed25519,rsa aur.archlinux.org > "$home/.ssh/known_hosts" 2>/dev/null
printf 'Host aur.archlinux.org\n  User aur\n  IdentityFile ~/.ssh/aur\n' > "$home/.ssh/config"
chmod 600 "$home/.ssh/aur" "$home/.ssh/config"
chown -R builder "$home/.ssh"
chmod -R a+rX "$pkgs"

for pkg in coiman-ide coiman-ide-bin; do
    su builder -s /bin/sh -c "
        set -eu
        cd ~ && rm -rf $pkg && git clone -q ssh://aur@aur.archlinux.org/$pkg.git && cd $pkg
        cp '$pkgs/PKGBUILD-$pkg' PKGBUILD
        makepkg --printsrcinfo > .SRCINFO
        git add PKGBUILD .SRCINFO
        if git diff --cached --quiet; then echo '$pkg: already at $version'; exit 0; fi
        git -c user.name='Manuel Pereira' -c user.email='ManuelCoimbra@users.noreply.github.com' commit -q -m 'Update to $version'
        git push -q origin HEAD:master
        echo '$pkg: $version sent to the AUR'
    "
done
