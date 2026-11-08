#!/usr/bin/env bash
# Builds the Rust core for the iPad and syncs what the Swift package needs.
# Run it from anywhere, inside `nix develop`:   nix develop -c ios/scripts/build-core.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
target="${TARGET:-aarch64-apple-ios}"   # aarch64-apple-ios-sim for the Simulator

# The UI is embedded in the library by rust-embed: build it first.
(cd "$root/ui" && npm install --no-audit --no-fund && npm run build)

cargo build --manifest-path "$root/Cargo.toml" --lib --release --target "$target"

mkdir -p "$root/ios/OpenPedalCore/lib"
cp "$root/target/$target/release/libopenpedal.a" "$root/ios/OpenPedalCore/lib/"

# Bundled presets and pedals, copied by the app to its data dir on first launch.
res="$root/ios/Sources/OpenPedalApp/Resources"
rm -rf "$res/presets" "$res/pedals"
mkdir -p "$res"
cp -r "$root/presets" "$root/pedals" "$res/"
echo "ok: ios/OpenPedalCore/lib/libopenpedal.a + bundled presets/pedals"
