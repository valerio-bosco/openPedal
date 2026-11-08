#!/usr/bin/env bash
# Records the dry guitar with several buffer sizes, to find the smallest one that
# doesn't crackle. Run it inside `nix develop` and keep playing for the whole test.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q

BUFFERS=(${@:-48 96 128 144 192})
SECONDS_EACH=10
echo "Play clean (chords and single notes) for about $(( ${#BUFFERS[@]} * (SECONDS_EACH + 2) )) seconds."
read -rp "Press Enter and start playing... "
for b in "${BUFFERS[@]}"; do
    echo ">> buffer $b"
    # timeout sends SIGTERM and closes openPedal after the recording
    timeout -s TERM $((SECONDS_EACH + 2)) ./target/release/openpedal content/presets/dry.toml \
        --buffer "$b" --record "recordings/b$b.wav" --seconds "$SECONDS_EACH" 2>/dev/null || true
done
echo "Done: recordings/b*.wav"
