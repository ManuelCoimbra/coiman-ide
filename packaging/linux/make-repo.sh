#!/bin/sh
# Makes the signed APT and RPM repositories that Debian, Ubuntu, Fedora and openSUSE systems
# update Coiman IDE from:
#   make-repo.sh <folder with the .deb and .rpm files> <site folder> <site address>
# REPO_KEY is the fingerprint of a GPG key in the keyring (GNUPGHOME) that signs them.
# Needs apt-ftparchive (apt-utils), createrepo_c, rpmsign (rpm) and gpg.
#
# The site, served at <site address> (GitHub Pages in the release), holds:
#   coiman-ide.asc      the public key that apt, dnf and zypper check signatures with
#   coiman-ide.sources  the APT source, for /etc/apt/sources.list.d
#   coiman-ide.repo     the RPM repository, for /etc/yum.repos.d or zypper addrepo
#   deb/                dists/stable (signed InRelease) and pool/
#   rpm/                the signed packages and repodata (signed repomd.xml)
set -eu

pkgs=$(cd "$1" && pwd)
mkdir -p "$2"
site=$(cd "$2" && pwd)
url=${3%/}
: "${REPO_KEY:?set REPO_KEY to the signing key's fingerprint}"

gpg --armor --export "$REPO_KEY" > "$site/coiman-ide.asc"

# APT: one distribution, "stable", with the packages of every architecture.
deb=$site/deb
mkdir -p "$deb/pool/main"
cp "$pkgs"/*.deb "$deb/pool/main/"
for arch in amd64 arm64; do
    dir=$deb/dists/stable/main/binary-$arch
    mkdir -p "$dir"
    (cd "$deb" && apt-ftparchive --arch "$arch" packages pool/main) > "$dir/Packages"
    gzip -9kf "$dir/Packages"
done
(cd "$deb/dists/stable" && apt-ftparchive \
    -o APT::FTPArchive::Release::Origin="Coiman IDE" \
    -o APT::FTPArchive::Release::Label="Coiman IDE" \
    -o APT::FTPArchive::Release::Suite=stable \
    -o APT::FTPArchive::Release::Codename=stable \
    -o APT::FTPArchive::Release::Architectures="amd64 arm64" \
    -o APT::FTPArchive::Release::Components=main \
    release . > Release)
gpg --batch --yes --local-user "$REPO_KEY" --clearsign -o "$deb/dists/stable/InRelease" "$deb/dists/stable/Release"
gpg --batch --yes --local-user "$REPO_KEY" --armor --detach-sign -o "$deb/dists/stable/Release.gpg" "$deb/dists/stable/Release"
cat > "$site/coiman-ide.sources" <<EOF
Types: deb
URIs: $url/deb
Suites: stable
Components: main
Signed-By: /usr/share/keyrings/coiman-ide.asc
EOF

# RPM: the packages are signed too, as dnf and zypper check each one by default.
rpm=$site/rpm
mkdir -p "$rpm"
cp "$pkgs"/*.rpm "$rpm/"
rpmsign --define "_gpg_name $REPO_KEY" --define "_openpgp_sign_id $REPO_KEY" --addsign "$rpm"/*.rpm >/dev/null
createrepo_c --quiet "$rpm"
gpg --batch --yes --local-user "$REPO_KEY" --armor --detach-sign -o "$rpm/repodata/repomd.xml.asc" "$rpm/repodata/repomd.xml"
cat > "$site/coiman-ide.repo" <<EOF
[coiman-ide]
name=Coiman IDE
baseurl=$url/rpm
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$url/coiman-ide.asc
EOF

cat > "$site/index.html" <<EOF
<!doctype html>
<meta charset="utf-8">
<title>Coiman IDE packages</title>
<h1>Coiman IDE packages</h1>
<p>Signed APT and RPM repositories of <a href="https://github.com/ManuelCoimbra/coiman-ide">Coiman IDE</a>.
How to add them: <a href="https://github.com/ManuelCoimbra/coiman-ide/blob/main/docs/INSTALL-LINUX.md">docs/INSTALL-LINUX.md</a>.</p>
<ul>
<li><a href="coiman-ide.asc">coiman-ide.asc</a> (signing key)</li>
<li><a href="coiman-ide.sources">coiman-ide.sources</a> (APT)</li>
<li><a href="coiman-ide.repo">coiman-ide.repo</a> (dnf, zypper)</li>
</ul>
EOF
echo "made the repositories in $site for $url"
