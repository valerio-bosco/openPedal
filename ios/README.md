# openPedal for iPad

Status: **builds and installs on a real iPad** (2026-11-08). The UI is the desktop node editor and is not touch-friendly yet: a tablet/phone frontend is the next task (docs/PLAN.md §9).

## How it works

```
 iPad app (Swift, this folder)                    Rust core (libopenpedal.a)
 ┌───────────────────────────────┐   C ABI       ┌─────────────────────────────┐
 │ AVAudioSession (.playAndRecord │ openpedal.h  │ engine: graph + DSP         │
 │   .measurement, ~2.7 ms buffer)│──────────────▶ openpedal_process (per buffer)
 │ AVAudioEngine: sink (input) +  │              │ UI server on 127.0.0.1:port │
 │   source (output) nodes        │              │ (assets embedded, axum + WS)│
 │ SwiftUI ── ws://127.0.0.1/ws ──┼──────────────▶ (same WebSocket protocol §5.4)│
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
| `Sources/OpenPedalApp/` | `App.swift` (root, ambient), `AudioHost.swift`, `Protocol.swift` (Codable protocol), `WSClient.swift`, `Store.swift` (@Observable state, undo, throttled set_param), `Chain.swift` (chain derivation), `Board.swift` (pedals), `Inspector.swift` (knobs), `Toolbar.swift`, `Sheets.swift` (add pedal, About), `Theme.swift` |
| `scripts/build-core.sh` | UI build + `cargo build --lib --release --target aarch64-apple-ios` + copies |

## Build with xtool (from Linux) — verified on 2026-11-08

Verified on NixOS with an iPad Air 5 (iPadOS 26.2.1), xtool 1.24.0, Swift 6.4.2 and Xcode 27.2.
xtool runs in a Docker container (`ios/docker/Dockerfile`, official Swift 6.4 image), because
nixpkgs ships an older Swift: `ios/scripts/xtool.sh <args>` runs it with the host's usbmuxd socket,
the repository and a persistent home (`~/.local/share/openpedal-xtool`: Darwin SDK ~7 GB in
`.swiftpm/`, Apple ID session in `.config/xtool/`).

One-time setup:

1. Host: `services.usbmuxd.enable = true;` (NixOS) and `libimobiledevice` for `ideviceinfo`; Docker.
2. Download **Xcode.xip** with a free Apple ID from <https://developer.apple.com/download/all/>, then
   `ios/scripts/xtool.sh sdk install /downloads/<Xcode file>.xip` (`~/Downloads` is mounted on `/downloads`).
3. `ios/scripts/xtool.sh auth login` (interactive: Apple ID, password, 2FA code).
4. iPad on USB-C, unlocked, *Trust* the computer. Developer Mode: `idevicedevmodectl reveal`, then
   Settings > Privacy & Security > Developer Mode (reboots). Check with `idevicedevmodectl list` → `enabled`.

Every build (iPad connected and unlocked):

```sh
nix develop -c ios/scripts/build-core.sh   # Rust staticlib (UI embedded) + bundled presets/pedals
ios/scripts/xtool.sh dev                   # build, sign, install, launch
```

First launch on the iPad: Settings > General > VPN & Device Management > trust your Apple ID; allow the
microphone (iOS treats USB audio inputs as a microphone). Then unplug the iPad from the computer and plug
the audio interface into it. Free Apple IDs sign for 7 days: reconnect and re-run `xtool.sh dev`.

Troubleshooting:
- `Mux error (-8)` / `LockdownClient.Error.muxError`: after an iPad reboot the stored pairing can become
  invalid. Unlock the iPad with the passcode, then `sudo systemctl restart usbmuxd`, replug, *Trust* again.
- The app needs iOS 17 (`AVAudioApplication`); `Package.swift` targets `.iOS(.v17)`.

## Known unknowns (first build)

- `Package.swift` finds `libopenpedal.a` through `-L <absolute path>` computed from `#filePath`; if xtool builds from a copy of the package, the path may need adjusting.
- `Bundle.module` (used for the bundled presets) needs xtool to bundle SwiftPM resources into the app; if not, copy the folders into the app bundle and use `Bundle.main`.
- If xtool refuses `unsafeFlags` for the root package, switch to an `.xcframework` binary target.
- Audio latency: start with 128 frames (`setPreferredIOBufferDuration(0.0027)`); iOS may grant more. The UI shows the real buffer in the meters.

## Native UI notes

- The UI needs iPadOS 26 (`glassEffect`, `GlassEffectContainer`, `.glass`/`.glassProminent` button styles). No WebView.
- It talks to the in-process server over `ws://127.0.0.1:<port>/ws` and tolerates a core without `protocol`/`bypass` (docs/PLAN.md §10.2).
- Parallel graphs are shown in topological order; adding, removing and moving pedals is only enabled for a simple chain.
