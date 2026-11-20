#!/bin/sh
# Opens Coiman IDE without rebuilding. Uses the newest of the release and debug
# binaries; pass arguments through (for example: scripts/open.sh /path/to/folder).
set -eu
cd "$(dirname "$0")/.."

release=target/release/coiman-ide
debug=target/debug/coiman-ide
bin=
if [ -x "$release" ] && [ -x "$debug" ]; then
    if [ "$debug" -nt "$release" ]; then bin=$debug; else bin=$release; fi
elif [ -x "$release" ]; then
    bin=$release
elif [ -x "$debug" ]; then
    bin=$debug
fi

if [ -z "$bin" ]; then
    echo "No Coiman IDE binary found. Build it first with:" >&2
    echo "  cargo build --release -p coiman-ide" >&2
    exit 1
fi

exec "$bin" "$@"