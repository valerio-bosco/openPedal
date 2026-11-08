# openPedal for iPad

Status: **draft, never built on a real device yet.** The Rust side (staticlib, C ABI) is built and tested on Linux; the Swift shell has not been compiled because it needs the Apple SDK. Expect small fixes on the first build.

## How it works

```
 iPad app (Swift, this folder)                    Rust core (libopenpedal.a)
 ┌───────────────────────────────┐   C ABI       ┌─────────────────────────────┐
 │ AVAudioSession (.playAndRecord │ openpedal.h  │ engine: graph + DSP         │
 │   .measurement, ~2.7 ms buffer)│──────────────▶ openpedal_process (per buffer)
 │ AVAudioEngine: sink (input) +  │              │ UI server on 127.0.0.1:port │
 │   source (output) nodes        │              │ (assets embedded, axum + WS)│
 │ WKWebView ── http://127.0.0.1 ─┼──────────────▶                             │
 └───────────────────────────────┘               └─────────────────────────────┘
```

- `openpedal_start(sample_rate, max_frames, data_dir, port)` builds the engine, loads `data_dir/presets/clean.toml` (else the first preset, else an empty graph), starts the UI server in background threads on loopback and returns when it accepts connections. `port = 0` picks a free port; read it with `openpedal_port`.
- `openpedal_process(handle, input, out_left, out_right, frames)` is called by the audio render callback. It only runs the real-time `Runner`: no allocation, locks or logging. Graph/parameter changes from the UI arrive through lock-free queues; a Rust thread frees retired plans.
- Call `openpedal_start` once per process: `openpedal_stop` stops the audio, but the UI server has no shutdown hook yet and lives until the app exits.
- `AudioHost.swift` assumes the input sink node and the output source node run in the same hardware cycle (both are in one `AVAudioEngine` on the same device), so the sink writes the guitar channel into a preallocated buffer and the source node reads it. If you hear glitches or one-buffer dropouts, replace that buffer with a small SPSC ring in Rust.
- Guitar input channel: index 1 (Scarlett Solo Inst jack), see `guitarChannel` in `AudioHost.swift`.
- Data dir: `Application Support/OpenPedal/{presets,pedals}`. On first launch the app copies the bundled `presets/` and `pedals/` (from the repo, synced into `Sources/OpenPedalApp/Resources` by `scripts/build-core.sh`) there, only for files that do not exist yet, so edits are never overwritten.

## Layout

| Path | What |
|---|---|
| `Package.swift`, `xtool.yml`, `Info.plist` | SwiftPM package and xtool config (microphone usage string, `UIBackgroundModes: audio`) |
| `OpenPedalCore/include/openpedal.h` | the C API, hand-written, keep in sync with `src/ffi.rs` |
| `OpenPedalCore/lib/libopenpedal.a` | built by `scripts/build-core.sh` (git-ignored) |
| `Sources/OpenPedalApp/` | `App.swift`, `AudioHost.swift`, `WebView.swift` |
| `scripts/build-core.sh` | UI build + `cargo build --lib --release --target aarch64-apple-ios` + copies |

## Build with xtool (from Linux)

Reference: <https://github.com/xtool-org/xtool> (check its docs if a command below has changed).

One-time setup:

1. Install the **Swift toolchain** (6.x) for Linux, and `usbmuxd` + `libimobiledevice` tools (on NixOS: `services.usbmuxd.enable = true;`). Make sure `swift --version` works.
2. Install **xtool** (release binary or AppImage from its GitHub releases) and put it in `PATH`.
3. Get **Xcode.xip** from <https://developer.apple.com/download/all/> (free Apple ID; pick an Xcode version supported by xtool) and run `xtool setup`. It asks for the Apple ID sign-in (used for free development signing) and the path to Xcode.xip, and extracts the Darwin SDK into `~/.xtool`.
4. On the iPad: **Settings > Privacy & Security > Developer Mode** on (it reboots). Connect it with USB-C and tap *Trust*.
5. Check with `xtool devices` that the iPad is listed.

Every build:

```sh
# 1. Rust core (UI embedded) + bundled presets, inside the dev shell
nix develop -c ios/scripts/build-core.sh

# 2. Swift shell: build, sign with your Apple ID, install and launch on the iPad
cd ios
xtool dev
```

`xtool dev build` only builds; `xtool dev run` runs the last build. Free Apple IDs sign apps for 7 days: re-run `xtool dev` weekly. The first run asks you to trust the developer profile on the iPad (Settings > General > VPN & Device Management). Allow the microphone when asked: iOS treats USB audio inputs as a microphone.

Simulator build of the core (no audio device there, only for UI checks): `TARGET=aarch64-apple-ios-sim nix develop -c ios/scripts/build-core.sh`.

## Known unknowns (first build)

- `Package.swift` finds `libopenpedal.a` through `-L <absolute path>` computed from `#filePath`; if xtool builds from a copy of the package, the path may need adjusting.
- `Bundle.module` (used for the bundled presets) needs xtool to bundle SwiftPM resources into the app; if not, copy the folders into the app bundle and use `Bundle.main`.
- If xtool refuses `unsafeFlags` for the root package, switch to an `.xcframework` binary target.
- Audio latency: start with 128 frames (`setPreferredIOBufferDuration(0.0027)`); iOS may grant more. The UI shows the real buffer in the meters.
