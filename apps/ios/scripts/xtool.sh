#!/usr/bin/env bash
# Runs xtool inside the openpedal-xtool container (Swift 6.4 + xtool), connected to:
#   - the host's usbmuxd socket, to reach the iPad over USB (NixOS: services.usbmuxd.enable)
#   - this repository, at the same path as on the host (Package.swift uses absolute paths)
#   - ~/Downloads (read-only), where Xcode.xip usually is
#   - a persistent home (~/.local/share/openpedal-xtool) for the Darwin SDK and the Apple ID session,
#     also mounted on /home/ubuntu: the image maps uid 1000 to "ubuntu" and some tools use that home
# Usage: apps/ios/scripts/xtool.sh <xtool arguments>, e.g. `setup`, `devices`, `dev`.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../../.." && pwd)"
home="${XDG_DATA_HOME:-$HOME/.local/share}/openpedal-xtool"
image=openpedal-xtool
mkdir -p "$home"

if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker build -t "$image" "$repo/apps/ios/docker"
fi

tty=()
[ -t 0 ] && tty=(-it)
exec docker run --rm "${tty[@]}" \
    --user "$(id -u):$(id -g)" -e HOME=/home/xtool -v "$home:/home/xtool" \
    -v "$home:/home/ubuntu" \
    -v "$repo:$repo" -w "$repo/apps/ios" \
    -v /var/run/usbmuxd:/var/run/usbmuxd \
    -v "$HOME/Downloads:/downloads:ro" \
    --network host \
    "$image" xtool "$@"
