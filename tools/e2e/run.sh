#!/usr/bin/env bash
# Browser tests of the UI: starts `openpedal ui --no-audio` on a temporary copy of
# content/presets/ and content/pedals/, drives a headless Chromium over the DevTools protocol and runs
# every test. Usage, inside `nix develop`, after `cargo build --release`:
#   tools/e2e/run.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
port=7811
dir="$(mktemp -d)"
mkdir -p "$dir/content"
cp -r "$root/content/presets" "$root/content/pedals" "$dir/content/"
cp "$root/openpedal.toml" "$dir/"
git -C "$root" ls-files --error-unmatch content/presets >/dev/null   # only tracked presets are expected
for f in "$dir"/content/presets/*.toml; do git -C "$root" ls-files --error-unmatch "content/presets/$(basename "$f")" >/dev/null 2>&1 || rm "$f"; done
chrome="$(nix build --no-link --print-out-paths nixpkgs#chromium)/bin/chromium"

(cd "$dir" && exec "$root/target/release/openpedal" ui --no-audio --no-browser --port $port >"$dir/server.log" 2>&1) &
server=$!
sleep 1.5
"$chrome" --headless=new --no-sandbox --disable-gpu --remote-debugging-port=9333 --window-size=1500,900 \
    --user-data-dir="$dir/chrome" "http://127.0.0.1:$port/" >/dev/null 2>&1 &
browser=$!
# Chromium's helper processes can still be writing to the profile after the main process exits:
# retry the removal and keep the tests' exit status.
cleanup() {
    local status=$?
    kill -TERM $browser $server 2>/dev/null
    wait 2>/dev/null
    for _ in 1 2 3 4 5 6 7 8 9 10; do rm -rf "$dir" 2>/dev/null && break; sleep 0.3; done
    exit $status
}
trap cleanup EXIT
sleep 3

status=0
node "$root/tools/e2e/history.test.mts" || status=1
for t in editing new-graph delete-preset designer tonelab; do
    echo "== $t"
    node "$root/tools/e2e/$t.mjs" $port "$dir" || status=1
done
exit $status
