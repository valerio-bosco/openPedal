# openPedal: architecture, contracts and plan

This is the **starting context** for anyone (human or agent) working on the project: read it together with
`CONTRIBUTING.md` before changing code. Status: 2026-11-08, node-graph editor integrated with the real engine.

---

## 1. Goal

A **node editor** (like Blender's or Unreal's) where:

- every **node is a pedal**; nodes are wired with cables, in series or in parallel;
- the signal starts at the **Guitar** node (`input`) and is heard when it reaches the **Amp** node (`output`);
- every knob works **live**, while you play;
- new pedals can be **created from scratch**, both from the UI and from files:
  - a **composite pedal** is a sub-graph of basic blocks with some knobs "exposed" (UI: select nodes → *Create pedal*; files: `pedals/*.toml`);
  - a **basic block** is Rust code in `src/dsp/`. A future `formula` block will let users write DSP as math expressions from the UI (phase 3);
- it runs on **Linux, Windows and macOS** (Windows/macOS audio backend: task E1).

## 2. UI technology

**Decision:** a web UI (TypeScript + React + React Flow / `@xyflow/react`, built with Vite) served by an **axum** server inside openPedal, with the assets **embedded in the binary** (`rust-embed`).

- One executable per OS; `openpedal ui` opens the browser. Node.js is only needed by developers (it is in the Nix flake).
- It also works from a **phone** on the local network (`--lan`), as a wireless pedalboard.
- A native window can be added later with Tauri, reusing the same UI.
- Rejected: Electron (ships Chromium + Node, two processes to keep in sync with the Rust engine), egui (node editors far less polished).

## 3. Architecture

```
 ┌──────────── Browser / phone ───────────────┐
 │  React Flow UI: palette, nodes, cables,     │
 │  knobs, meters, presets, undo/redo          │
 └──────────────┬──────────────────────────────┘
                │ WebSocket JSON (protocol §5.4)
 ┌──────────────▼──────────────────────────────┐
 │ src/server/   axum: HTTP (assets) + WS       │
 │               EngineHandle trait, MockEngine │
 │ src/live.rs   EngineHandle on the real engine│
 ├──────────────────────────────────────────────┤
 │ src/graph/    model, loading, compilation    │
 │               (composite pedals → blocks,    │
 │               topological order), executor   │
 ├──────────────────────────────────────────────┤
 │ src/dsp/      basic blocks + catalog         │
 │               (parameter descriptors)        │
 ├──────────────────────────────────────────────┤
 │ src/engine.rs JACK/PipeWire (Linux)          │
 │ src/audio/    cpal backend (Win/macOS, E1)   │
 └──────────────────────────────────────────────┘
```

**Real-time rule, always:** in `process`, in graph execution and in `set_param`, no allocations, locks, I/O or logging.
Changes reach the audio thread through lock-free queues (`rtrb`); old structures are freed outside the audio thread.

## 4. Concepts

| Term | Meaning |
|---|---|
| **Basic block** | Rust DSP: `gain`, `drive`, `filter`, `cab`, `compressor`… Described by a `BlockDesc` |
| **Composite pedal** | sub-graph of basic blocks and/or other pedals, with exposed `knob`s. File `pedals/<id>.toml` |
| **Graph** (preset) | nodes + cables + screen positions. File `presets/<name>.toml` (format v2) |
| `input` ("Guitar") | special node: the guitar. Exactly one, no inputs |
| `output` ("Amp") | special node: the output, with `volume_db`. Exactly one. Without a path to it there is no sound |
| Fan-out / fan-in | many cables may leave one output (the signal is copied); cables entering the same node are **summed** |
| Cycles | **forbidden** (the compiler rejects them). Feedback lives inside blocks, e.g. the delay |

The graph signal is **mono**; true stereo is planned for phase 3.

## 5. Contracts

### 5.1 Parameters and catalog: `src/dsp/params.rs`

```rust
pub enum ParamKind {
    Number { min: f32, max: f32, default: f32, unit: &'static str, log: bool, step: f32 },
    Choice { options: &'static [&'static str], default: usize },  // value = option index, as f32
    File   { extensions: &'static [&'static str] },               // changing it rebuilds the block
}
pub struct ParamDesc { pub id: &'static str, pub label: &'static str, pub kind: ParamKind }
pub struct BlockDesc { pub kind: &'static str, pub label: &'static str, pub category: Category, pub params: &'static [ParamDesc] }
pub struct ParamValues(BTreeMap<String, Value>);   // Value = Number(f32) | Text(String)
// .number(desc), .choice(desc) (accepts the option name or index), .text(desc), .validate(block)
```

`id`s, block `kind`s and choice option strings are stored in users' files: **never rename them**. Labels are free text shown in the UI.

### 5.2 Blocks: `src/dsp/mod.rs`

```rust
pub trait Block: Send {
    fn process(&mut self, buf: &mut [f32]);
    /// Called FROM THE AUDIO THREAD: no allocations. `index` = position in BlockDesc::params.
    /// Abrupt changes must be smoothed (about 10-20 ms) to avoid clicks.
    fn set_param(&mut self, index: usize, value: f32) {}
    fn reset(&mut self) {}
}

// Each block `<name>` exposes, in its module:
pub const <NAME>_DESC: BlockDesc;
pub fn build_<name>(values: &ParamValues, sr: f32) -> anyhow::Result<Box<dyn Block>>;
```

- Registering a new block = **one line** in `CATALOG` (`src/dsp/mod.rs`).
- `Smoothed` (in `src/dsp/mod.rs`) is the smoothed value for gains and levels: `Smoothed::new(v, 15.0, sr)`, then `.target = …` in `set_param` and `.next()` per sample.
- Reference examples: `drive.rs` and the `Gain` part of `dynamics.rs`.
- A parameter that cannot change live (e.g. the procedural `cab` IR) is handled by `src/live.rs`: it marks the graph for a rebuild, done at most every 100 ms with a crossfade.

### 5.3 Graph (preset v2) and pedal files: `src/graph/model.rs`

```toml
# presets/crunch.toml
name = "crunch"
version = 2

[[node]]
id = "in"
type = "input"
pos = [0, 0]

[[node]]
id = "dist"
type = "drive"
pos = [220, 0]
params = { shape = "asym", gain_db = 12 }

[[node]]
id = "speaker"
type = "pedal:cab-4x12"      # composite pedal: "pedal:" + file name in pedals/
pos = [440, 0]
params = { Brightness = 0.4 }

[[node]]
id = "out"
type = "output"
pos = [660, 0]
params = { volume_db = -8 }

[[edge]]
from = "in"
to = "dist"
# … more edges
```

```toml
# pedals/cab-4x12.toml
name = "4x12 Cab"
category = "amp"

[[node]]                     # "pedal_in" and "pedal_out" are implicit
id = "c"
type = "cab"
params = { high_hz = 4800, seed = 7 }

[[edge]]
from = "pedal_in"
to = "c"

[[edge]]
from = "c"
to = "pedal_out"

[[knob]]                     # exposed knob: 0..1 mapped onto an inner parameter
label = "Brightness"
target = "c.high_hz"
min = 3500
max = 7000
default = 0.4
```

- v1 presets (a linear list of `[[block]]`) are converted on load into `input → … → output`.
- In JSON the keys stay **singular** (`node`, `edge`, `knob`), as in the TOML files.
- In `params`, choices are stored as text (`shape = "asym"`) and files as paths; `set_param` sends a choice as its index.
- A loaded or saved graph is named after its file (`clean.toml` → `"clean"`), so the UI can recognize it.
- Rust types: `src/graph/model.rs`. TypeScript types: `ui/src/protocol.ts`.

### 5.4 WebSocket protocol (`/ws`, JSON messages with a `type` field)

Client → server:

| `type` | Fields | Effect |
|---|---|---|
| `hello` | – | the server replies with `state` |
| `set_param` | `node`, `param`, `value` | live change, no graph rebuild |
| `set_graph` | `graph` | new graph: compiled and swapped in with a crossfade |
| `load_preset` / `save_preset` | `name` (+ `graph` when saving) | replies with `state` |
| `delete_preset` | `name` | deletes `presets/<name>.toml`; the playing graph is unchanged; replies with `state` |
| `save_pedal` / `delete_pedal` | `id` + `pedal` / `id` | writes or deletes `pedals/<id>.toml`, replies with `state` (updated catalog) |

Server → client:

| `type` | Fields |
|---|---|
| `state` | `catalog` (blocks + pedals, with parameters), `graph`, `presets`, `pedals` (content of composite pedals), `audio` (sample rate, buffer, running) |
| `meters` | `in_db`, `out_db`, `dsp` (0..1), `xruns`, about 20 times per second |
| `error` | `message` (shown to the user), `node` (optional: the node the error is about) |

Exact JSON examples live in `ui/src/fixtures/*.json` and are regenerated with `openpedal dev-fixtures ui/src/fixtures`. They are the reference.

### 5.5 Server ↔ engine boundary: `src/server/mod.rs`

```rust
pub trait EngineHandle: Send + Sync + 'static {
    fn catalog(&self) -> Vec<CatalogItem>;
    fn graph(&self) -> GraphFile;
    fn audio(&self) -> AudioInfo;
    fn meters(&self) -> MetersMsg;
    fn set_graph(&self, graph: GraphFile) -> Result<()>;
    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()>;
    fn reload_pedals(&self) -> Result<()>;
}
```

The **server** reads and writes files in `presets/` and `pedals/`; the **engine** compiles and plays. `MockEngine` (`src/server/mock.rs`) is used with `--no-audio` or when no audio interface is available; `LiveEngine` (`src/live.rs`) drives the real engine. Graph errors are `graph::GraphError`, which carries the node to highlight.

## 6. How parallel work is organized

The project has been built by an integrator (Opus) plus parallel agents (Sonnet for design-heavy tasks, Haiku for fully specified ones), each in its own git worktree with **exclusive ownership of a set of files**. Rules for every agent:

1. Work in your worktree and change **only the files you own**. If you need to change a contract (§5), **stop and say so in your report** instead of changing it.
2. Follow `CONTRIBUTING.md` (English everywhere, Conventional Commits, real-time rules).
3. Before finishing: `nix develop -c cargo test` and `nix develop -c cargo clippy --all-targets` with no warnings in your files (for `ui/`: `cd ui && npm install && npm run build`).
4. Never start live audio (`openpedal` without a subcommand, or `openpedal ui` without `--no-audio`): the audio interface belongs to the user.
5. Finish with one commit in your worktree and a short report: what you did, files touched, tests added, doubts or deviations.

## 7. Completed

| ID | Model | Result |
|---|---|---|
| F0 | Opus | contracts, catalog, fixtures, UI skeleton |
| A1 | Sonnet | graph compiler and executor, composite pedals, 20 ms crossfade, v1 conversion |
| B1 | Sonnet | axum + WebSocket server, MockEngine, `openpedal ui` |
| C1 | Sonnet | node editor, live knobs, WebSocket client with mock fallback, *Create pedal* |
| D1 | Haiku | live `set_param` for gate, filter, tremolo, delay, chorus, cab |
| D2 | Haiku | compressor (soft knee) |
| D3 | Haiku | Freeverb reverb |
| D4 | Sonnet | 4x oversampled drive, TMB tone stack (Fender, Marshall, Vox) |
| I2 | Opus | integration: `LiveEngine`, rebuild of non-live parameters, filter glide, `pedals` in `state`, error node |
| – | Opus | undo/redo, deleting pedals/cables/presets, context menu, *Tidy up*, *New*; English translation |

## 8. Next

- **E1 · Windows and macOS audio:** `src/audio/` with an `AudioBackend` trait; `jack.rs` (current code) as the Linux default, `cpal.rs` for WASAPI/CoreAudio (separate input/output streams joined by an `rtrb` queue of 2 buffers; optional `asio` feature). Check builds for `x86_64-pc-windows-gnu` and `aarch64-apple-darwin`.
- **`formula` block:** DSP written as an expression (e.g. `tanh(x * drive) * 0.5`), compiled to a small bytecode when it changes and evaluated without allocations.
- True stereo (wide reverb and chorus), MIDI (footswitches and controllers), state sync between several open clients (PC + phone), a native window with Tauri, packages and CI for the three OSes.

## 9. Current tasks (2026-11-08)

Run in parallel, each in its own worktree; file ownership as listed. General rules: §6 and `CONTRIBUTING.md`.

### P1 · `fuzz` block (Sonnet) — owns `src/dsp/fuzz.rs`

Goal: the gated, sputtery fuzz of a Z.Vex Fuzz Factory (Muse, "Plug In Baby"). The current `drive` with `shape = "fuzz"` is a static curve: it saturates the same way at every level, with no gating. What is missing is **level-dependent behavior**: as a note decays the fuzz breaks up into "velcro" and then cuts off abruptly; louder picking clips harder and asymmetrically.

`FUZZ_DESC` already exists (do not change ids/defaults/ranges): `drive`, `gate`, `comp`, `bias` (0..1), `tone_hz`, `level_db`. Suggested behavioral model (you may improve it, but document what you do):

1. Input coupling high-pass (~80-150 Hz) — a Fuzz-Face-style input that tightens the lows.
2. Pre-gain from `drive` (roughly +20..+50 dB, perceptually spread).
3. **Starvation / gate:** a dead zone before the clipper, `u = sign(x) * max(|x| - d, 0)`, with `d` from `gate` scaled relative to the drive so that it acts on the decaying tail of notes. With high gain after it, decaying notes turn into narrowing pulses (sputter) and then silence. Make `d` follow a fast envelope slightly (bias drift) so the onset of a note "blooms".
4. Asymmetric clipping with `bias` (e.g. `tanh(k*u + b) - tanh(b)` blended with a harder clipper by `comp`: low `comp` = smooth and compressed, high = harder/spikier).
5. Run steps 3-4 at 4x with `oversample::Oversampler4` (see `drive.rs`).
6. DC blocker, low-pass at `tone_hz`, `level_db`.
7. `set_param` live for every parameter, smoothed where needed; no allocation in `process`/`set_param`.

Tests (measure, don't just run): a decaying 220 Hz note gets cut off (output below -60 dBFS) earlier with `gate = 0.8` than with `gate = 0`, while `drive` on the same note keeps a gradual tail; output bounded and finite for 0 dBFS input at max drive; `bias > 0` produces even harmonics (2nd harmonic of a sine above -40 dB relative to the fundamental) and `bias = 0` doesn't; aliasing check like `drive.rs`; `set_param` changes the output.

### P2 · `octave` block (Haiku) — owns `src/dsp/octave.rs`

Analog octaver. `OCTAVE_DESC` already exists: `dry`, `down`, `up` (0..1 mix levels), `tone_hz`, `track_hz`.

- **Tracking signal:** the input through a 2nd-order low-pass at `track_hz` (reuse the RBJ formulas: you may copy the lowpass coefficient code from `filter.rs` into a small private struct).
- **Octave down (Boss OC-style divider):** a flip-flop `ff` (±1) toggles on every *rising* zero crossing of the tracking signal, with hysteresis (a crossing counts only after the signal went below `-h`, then above `+h`, with `h` = 2% of a slow envelope of the tracking signal). Octave-down voice = `ff * tracking_signal` (keeps the note's envelope).
- **Octave up (Octavia-style):** `|tracking_signal|` minus its DC (a one-pole high-pass at ~20 Hz).
- Both voices go through a 2nd-order low-pass at `tone_hz`. Output = `dry*x + down*voice_down + up*voice_up`.
- `set_param` live (smoothed mixes with `Smoothed`, filters recomputed without allocating).

Tests: a 220 Hz sine with `dry=0, down=1, up=0`: the magnitude at 110 Hz (single-bin DFT) is larger than at 220 Hz; with `dry=0, down=0, up=1`: 440 Hz dominates over 220 Hz; `dry=1, down=0, up=0`: output equals input; finite output on noise.

### P3 · iPad core (Sonnet) — owns `Cargo.toml`, `flake.nix`, `src/lib.rs` (new), `src/ffi.rs` (new), `src/audio/**` (new), `ios/**` (new), and the module/`use` lines of `src/main.rs`, `src/engine.rs`, `src/live.rs` that the split requires. Must not change `src/dsp/**`, `src/graph/**`, `src/server/**` beyond `use` paths.

Target architecture on iPad: **everything stays in Rust** (DSP, graph, server, UI assets); a thin Swift app (built later with [xtool](https://github.com/xtool-org/xtool) from Linux) opens the audio session (USB interface), calls into Rust for every audio buffer, and shows the UI in a `WKWebView` pointed at the Rust server on `127.0.0.1`.

1. **Library split:** move everything except the CLI into a library crate (`src/lib.rs`), keep `src/main.rs` as the desktop binary. Add `crate-type = ["rlib", "staticlib"]`.
2. **Audio backend abstraction:** the real-time `Runner` in `engine.rs` is already independent of JACK. Make the JACK client one backend (`src/audio/jack.rs`, Linux only via `cfg`); `jack` becomes a Linux-only dependency. `LiveEngine` must work with any backend.
3. **C ABI for hosts that own the audio callback** (`src/ffi.rs`): e.g. `openpedal_start(sample_rate, max_frames, data_dir, port) -> *mut Handle` (builds the engine with the preset in `data_dir`, starts the UI server on `127.0.0.1:port` in a background thread), `openpedal_process(handle, input, output, frames)` (real-time safe: just runs the `Runner`), `openpedal_stop(handle)`. Generate or hand-write `ios/OpenPedalCore/include/openpedal.h`.
4. **Cross-compilation check:** add the Rust std for `aarch64-apple-ios` to the dev shell (e.g. switch the flake's Rust to `rust-overlay` or `fenix` with that target, keeping the current stable version) and make `cargo build --lib --release --target aarch64-apple-ios` succeed on Linux (a staticlib needs no Apple linker). Desktop builds/tests must keep working and the binary must still embed the UI.
5. **Swift shell draft** in `ios/` (SwiftPM package + `xtool.yml`, not buildable yet without the Apple SDK): `AVAudioSession` (`.playAndRecord`, `.measurement` mode, preferred IO buffer ~2-3 ms, input from the USB interface), `AVAudioEngine` with a source/sink calling `openpedal_process`, a `WKWebView` on `http://127.0.0.1:<port>`, `Info.plist` with the microphone usage description and `UIBackgroundModes: audio`. Write `ios/README.md` with the exact build steps with xtool (Swift 6.4 toolchain, Xcode.xip, usbmuxd, Developer Mode on the iPad) — the integrator will run them with the user.

Tests: everything that passes today still passes; a test for the FFI that calls start/process/stop on a buffer with the mock data dir (no audio device).
