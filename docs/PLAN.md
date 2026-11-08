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

## 9. Status and next task (2026-11-08)

Done since §7: `fuzz` (gated Fuzz Factory style) and `octave` blocks, `presets/plug-in-baby.toml` (tuned
on a real recording of the riff: the fuzz `gate` depends on the input level), the core split into a
library with an `AudioBackend` trait and a C ABI (`src/ffi.rs`), and the iPad app (`ios/`, Swift shell +
WKWebView on the Rust server) built from Linux with xtool and installed on a real iPad — see
`ios/README.md`.

**Next: a touch-first frontend for tablets and phones.** The node editor works on the iPad but is
designed for mouse and keyboard (small handles, right-click menus, vertical-drag knobs, a dense top bar).
Plan the new frontend before coding: what a player needs on stage (preset switching, big knobs,
bypass, tuner?) versus building pedals (keep the node editor, maybe tablet-only). It reuses the same
WebSocket protocol (§5.4), so the backend should not need changes beyond small additions.

Open issues: the server has no shutdown hook (`openpedal_stop` keeps it alive until the app exits);
changes from one client are not broadcast to the others; mono signal path; Windows/macOS audio backend
(E1, the `AudioBackend` trait exists); whammy/pitch shifter; `formula` block; stereo.

UI regression tests: `tools/e2e/run.sh` (inside `nix develop`, after `cargo build --release`).
