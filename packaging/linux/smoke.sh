#!/bin/sh
# Checks an installed Coiman IDE starts: prints its version, then opens its window on a
# virtual screen, draws the first frame and exits (--measure-startup). Used by CI on each
# distribution:  smoke.sh [command, default coiman-ide]
set -eu

cmd=${1:-coiman-ide}
$cmd --version
out=$(mktemp)
rm -f "$out"
# Mesa's software renderer, so no graphics card is needed.
LIBGL_ALWAYS_SOFTWARE=1 timeout 120 xvfb-run -a -s "-screen 0 1280x800x24" $cmd --measure-startup "$out"
test -s "$out" || { echo "no first frame" >&2; exit 1; }
echo "first frame: $(cat "$out")"
