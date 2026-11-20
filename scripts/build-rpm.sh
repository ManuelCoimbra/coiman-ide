#!/bin/sh
# Builds the release binary and the RPM for this machine, leaving the RPM in the
# project root: coiman-ide-<version>-1.<arch>.rpm
set -eu

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/coiman-ide/Cargo.toml | head -n 1)
if [ -z "$version" ]; then
    echo "could not read the version from crates/coiman-ide/Cargo.toml" >&2
    exit 1
fi

echo "building coiman-ide $version in release mode"
cargo build --release --locked -p coiman-ide

exec packaging/linux/build-rpm.sh target/release/coiman-ide "$version" .