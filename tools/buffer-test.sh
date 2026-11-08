#!/usr/bin/env bash
# Registra la chitarra (senza effetti) con diversi buffer, per trovare il più piccolo
# che non crepita. Va lanciato dentro `nix develop`. Suona di continuo per tutta la prova.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q

BUFFERS=(${@:-48 96 128 144 192})
SECONDS_EACH=10
echo "Suona in pulito (accordi e note singole) per circa $(( ${#BUFFERS[@]} * (SECONDS_EACH + 2) )) secondi."
read -rp "Premi Invio e inizia a suonare... "
for b in "${BUFFERS[@]}"; do
    echo ">> buffer $b"
    # timeout manda SIGTERM e chiude openPedal dopo la registrazione
    timeout -s TERM $((SECONDS_EACH + 2)) ./target/release/openpedal presets/dry.toml \
        --buffer "$b" --record "recordings/b$b.wav" --seconds "$SECONDS_EACH" 2>/dev/null || true
done
echo "Fatto: recordings/b*.wav"
