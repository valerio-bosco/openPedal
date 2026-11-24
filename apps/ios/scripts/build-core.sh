#!/usr/bin/env bash
# Builds the Rust core for the iPad and syncs what the Swift package needs.
# Run it from anywhere, inside `nix develop`:   nix develop -c apps/ios/scripts/build-core.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
target="${TARGET:-aarch64-apple-ios}"   # aarch64-apple-ios-sim for the Simulator

# The UI is embedded in the library by rust-embed: build it first.
(cd "$root/apps/desktop/ui" && npm install --no-audit --no-fund && npm run build)

# Opt-in spike (docs/spikes/ios-cxx.md): OPENPEDAL_CXX_PROBE=1 also compiles C++17 + Eigen into the
# library. Needs the nix shell (clang, Eigen) and the xtool iPhoneOS SDK. Off by default.
features=()
if [ "${OPENPEDAL_CXX_PROBE:-0}" = 1 ]; then
    sdkdir="${XDG_DATA_HOME:-$HOME/.local/share}/openpedal-xtool/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms"
    case "$target" in
        *-sim) sdk="$sdkdir/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"; triple="arm64-apple-ios17.0-simulator" ;;
        *)     sdk="$sdkdir/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk";               triple="arm64-apple-ios17.0" ;;
    esac
    [ -d "$sdk" ] || { echo "error: SDK not found: $sdk (run xtool sdk install)" >&2; exit 1; }
    # nixpkgs clang-unwrapped can target Apple; the SDK provides libc++ headers and libc++.tbd.
    wrap="$(mktemp -d)/clangxx-ios"
    printf '#!/bin/sh\nexec clang++ -target %s "$@"\n' "$triple" > "$wrap"
    chmod +x "$wrap"
    export SDKROOT="$sdk"
    export "CXX_${target//-/_}=$wrap"
    export "CXXFLAGS_${target//-/_}=-fno-lto -Wno-#warnings"
    export "AR_${target//-/_}=llvm-ar"   # BSD/Darwin archive format; GNU ar output is rejected by rustc
    features=(--features openpedal-ffi/cxx-probe)
fi

cargo build --manifest-path "$root/Cargo.toml" -p openpedal-ffi --release --target "$target" "${features[@]}"

mkdir -p "$root/apps/ios/OpenPedalCore/lib"
cp "$root/target/$target/release/libopenpedal.a" "$root/apps/ios/OpenPedalCore/lib/"
rm -f "$root/apps/ios/OpenPedalCore/lib/.cxx-probe"
[ "${OPENPEDAL_CXX_PROBE:-0}" = 1 ] && touch "$root/apps/ios/OpenPedalCore/lib/.cxx-probe"

# Bundled presets and pedals, copied by the app to its data dir on first launch.
res="$root/apps/ios/Sources/OpenPedalApp/Resources"
rm -rf "$res/presets" "$res/pedals"
mkdir -p "$res"
cp -r "$root/content/presets" "$root/content/pedals" "$res/"
echo "ok: apps/ios/OpenPedalCore/lib/libopenpedal.a + bundled presets/pedals from content/"
