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
# The Demucs separation crate (openpedal-separate: demucs.cpp + Eigen with Accelerate BLAS) is linked
# into the core by default (ffi features `separate` + `native-tonelab`, the latter only when it exists
# in crates/ffi/Cargo.toml) and the Demucs weights are bundled in the app. Opt out with
# OPENPEDAL_SEPARATE=0 (plain core, no C++, no weights). Same toolchain needs as the probe.
# OPENPEDAL_DEMUCS_WEIGHTS overrides the weights path.
OPENPEDAL_SEPARATE="${OPENPEDAL_SEPARATE:-1}"
weights="${OPENPEDAL_DEMUCS_WEIGHTS:-$HOME/.local/share/openpedal/models/ggml-model-htdemucs-6s-f16.bin}"
if [ "$OPENPEDAL_SEPARATE" = 1 ] && [ ! -f "$weights" ]; then
    cat >&2 <<EOF
error: Demucs weights not found: $weights
Regenerate them (docs/spikes/demucs-cpp.md, section 2; output is work/ggml/ggml-model-htdemucs-6s-f16.bin):
  cd spikes/demucs-cpp && mkdir -p ~/.cache/torch/hub/checkpoints work/ggml
  curl -fL -o ~/.cache/torch/hub/checkpoints/5c90dfd2-34c22ccb.th https://dl.fbaipublicfiles.com/demucs/hybrid_transformer/5c90dfd2-34c22ccb.th
  ~/.local/share/openpedal-gpu/py convert.py third_party/demucs.cpp/scripts/convert-pth-to-ggml.py work/ggml --six-source
  mkdir -p ~/.local/share/openpedal/models && cp work/ggml/ggml-model-htdemucs-6s-f16.bin ~/.local/share/openpedal/models/
or set OPENPEDAL_DEMUCS_WEIGHTS=/path/to/file, or OPENPEDAL_SEPARATE=0 to build without separation.
EOF
    exit 1
fi
if [ "${OPENPEDAL_CXX_PROBE:-0}" = 1 ] || [ "$OPENPEDAL_SEPARATE" = 1 ]; then
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
fi
[ "${OPENPEDAL_CXX_PROBE:-0}" = 1 ] && features+=(--features openpedal-ffi/cxx-probe)
if [ "$OPENPEDAL_SEPARATE" = 1 ]; then
    for f in separate native-tonelab; do
        if grep -qE "^$f[[:space:]]*=" "$root/crates/ffi/Cargo.toml"; then
            features+=(--features "openpedal-ffi/$f"); echo "feature: $f (on)"
        else
            echo "feature: $f (missing in crates/ffi/Cargo.toml, skipped)"
        fi
    done
fi

cargo build --manifest-path "$root/Cargo.toml" -p openpedal-ffi --release --target "$target" "${features[@]}"

mkdir -p "$root/apps/ios/OpenPedalCore/lib"
cp "$root/target/$target/release/libopenpedal.a" "$root/apps/ios/OpenPedalCore/lib/"
rm -f "$root/apps/ios/OpenPedalCore/lib/.cxx-probe"
rm -f "$root/apps/ios/OpenPedalCore/lib/.separate"
[ "${OPENPEDAL_CXX_PROBE:-0}" = 1 ] && touch "$root/apps/ios/OpenPedalCore/lib/.cxx-probe"
[ "$OPENPEDAL_SEPARATE" = 1 ] && touch "$root/apps/ios/OpenPedalCore/lib/.separate"

# Bundled presets and pedals, copied by the app to its data dir on first launch.
res="$root/apps/ios/Sources/OpenPedalApp/Resources"
rm -rf "$res/presets" "$res/pedals" "$res/models" "$res/tonematch"
mkdir -p "$res"
cp -r "$root/content/presets" "$root/content/pedals" "$root/content/tonematch" "$res/"
# Demucs weights (52 MiB, not in git): the app passes the bundled path to openpedal_set_demucs_weights.
if [ "$OPENPEDAL_SEPARATE" = 1 ]; then
    mkdir -p "$res/models"
    cp "$weights" "$res/models/ggml-model-htdemucs-6s-f16.bin"
fi
echo "ok: apps/ios/OpenPedalCore/lib/libopenpedal.a + bundled presets/pedals from content/"

# Package.swift reads the markers (lib/.separate, Resources/models) while SwiftPM evaluates it, and
# SwiftPM caches that evaluation by the file's content, so a changed build mode would be ignored.
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/openpedal-xtool/.cache/org.swift.swiftpm/manifests"
