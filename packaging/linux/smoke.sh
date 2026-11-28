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

# Logging works: the log file exists and says how starting went.
log=$(find "$HOME" -name coiman-ide.log -path '*logs*' 2>/dev/null | head -n 1)
test -n "$log" || { echo "no log file under $HOME" >&2; exit 1; }
echo "log: $log"
grep -a "startup:" "$log" || { echo "the log says nothing about starting" >&2; exit 1; }

# A big project (20,000 folders) mustn't hold up the window: watching it for changes, one
# inotify watch per folder, happens off the window's thread.
big=$(mktemp -d)
for i in $(seq 1 200); do
    for j in $(seq 1 100); do mkdir -p "$big/d$i/e$j"; done
done
rm -f "$out"
LIBGL_ALWAYS_SOFTWARE=1 timeout 120 xvfb-run -a -s "-screen 0 1280x800x24" $cmd --measure-startup "$out" "$big"
test -s "$out" || { echo "no first frame with a big folder" >&2; exit 1; }
first=$(cut -d' ' -f2 "$out")
echo "first frame with 20,000 folders: $first ms"
test "$first" -lt 10000 || { echo "starting with a big folder took $first ms" >&2; exit 1; }
rm -rf "$big"
