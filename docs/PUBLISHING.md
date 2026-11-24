# How a release reaches users, and how each copy gets updates

Coiman never updates itself and opens no connection to look for updates. Each way of
installing it has an updater outside the program: the system's package manager, winget,
Scoop, a store, or AppImageUpdate. This page is for the maintainer: what a release does on its
own, and the one-time setup each channel needs.

## What a release does

Pushing a tag such as `v0.7.0` runs two workflows.

`.github/workflows/release.yml` (Windows) makes the release and attaches:

- the zip;
- the setup program, `coiman-ide-<version>-windows-x64-setup.exe` (Inno Setup,
  `packaging/windows/coiman-ide.iss`), after installing it, checking it and uninstalling it;
- `coiman-ide.json`, the Scoop manifest. Scoop users install from
  `releases/latest/download/coiman-ide.json`, and `scoop update` reads that address again.

Then, with the `WINGET_TOKEN` secret set and the package already in winget's catalogue, it
opens the pull request for the new version in `microsoft/winget-pkgs`.

`.github/workflows/linux.yml` builds and tests every Linux package, attaches them, then:

- signs the APT and RPM repositories with the `REPO_GPG_KEY` secret and puts them on GitHub
  Pages, at <https://manuelcoimbra.github.io/coiman-ide/>;
- uploads the snaps to the Snap Store's stable channel, with the `SNAPCRAFT_STORE_CREDENTIALS`
  secret;
- pushes both PKGBUILDs to the AUR, with the `AUR_SSH_KEY` secret;
- attaches `flathub-<version>.tar.gz`, the files for the Flathub repository.

Each of these steps is skipped, with a note in the log, while its secret is missing. Nothing
fails because a channel isn't set up yet.

| Channel | Secret | One-time setup | Then each release |
|---|---|---|---|
| Setup program, zip | none | none | automatic |
| Scoop | none | none | automatic |
| AppImageUpdate | none | none | automatic |
| APT and RPM repository | `REPO_GPG_KEY` | signing key, Pages | automatic |
| winget | `WINGET_TOKEN` | first version by hand | automatic pull request, merged by winget's moderators |
| Snap Store | `SNAPCRAFT_STORE_CREDENTIALS` | register the name, ask for classic | automatic |
| AUR | `AUR_SSH_KEY` | account and SSH key | automatic |
| Flathub | none | submission | a pull request by hand, from the release's file |

Secrets are set in the repository's Settings > Secrets and variables > Actions, or with
`gh secret set <NAME>`. They never go in a file in the repository.

## APT and RPM repository (GitHub Pages)

The repository is rebuilt on each release from that release's deb and rpm files
(`packaging/linux/make-repo.sh`), so it always holds the newest version. apt checks the
signed `InRelease`; dnf and zypper check the signed `repomd.xml` and each signed package.

1. A signing key. It is an RSA key with no passphrase (the workflow can't type one), used for
   nothing else. The workflow publishes its public half as `coiman-ide.asc`. Keep a copy of
   the private key outside GitHub, in a password manager: if it is lost, every user has to add
   a new key.

   ```
   gpg --batch --passphrase '' --quick-gen-key "Coiman IDE packages" rsa4096 sign never
   gpg --armor --export-secret-keys "Coiman IDE packages" | gh secret set REPO_GPG_KEY
   ```

2. GitHub Pages, served by the workflow: Settings > Pages > Source: **GitHub Actions**. The
   release runs from a tag, so the `github-pages` environment must allow tags: Settings >
   Environments > github-pages > Deployment branches and tags > Add rule > Tag, `v*`. Both at
   once from a terminal:

   ```
   gh api -X POST repos/ManuelCoimbra/coiman-ide/pages -f build_type=workflow
   gh api -X POST repos/ManuelCoimbra/coiman-ide/environments/github-pages/deployment-branch-policies -f name='v*' -f type=tag
   ```

If the key ever leaks, make a new one, replace the secret, make a release, and tell users to
fetch `coiman-ide.asc` again (docs/INSTALL-LINUX.md).

## winget

winget's catalogue is the `microsoft/winget-pkgs` repository. The package is
`ManuelCoimbra.CoimanIDE` and installs with the setup program, so the first version in it is
the first release that has one.

1. A token for the pull requests: GitHub > Settings > Developer settings > Personal access
   tokens > Tokens (classic) > Generate, with only the `public_repo` scope.
2. The first version, by hand, after that release is out:

   ```
   powershell -File packaging/windows/winget-manifests.ps1 -Version 0.7.0 -Out winget
   winget validate --manifest winget
   wingetcreate submit winget --token <the token>
   ```

   `wingetcreate` (`winget install Microsoft.WingetCreate`) forks `winget-pkgs` and opens the
   pull request. winget's checks install the program in a sandbox; a moderator merges it,
   usually within a few days.
3. Once it is merged: `gh secret set WINGET_TOKEN` with the same token. From then on each
   release opens the pull request for its version.

## Snap Store

The snap uses classic confinement (it runs the computer's compilers, debuggers and git), which
the Snap Store grants only on request. snapcraft runs on Linux (an Ubuntu in WSL works).

1. A Snap Store account at <https://snapcraft.io/account>, then:

   ```
   sudo snap install snapcraft --classic
   snapcraft login
   snapcraft register coiman-ide
   ```

2. Ask for classic confinement: a post in the **store-requests** category of
   <https://forum.snapcraft.io>, saying that Coiman IDE is an IDE that runs the user's own
   toolchains, as other IDE snaps do.
3. Once granted, the credentials for the workflow:

   ```
   snapcraft export-login --snaps=coiman-ide --acls package_access,package_push,package_update,package_release - | gh secret set SNAPCRAFT_STORE_CREDENTIALS
   ```

   They expire after a year; run the same line again then.

## AUR (Arch User Repository)

The AUR is a set of git repositories, one per package, each holding a `PKGBUILD` and its
`.SRCINFO`. Arch users install from it with a helper such as `yay`, which also updates them.
You don't need Arch Linux: the release workflow does the work in an Arch container. Both names,
`coiman-ide` (built from source) and `coiman-ide-bin` (the prebuilt program), were free on
2026-11-24.

1. An account: <https://aur.archlinux.org/register>. Pick a user name and give an email
   address; the account needs no Arch Linux.
2. An SSH key for the AUR alone. In PowerShell:

   ```
   ssh-keygen -t ed25519 -N '""' -C "aur coiman-ide" -f $HOME\.ssh\aur
   ```

   Leave it without a passphrase: the workflow uses it without anyone to type one.
3. Give the AUR the public half: log in at <https://aur.archlinux.org>, open **My Account**,
   paste the content of `$HOME\.ssh\aur.pub` into **SSH Public Key**, type your password at
   the bottom, and save.
4. Give the workflow the private half, then remove your local copy if you don't want to push
   by hand:

   ```
   Get-Content $HOME\.ssh\aur -Raw | gh secret set AUR_SSH_KEY
   ```

5. The next release pushes both packages. The first push makes them, with your account as
   their maintainer; the packages appear at `https://aur.archlinux.org/packages/coiman-ide-bin`.

After that, users may leave comments or flag a package out of date on its AUR page: the AUR
emails you. A new release clears the flag. To push by hand (from Linux or WSL with `git` and
`makepkg`), clone `ssh://aur@aur.archlinux.org/coiman-ide-bin.git`, copy the release's
`PKGBUILD-coiman-ide-bin` in as `PKGBUILD`, run `makepkg --printsrcinfo > .SRCINFO`, commit
and push. `packaging/arch/push-aur.sh` does the same.

## Flathub

Flathub builds the app itself, from source and without a network, and serves it to
`flatpak update` and software centres. The manifest is
`packaging/flathub/io.github.manuelcoimbra.CoimanIde.yml`; `cargo-sources.json` lists the
crates it needs. CI builds it on each run, exactly as Flathub would.

1. The submission, once, from the newest release's `flathub-<version>.tar.gz`:
   - fork <https://github.com/flathub/flathub> and make a branch from its `new-pr` branch;
   - unpack the file into the branch's root (`io.github.manuelcoimbra.CoimanIde.yml`,
     `cargo-sources.json`, `flathub.json`), commit and push;
   - open a pull request against `new-pr`, titled `Add io.github.manuelcoimbra.CoimanIde`.
     Say why it needs `--filesystem=host` (an IDE edits projects anywhere) and
     `--talk-name=org.freedesktop.Flatpak` (it runs the computer's own tools with
     `flatpak-spawn --host`).

   Flathub's reviewers comment on the pull request; the app ID `io.github.manuelcoimbra` is
   verified through the ManuelCoimbra GitHub account. Once merged, you get write access to
   `github.com/flathub/io.github.manuelcoimbra.CoimanIde`.
2. Each release: unpack its `flathub-<version>.tar.gz` into that repository over the old
   files, and open a pull request. Flathub builds it; merging publishes it.

Flathub also wants screenshots in the metadata (`packaging/linux/*.metainfo.xml`); its review
says so if they are missing or out of date.
