#!/bin/sh
# Opens Coiman IDE. Uses the newest of the release and debug binaries;
# pass arguments through (for example: scripts/open.sh /path/to/folder).
set -eu
cd "$(dirname "$0")/.."

release=target/release/coiman-ide
debug=target/debug/coiman-ide
bin=

if [ "${1:-}" = "--debug" ]; then
    shift
    bin=$debug
elif [ "${1:-}" = "--release" ]; then
    shift
    bin=$release
elif [ "${1:-}" = "--build" ]; then
    shift
    echo "Building Coiman IDE in release mode..." >&2
    cargo build --release -p coiman-ide
    bin=$release
elif [ -x "$release" ] && [ -x "$debug" ]; then
    if [ "$debug" -nt "$release" ]; then
        bin=$debug
    else
        bin=$release
    fi
elif [ -x "$release" ]; then
    bin=$release
elif [ -x "$debug" ]; then
    bin=$debug
fi

if [ -z "$bin" ] || [ ! -x "$bin" ]; then
    echo "No Coiman IDE binary found. Building release binary..." >&2
    cargo build --release -p coiman-ide
    bin=$release
fi

exec "$bin" "$@"
