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
  - a **composite pedal** is a sub-graph of basic blocks with some knobs "exposed" (UI: select nodes → *Create pedal*; files: `content/pedals/*.toml`);
  - a **basic block** is Rust code in `crates/dsp/src/`. A future `formula` block will let users write DSP as math expressions from the UI (phase 3);
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
 │ crates/server    axum: HTTP (assets) + WS    │
 │                  MockEngine, uses protocol + │
 │                  library (files on disk)     │
 ├──────────────────────────────────────────────┤
 │ crates/engine    EngineHandle trait, live.rs │
 │                  (LiveEngine), engine.rs,    │
 │                  audio/ (JACK; cpal: E1)     │
 ├──────────────────────────────────────────────┤
 │ crates/graph     model, loading, compilation │
 │                  (composite pedals → blocks, │
 │                  topological order), executor│
 ├──────────────────────────────────────────────┤
 │ crates/dsp       basic blocks + catalog      │
 │                  (parameter descriptors), wav│
 └──────────────────────────────────────────────┘
```

**Real-time rule, always:** in `process`, in graph execution and in `set_param`, no allocations, locks, I/O or logging.
Changes reach the audio thread through lock-free queues (`rtrb`); old structures are freed outside the audio thread.

## 4. Concepts

| Term | Meaning |
|---|---|
| **Basic block** | Rust DSP: `gain`, `drive`, `filter`, `cab`, `compressor`… Described by a `BlockDesc` |
| **Composite pedal** | sub-graph of basic blocks and/or other pedals, with exposed `knob`s. File `content/pedals/<id>.toml` |
| **Graph** (preset) | nodes + cables + screen positions. File `content/presets/<name>.toml` (format v2) |
| `input` ("Guitar") | special node: the guitar. Exactly one, no inputs |
| `output` ("Amp") | special node: the output, with `volume_db`. Exactly one. Without a path to it there is no sound |
| Fan-out / fan-in | many cables may leave one output (the signal is copied); cables entering the same node are **summed** |
| Cycles | **forbidden** (the compiler rejects them). Feedback lives inside blocks, e.g. the delay |

The graph signal is **mono**; true stereo is planned for phase 3.

## 5. Contracts

### 5.1 Parameters and catalog: `crates/dsp/src/params.rs`

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

### 5.2 Blocks: `crates/dsp/src/lib.rs`

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

- Registering a new block = **one line** in `CATALOG` (`crates/dsp/src/lib.rs`).
- `Smoothed` (in `crates/dsp/src/lib.rs`) is the smoothed value for gains and levels: `Smoothed::new(v, 15.0, sr)`, then `.target = …` in `set_param` and `.next()` per sample.
- Reference examples: `drive.rs` and the `Gain` part of `dynamics.rs`.
- A parameter that cannot change live (e.g. the procedural `cab` IR) is handled by `crates/engine/src/live.rs`: it marks the graph for a rebuild, done at most every 100 ms with a crossfade.

### 5.3 Graph (preset v2) and pedal files: `crates/graph/src/model.rs`

```toml
# content/presets/crunch.toml
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
type = "pedal:cab-4x12"      # composite pedal: "pedal:" + file name in content/pedals/
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
# content/pedals/cab-4x12.toml
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
- Rust types: `crates/graph/src/model.rs`. TypeScript types: `apps/desktop/ui/src/protocol.ts`.

### 5.4 WebSocket protocol (`/ws`, JSON messages with a `type` field)

Client → server:

| `type` | Fields | Effect |
|---|---|---|
| `hello` | – | the server replies with `state` |
| `set_param` | `node`, `param`, `value` | live change, no graph rebuild |
| `set_bypass` | `node`, `bypass` | live, click-free (~10 ms crossfade) bypass of a node, no graph rebuild; `error` for `input`/`output`; also stored in the graph (`bypass` on the node) |
| `set_graph` | `graph` | new graph: compiled and swapped in with a crossfade |
| `load_preset` / `save_preset` | `name` (+ `graph` when saving) | replies with `state` |
| `delete_preset` | `name` | deletes `content/presets/<name>.toml`; the playing graph is unchanged; replies with `state` |
| `save_pedal` / `delete_pedal` | `id` + `pedal` / `id` | writes or deletes `content/pedals/<id>.toml`, replies with `state` (updated catalog) |
| `add_node` | `kind`, `after?` | inserts a pedal into a linear chain after node `after` (default: before the output); the server picks the id; replies `state` then `added`; `error` on non-linear graphs |
| `remove_node` | `node` | removes a pedal from a linear chain and reconnects its neighbours |
| `move_node` | `node`, `to` | moves a pedal to index `to` among the pedals (0 = after the input, clamped); input stays first, output last |
| `undo` / `redo` | – | one step through the shared server-side history (cap 100); replies `state`; `error` if there is nothing to undo/redo |
| `edit_begin` | – | start of a continuous gesture (knob drag): the following `set_param`s become one undo step (the step is recorded at the first one) |
| `analyze_pedal` / `analyze_node` | `pedal` + `values` / `node` | frequency response of a pedal file (draft allowed) or of a node of the playing graph; reply `response` to the asker only (protocol v3, §12.2) |

Server → client:

| `type` | Fields |
|---|---|
| `state` | `protocol` (`PROTOCOL_VERSION`, currently 3), `catalog` (blocks + pedals, with parameters), `graph`, `presets`, `pedals` (content of composite pedals), `audio` (sample rate, buffer, running), `can_undo`, `can_redo`, `dirty` (graph differs from the last load/save) |
| `param` | `node`, `param`, `value`: a `set_param` made by another client (never echoed to its sender) |
| `response` | `freq`, `db`: reply to `analyze_pedal` / `analyze_node` (about 128 log-spaced points, 20 Hz-20 kHz), sent only to the client that asked |
| `added` | `node`: the id created by `add_node`, sent only to the client that asked, right after its `state` |
| `meters` | `in_db`, `out_db`, `dsp` (0..1), `xruns`, about 20 times per second |
| `error` | `message` (shown to the user), `node` (optional: the node the error is about) |

Broadcast: every change to the graph, presets or pedals is sent as `state` to all connected clients (the sender gets it as its reply, except `set_graph`, whose sender already has the graph). `set_param` is forwarded to the other clients as `param`; when it also changes `can_undo` or `dirty` (first change of a gesture, a choice) a `state` goes out instead. `meters` go to every client. History: one undo/redo stack of graph snapshots (cap 100) shared by all clients; pushed by `add_node`, `remove_node`, `move_node`, `set_bypass`, `set_graph` (if the graph changed), choice parameters and `edit_begin` gestures; plain number `set_param`s alone are not steps; `load_preset` clears it. `save_preset` renames the playing graph.

Exact JSON examples live in `apps/desktop/ui/src/fixtures/*.json` and are regenerated with `openpedal dev-fixtures apps/desktop/ui/src/fixtures`. They are the reference.

### 5.5 Server ↔ engine boundary: `crates/engine/src/lib.rs`

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

The **server** reads and writes files in `content/presets/` and `content/pedals/`; the **engine** compiles and plays. `MockEngine` (`crates/server/src/mock.rs`) is used with `--no-audio` or when no audio interface is available; `LiveEngine` (`crates/engine/src/live.rs`) drives the real engine. Graph errors are `graph::GraphError`, which carries the node to highlight.

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
| – | Opus | undo/redo, deleting content/pedals/cables/presets, context menu, *Tidy up*, *New*; English translation |

## 8. Next

- **E1 · Windows and macOS audio:** `crates/engine/src/audio/` with an `AudioBackend` trait; `jack.rs` (current code) as the Linux default, `cpal.rs` for WASAPI/CoreAudio (separate input/output streams joined by an `rtrb` queue of 2 buffers; optional `asio` feature). Check builds for `x86_64-pc-windows-gnu` and `aarch64-apple-darwin`.
- **`formula` block:** DSP written as an expression (e.g. `tanh(x * drive) * 0.5`), compiled to a small bytecode when it changes and evaluated without allocations.
- True stereo (wide reverb and chorus), MIDI (footswitches and controllers), state sync between several open clients (PC + phone), a native window with Tauri, packages and CI for the three OSes.

## 9. Status and next task (2026-11-08)

Done since §7: `fuzz` (gated Fuzz Factory style) and `octave` blocks, `content/presets/plug-in-baby.toml` (tuned
on a real recording of the riff: the fuzz `gate` depends on the input level), the core split into a
library with an `AudioBackend` trait and a C ABI (`crates/ffi/src/lib.rs`), and the iPad app (`ios/`, Swift shell +
WKWebView on the Rust server) built from Linux with xtool and installed on a real iPad — see
`apps/ios/README.md`.

**Next: a touch-first frontend for tablets and phones.** The node editor works on the iPad but is
designed for mouse and keyboard (small handles, right-click menus, vertical-drag knobs, a dense top bar).
Plan the new frontend before coding: what a player needs on stage (preset switching, big knobs,
bypass, tuner?) versus building pedals (keep the node editor, maybe tablet-only). It reuses the same
WebSocket protocol (§5.4), so the backend should not need changes beyond small additions.

Open issues: the server has no shutdown hook (`openpedal_stop` keeps it alive until the app exits);
changes from one client are not broadcast to the others; mono signal path; Windows/macOS audio backend
(E1, the `AudioBackend` trait exists); whammy/pitch shifter; `formula` block; stereo.

UI regression tests: `tools/e2e/run.sh` (inside `nix develop`, after `cargo build --release`).

## 10. Native iPad app and separate releases (planned 2026-11-08)

Decisions (by the user): the iPad frontend is **fully native SwiftUI** with the system Liquid Glass
material (iPadOS 26, `glassEffect`; the xtool SDK is iPhoneOS 27.2 and has it). **iPad first**, iPhone
later. The **node editor stays desktop-only**: the iPad app has no WebView. **Separate releases**
for desktop and iOS. iOS distribution stays **sideload with xtool** for now (7-day free signing).
Reference design: the Liquid Glass pedalboard mockup (concept A v2): full-bleed horizontal pedalboard,
floating glass toolbar (preset title menu, undo, meters, tuner, Save), floating glass inspector with
big knobs for the selected pedal, bottom sheet to add a pedal.

### 10.1 Architecture

```
 ┌──────────── iPad app (apps/ios/) ─────────────────────┐
 │ SwiftUI views: Pedalboard, Inspector, Toolbar     │
 │ OpenPedalKit: Codable protocol + WebSocket client │──┐ ws://127.0.0.1:<port>/ws
 │ AudioHost: AVAudioSession + C ABI (crates/ffi/src/lib.rs)    │  │ (later: a desktop on the LAN)
 └──────────────┬────────────────────────────────────┘  │
                │ openpedal_start / process             │
 ┌──────────────▼──── Rust core (libopenpedal.a) ───────▼┐
 │ server (same protocol §5.4) · graph · dsp             │
 └───────────────────────────────────────────────────────┘
```

The native app talks to the in-process Rust server over the **same WebSocket protocol** as the web UI.
This keeps one contract and later lets the iPad remote-control a desktop instance.

**Chain view.** The iPad shows the graph as a chain: nodes in signal order from `input` to `output`.
If the graph is not a simple chain (parallel branches, composite pedals are fine), the board shows the
nodes in topological order and disables reordering/adding, with a note "Parallel routing: edit on the
desktop". Adding/removing/reordering on a chain is done by the core (`add_node`, `remove_node`, `move_node`, protocol v2).

### 10.2 Contract changes (protocol v1)

- **`protocol` field** in `state`: `u32`, constant `PROTOCOL_VERSION = 1` in `crates/protocol/src/lib.rs`.
  Clients refuse to run (clear error) when the server's major contract is newer than they know.
  Bump it on any breaking change to §5.4; additive optional fields do not bump it.
- **Bypass.** `NodeSpec` gains `bypass: bool` (TOML `bypass = true`, omitted when false; default false).
  A bypassed node passes its input through unchanged (sum of its inputs, as for any node). New
  client message `{ "type": "set_bypass", "node": "<id>", "bypass": true }`: live, no graph rebuild,
  with a short click-free crossfade (~10 ms) between processed and dry. `input`/`output` cannot be
  bypassed (error). Saved with the preset. The web UI shows it as a toggle on the node.
- Tuner (pitch in `meters`) is **not** in this round.

### 10.2b Protocol v2 (2026-11)

`PROTOCOL_VERSION = 2`: chain edits (`add_node`, `remove_node`, `move_node`), `undo`/`redo`/`edit_begin`, the server-to-client messages `param` and `added`, and `can_undo`/`can_redo`/`dirty` in `state` (see §5.4). Breaking for v1 decoders, which do not know the new server messages. Clients are thin: they send commands and render state; the core owns the chain logic (`crates/graph/src/chain.rs`) and the history, and broadcasts to every client (desktop and iPad together).

### 10.3 Versions and releases

| Product | Version source | Tag | Changelog |
|---|---|---|---|
| Desktop (binary + web UI) | `Cargo.toml` `version` | `desktop-vX.Y.Z` | `CHANGELOG.md` |
| iPad app | `apps/ios/Info.plist` `CFBundleShortVersionString` (+ `CFBundleVersion` build number) | `ios-vX.Y.Z` | `apps/ios/CHANGELOG.md` |
| Protocol | `PROTOCOL_VERSION` | – | noted in both changelogs |

- Changelogs are generated with **git-cliff** from Conventional Commits. Scope routing: `ios` → iOS
  only; `ui` → desktop only; `dsp`, `graph`, `engine`, `server`, `ffi` → both (shared core); `docs`,
  `nix`, `chore` → neither by default.
- `tools/release.sh <desktop|ios> <X.Y.Z>`: requires a clean tree on `master`, runs the checks
  (desktop: cargo test, clippy, `npm run build`; ios: cargo test + `build-core.sh` + `xtool dev build`),
  bumps the version, regenerates that changelog, commits `chore(release): desktop vX.Y.Z` and creates
  the annotated tag. It never pushes; the user pushes.
- The iOS app shows its version, the core commit and the protocol version in an About sheet.
- First releases: `desktop-v0.2.0` (current master + bypass) and `ios-v0.1.0` (first native app).

### 10.4 Work split

| ID | Model | Owns | Task |
|---|---|---|---|
| N1 | Sonnet | `src/**`, `apps/desktop/ui/src/**`, `content/presets/` fixtures | protocol v1: `protocol` field, bypass (model, compiler/executor, `set_bypass`, crossfade), fixtures, tests, web UI toggle |
| N2 | Sonnet | `apps/ios/Sources/**`, `apps/ios/Package.swift`, `apps/ios/Info.plist` | native SwiftUI app per §10.1 against contract §10.2 (codes `bypass`/`protocol` before N1 lands) |
| N3 | Haiku | `tools/release.sh`, `cliff.toml`, `CHANGELOG.md`, `apps/ios/CHANGELOG.md`, `docs/RELEASING.md` | release tooling per §10.3 |
| – | Opus | integration | merge, iPad build and install, first tags |

## 11. Restructuring (decided 2026-11-08)

Architecture map and findings: artifact "openPedal Architecture Map". Decisions by the user:

1. **Cargo workspace.** Split the single crate into `crates/{dsp,graph,engine,protocol,library,server,ffi}`;
   desktop binary + web UI move to `apps/desktop`, the iPad app to `apps/ios`; presets/pedals/IRs to `content/`.
   Dependencies go one way: dsp ← graph ← engine; protocol ← server; library ← server; ffi → server + engine.
2. **One protocol source.** The Rust types in `crates/protocol` are the source; TypeScript and Swift types are
   generated (schemars JSON Schema + an in-repo generator, `cargo run -p openpedal-protocol --bin gen-protocol`; typeshare cannot express the internally tagged enums and flatten used on the wire). The JSON fixtures stay as contract tests.
3. **File compatibility.** Saved presets and pedals stay readable forever; new formats get an automatic migration
   (as v1 → v2 today). Files are always written in the latest format.
4. **Branches.** `master` always green; short-lived `feat/*` / `fix/*` branches deleted after merge; tags
   `desktop-v*` / `ios-v*`; CI must pass before merging.
5. **Thin clients.** Chain edits (move, add, remove a pedal) and undo/redo move into the core as protocol
   messages; desktop and iPad only send commands and render state. The server broadcasts state to every client.

Phase 2 done: layout. The workspace is `crates/{dsp,graph,protocol,library,engine,server,ffi}`,
`apps/{desktop,ios}` and `content/{presets,pedals,irs}`. The `EngineHandle` trait now lives in `crates/engine`
(re-exported by `crates/server`), so the server depends on the engine's interface and never the reverse.
Dependencies: graph → dsp; protocol → graph; library → graph; engine → dsp, graph, protocol;
server → engine, protocol, library, graph; ffi → engine, server, graph; desktop → all but ffi.
The static library for the iPad is `openpedal-ffi`, with `[lib] name = "openpedal"` so it is still `libopenpedal.a`.
Phase 3 wave 1 done: Effect/Control/Route + registry in `crates/graph` (v1 linear chain removed), `crates/library` owns files and factory seeding (C ABI `openpedal_start2`, `openpedal_version`).

Phases (each ends with green tests and unchanged audio renders):

| Phase | Work | Who |
|---|---|---|
| 1 | Safety net: golden renders of every preset, CI (tests, clippy, UI build) | 1 Sonnet |
| 2 | Move files into the workspace, no behaviour change | 1 agent, sequential |
| 3 | Effect/Control/Route in graph · protocol codegen · library crate + broadcast · core chain edits + undo · web UI split · iOS on generated types · legacy cleanup | Sonnet swarm + Haiku, Opus integrates |
| 4 | Agnostic (custom) pedals on top of Effect/Route | to be designed |

Phase 3 wave 2 done (a8b4d49): protocol v2 (chain edits, undo/redo, edit_begin, broadcast to every client, `param`/`added` messages); web UI split into hooks (a174f0c). Phase 3 done (fb8f406): the desktop editor uses the core undo/redo and dirty flag; local history only inside composite-pedal editing. Next: phase 4 (custom pedals), mockup "openPedal Pedal Designer".

## 12. Phase 4: custom pedals (planned 2026-11-09)

Approved design: mockup "openPedal Pedal Designer". A custom pedal = an inner graph of existing blocks
(linear on iPad, any shape on desktop) + **controls** chosen by the user, each driving one or more inner
parameters through a map. Open-ended by design: control kinds, map kinds and effect sources are tagged
enums, so envelope/LFO controls, drawn curves and a `formula` source can be added later without changing
the structure. Created and edited on both desktop and iPad.

### 12.1 Pedal file v2 (`content/pedals/<id>.toml`)

```toml
version = 2
name = "Grit Box"
category = "drive"
description = "..."
color = "#d9573b"          # optional enamel colour

[[node]] / [[edge]]        # as today, with the implicit pedal_in / pedal_out

[[control]]
id = "grit"
label = "Grit"
kind = "knob"              # "knob" | "switch" | "selector"
default = 0.55             # knob 0..1 · switch true/false · selector option index
[[control.route]]
target = "drive.gain_db"
map = "linear"             # knob: "linear" | "log" | "inverse"
min = 6
max = 40

[[control]]
id = "voice"
label = "Voice"
kind = "selector"
options = ["Smooth", "Crunch", "Fuzz"]
default = 1
[[control.route]]
target = "drive.shape"
values = ["soft", "asym", "hard"]   # one per option (numbers or choice names)

[[control]]
id = "boost"
label = "Boost"
kind = "switch"
default = false
[[control.route]]
target = "drive.level_db"
off = 0
on = 6
mode = "add"               # "set" (default) or "add" (offset on top of the other routes / the block value)
```

v1 pedal files (`[[knob]]` with `label/target/min/max/default`) are read as v2 knobs with one linear route
and written back as v2 (rule §11.3).

### 12.2 Protocol v3

- Catalog: a custom pedal's controls are exposed as today's `ParamDesc`: knob → number 0..1, switch →
  choice `["off","on"]`, selector → choice with its options. `CatalogItem` gains optional `color`.
- `state.pedals` and `save_pedal` carry the v2 pedal file (with `control`, `color`, `version`).
- New client message `analyze_pedal { pedal, values: { <control id>: number } }` (works for an unsaved draft)
  and `analyze_node { node }` (a node of the playing graph with its current values) → server replies
  `response { freq: [f32], db: [f32] }`: magnitude response measured offline (never on the audio thread) by
  rendering a low-level test signal through a compiled copy of the effect, FFT, ~128 log-spaced points
  20 Hz-20 kHz.
- `PROTOCOL_VERSION = 3`.

### 12.2b Implemented in wave A (differences and rules)

- Files omit defaults: a knob route with the linear map has no `map` key, a `set` route has no `mode` key, a control without `default` takes 0.5 (knob), off (switch) or the first option (selector). `label` defaults to `id`. A switch `default` may also be 0/1. `[[knob]]` and `[[control]]` cannot be mixed in one file; `version` above 2 is refused. A v1 knob's control `id` is its label (so saved graphs keep working).
- `inverse` is a linear map run backwards (0 gives `max`, 1 gives `min`); `log` is `min * (max/min)^n` and needs `min`, `max` > 0. Switch/selector route values are numbers, or option names when the target is a choice parameter.
- **Combination rule** (`crates/graph/src/compile.rs`, `apply_routes`; identical at build time and for live changes): controls are applied in file order; a `set` route replaces the parameter (the last control, then the last route, wins); then every `add` route adds its value, in file order: `final = base + offset1 + offset2 ...`, where `base` is the value written by the `set` routes or, if none, the inner node's own value in the pedal file, or the parameter default; the sum is clamped to the parameter range. `add` is for numeric block parameters only (not for a control of a nested pedal).
- `NodeMap::resolve_changes` returns one change per driven parameter and recomputes each from the current values of all controls of that pedal (kept inside the `NodeMap`).
- Pedals are validated on save (`Library::save_pedal`, `graph::validate_pedal`) and when compiled; `analyze_pedal` also reports them.
- Analysis (`crates/graph/src/analysis.rs`): a standalone `input -> effect -> output` plan at the engine rate, a -30 dBFS impulse (about 0.68 s), FFT, power averaged over a sixth of an octave, 128 log points. About 4 ms for the example pedal. It is the small-signal response at that level (drives are not driven into saturation by the test signal).
- TOML numbers are written with their shortest decimal text (`0.4`, not `0.4000000059604645`).

### 12.3 Work split

| Wave | Work | Owner files |
|---|---|---|
| A | Core: file v2 + v1 reading, Map::{Log, Inverse}, switch/selector routes (set/add), registry/compile/resolve, catalog projection + colour, analysis, protocol v3, fixtures, generated types | crates/**, generated, fixtures, apps/desktop/src (dev-fixtures) |
| B1 | Desktop pedal designer (replaces PedalDialog), per the mockup | apps/desktop/ui/src/** |
| B2 | iPad: "Merge into a pedal" (multi-select on the board), designer sheet, edit custom pedal, enamel colour | apps/ios/Sources/** |

Phase 4 done (da919a7): core (pedal file v2, routes, analysis, protocol v3), desktop pedal designer
(`components/PedalDesigner.tsx`, e2e `tools/e2e/designer.mjs`), iPad "Merge into a pedal" + designer
(`apps/ios/Sources/OpenPedalApp/Designer/`). Next ideas: formula source, envelope/LFO controls, drawn curves.

## 13. Two parallel tracks (planned 2026-11-24)

Two branches off master, each in its own worktree, with no shared files.

| Branch | Goal | Owner files |
|---|---|---|
| `feat/ios-speaker-output` | iPad: guitar into the Scarlett (USB-C), sound out of the iPad's own speakers instead of the interface's headphone jack | `apps/ios/**` |
| `feat/presets-famous-tones` | Presets that recreate famous guitar tones, starting with Muse "Cryogen" (*The Wow! Signal*, 2026), as accurate as possible and one preset per distinct guitar part | `content/presets/**`, `docs/tones/**` |

**iOS output.** iOS normally routes both input and output to the USB interface when one is
connected. The work: an output setting in the app (Interface / iPad speaker), using
`AVAudioSession` (`overrideOutputAudioPort(.speaker)` with `.defaultToSpeaker`, then re-select the
USB input; fall back to `.multiRoute` if the override also steals the input), a visible readout of
the current input/output route, and handling of route changes (plug/unplug). It can only be verified
on the iPad: the user installs and tests.

*Outcome (2026-11-24): dropped.* Tested on the iPad with a routing test of three strategies: iOS
treats the Scarlett as a wired headset and keeps input and output on the same device (forcing the
speaker moves the input to the built-in mic; `.multiRoute` does not list the speaker). Bluetooth
output works but its latency is unusable for playing. The feature was removed; what stayed is the
hot-plug fix (fresh engine with the Scarlett's 2 input channels, no reconfigure loop).

**Famous tones.** Research first (interviews, rig rundowns, gear wikis, live footage), write the findings
and sources to `docs/tones/<song>.md` (guitars, pedals, amp, which part plays what), then build the
preset(s) and check them with `openpedal render` on the user's recordings. Missing DSP blocks (e.g. a
whammy/pitch shifter) are reported, not added on this branch. Exact matching needs a reference clip of
the song in `recordings/`.

## 14. Tone Lab: presets from a song (planned 2026-11-24)

Goal: give openPedal a song (file, voice memo or, for personal use, a YouTube link), let the user pick
the guitar part, and get a preset that sounds like it. Proven by hand on Muse "Cryogen" (§13,
`docs/tones/cryogen.md`): Demucs separation, analysis, synthetic DI, parameter fit; the user judged
the result "perfect". This section turns those throwaway scripts into a tool.

### 14.1 Flow

1. **Source** → a job folder. Kinds: `file` (any audio ffmpeg reads: mp3, m4a voice memo, wav) and
   `youtube` (optional, see 14.3).
2. **Separate** (Demucs `htdemucs_6s`, GPU) → stems. Skipped when the source is already guitar only
   (`separate = "auto"` checks the stems' energy, or the user forces it).
3. **Analyse** → sections of the song with guitar, for each: centre / double-tracked / wide, level,
   brightness, tempo and key, low-note check (octave-down hint).
4. **Pick a part** (user): a section + channel (mid, left or right). This is how several guitars are told
   apart: by time and stereo side. No automatic "how many guitars" (not reliable today).
5. **DI** (the clean input the fit plays through the presets): the user's recording of the part
   (`openpedal --record`, best: fits the user's own guitar) or a synthetic DI from the transcribed
   notes, spectrally matched to a real DI of the user.
6. **Fit** (Rust, in-process renders): try each chain template, fit its knobs to the part's spectrum
   and dynamics, keep the best, validate on held-out audio, write a normal preset + a report.
7. **Listen**: A/B the separated part and the preset's render; save to the library.

### 14.2 Components and contracts

```
tools/tonelab/            Python (uv project): source, separate, analyse, transcribe, synth DI
  tonelab.toml            config (committed defaults), tonelab.local.toml overrides (gitignored)
  tonelab/cli.py          `tonelab <command>`; every command reads/writes a job folder
crates/tonematch/         Rust: features, optimiser, templates, fit, report (no Python)
content/tonematch/templates/   chain templates: <name>.toml (a preset) + <name>.fit.toml (ranges)
apps/desktop/src/main.rs  `openpedal tone-match <job> --part <id> [--di file]`
crates/server, protocol   wave 2: run jobs from the UI, progress events
apps/desktop/ui/src/components/tonelab/   wave 2: Tone Lab panel
```

**Job folder** (the contract between Python and Rust; default root
`~/.local/share/openpedal/tonelab/jobs/<id>/`, `<id>` = date + slug of the title):

| File | Writer | Content |
|---|---|---|
| `job.json` | fetch | `{ "id", "source": { "kind": "file"\|"youtube", "ref", "title" }, "created", "sample_rate", "duration_s" }` |
| `mix.wav` | fetch | the song, float32, original rate, stereo |
| `stems/{guitar,bass,drums,vocals,piano,other}.wav` | separate | Demucs output (or `guitar.wav` = mix when separation is skipped) |
| `analysis.json` | analyse | `{ "tempo_bpm", "key", "sections": [ { "start", "end", "guitar_db", "corr", "side_mid_db", "centroid_hz", "kind": "centre"\|"double"\|"wide"\|"none", "bass_db" } ], "low_notes": [ { "start", "end", "hz" } ] }` (4 s windows merged into sections) |
| `parts.json` | parts | `[ { "id", "label", "start", "end", "channel": "mid"\|"left"\|"right", "notes": [ { "t", "dur", "midi", "amp" } ] } ]` |
| `parts/<id>/target.wav` | parts | the separated part, mono, 48 kHz |
| `parts/<id>/di-synth.wav` | synth-di | synthetic DI, mono 48 kHz, RMS -29.8 dBFS (or matched to `--match-di`) |
| `parts/<id>/di-user.wav` | user / UI | the user's recording of the part |
| `parts/<id>/fit/` | tone-match | `preset.toml`, `report.json`, `render.wav` |

`report.json`: `{ "template", "params": {...}, "error_db", "holdout_error_db", "crest_db": { "target", "render" }, "bands_hz": [...], "diff_db": [...], "renders", "seconds" }`.

**Template** (`content/tonematch/templates/<name>.toml` is a regular preset; `<name>.fit.toml`):
```toml
[[param]]
node = "fuzz"     # node id in the template preset
key = "drive"
min = 0.3
max = 1.0
log = false       # true: searched on a log scale (frequencies)
```
Templates (wave 1): `clean`, `crunch`, `overdrive`, `fuzz`, `fuzz-octave` (octave down before the
fuzz), `highgain`. Each ends in tonestack → amp drive → cab → reverb (reverb mix is fitted: the mix's
ambience is in the target).

**Fit** (as validated on Cryogen): features = 1/3-octave long-term spectrum 40 Hz-10 kHz normalised to
its max, plus crest factor; cost = rms of the band differences + 0.3 x |crest difference|.
Optimiser: Nelder-Mead in a normalised [0,1] space with clamping and 2 restarts. Speed: fit on the
densest 8 s of the part; screen every template with ~60 renders, fully fit the best 2 (~350). Renders
run in-process (no subprocess, no WAV round trip). CPU: at most 4 threads, low priority (DSP renders
cannot run on the GPU; the GPU does separation). Validation: with a user DI, fit on the first 70 %
and report the error on the last 30 %; with a synthetic DI, also report the error on any real DI given
with `--validate-di`. Lesson from §13: a fit that only ever saw the synthetic DI can look perfect and
generalise badly; always report a held-out number.

### 14.3 YouTube (personal use)

`tonelab.toml`:
```toml
[sources]
youtube = true    # personal use of whoever clones the repo; false hides the link field and never imports yt-dlp
[gpu]
device = "cuda"   # "cpu" must be chosen explicitly: no silent fallback
[paths]
jobs = "~/.local/share/openpedal/tonelab/jobs"
```
`yt-dlp` is an optional dependency group (`uv sync --extra youtube`); the code lives only in
`tonelab/sources/youtube.py`, imported lazily when enabled. Removing it = `youtube = false`, or delete
that file and the extra. The README states that downloading from YouTube may break its Terms of
Service and is meant for the user's own use. The server reports `youtube` in its capabilities, so the
UI shows the link field only when enabled.

### 14.4 Waves, branches, owners

Worktrees under `~/Projects/openPedal-wt/<name>`, Conventional Commits, no shared files inside a wave.
Opus: plan, review, merge, end-to-end check on Cryogen. Sonnet: design-heavy tasks. Haiku: fully specified tasks.

| Wave | Branch | Model | Work | Owns |
|---|---|---|---|---|
| 1 | `feat/tonelab-py` | Sonnet | uv project (Python 3.12, torch 2.7.1+cu126 for the GTX 1060, demucs, librosa, soundfile, scipy; ffmpeg from nixpkgs), wrapper `tools/tonelab/tonelab` (sets `LD_LIBRARY_PATH` for NixOS), config loading, sources `file` + `youtube`, `fetch`, `separate`, `analyse`, CLI with `parts`/`synth-di` delegating to `transcribe.py`/`synth.py` | `tools/tonelab/**` except `tonelab/transcribe.py`, `tonelab/synth.py`, `tests/test_transcribe*.py`, `tests/test_synth*.py`; `.gitignore` |
| 1 | `feat/tonelab-transcribe` | Sonnet | `transcribe.py`: `transcribe(target: np.ndarray, sr: int) -> list[Note]` (onsets + pYIN, notes below E2 moved up an octave and flagged); `synth.py`: `synth_di(notes, sr=48000, match_di: np.ndarray \| None) -> np.ndarray` (Karplus-Strong, pickup filter, FIR spectral match to the real DI, RMS -29.8 dBFS) | `tools/tonelab/tonelab/{transcribe,synth}.py`, their tests |
| 1 | `feat/tonematch-core` | Sonnet | crate `tonematch`: features, cost, Nelder-Mead, template loading, in-process render via `openpedal-graph`, screening + full fit, hold-out, report; CLI `openpedal tone-match`; test: render a known preset on a synthetic DI, fit from its template, recover it within 2 dB | `crates/tonematch/**`, `Cargo.toml` (workspace member), `apps/desktop/src/main.rs` (subcommand only) |
| 1 | `feat/tonematch-templates` | Haiku | the 6 templates + `.fit.toml` ranges (from the block param ranges in `crates/dsp/src/*.rs`), each must pass `openpedal check` | `content/tonematch/**` |
| 2 | `feat/tonelab-server` | Sonnet | protocol v4: `tonelab_caps`, `tonelab_start { source }`, `tonelab_pick { job, part }`, `tonelab_fit { job, part, di }`, progress/result events, job list; server runs `tonelab` as a subprocess and the fit in a thread; regenerate types + fixtures | `crates/protocol/**`, `crates/server/**`, generated, fixtures |
| 2 | `feat/tonelab-ui` | Sonnet | Tone Lab panel: source (file drop, link if enabled), section timeline coloured by kind, play the separated part, pick mid/left/right, record or synthesise the DI, run the fit with progress, A/B player, save preset; e2e test on the mock | `apps/desktop/ui/src/components/tonelab/**`, one entry point in the top bar |
| 2 | `docs/tonelab` | Haiku | `tools/tonelab/README.md` (install, GPU, config, YouTube note), user guide in `docs/tonelab.md` | those files |
| 3 | backlog | | detectors (delay, reverb, modulation), phaser block, true stereo for double-tracked parts, sending a fitted preset to the iPad over `ui --lan`, iPad Tone Lab client | |

Wave 1 acceptance (Opus): end to end on `recordings/Cryogen.mp3`, riff part 8-28 s mid, synthetic DI
matched to `recordings/plug-in-baby.wav`, validated on it: held-out error at most 6.5 dB (the hand-made
fit reached 5.9 dB), and the chosen template is a fuzz one.

Wave 1 done (2026-11-24, on `dev`): end to end on Cryogen (riff 8-28 s, synthetic DI matched to a
real DI) the fit picks `fuzz-octave` (fuzz within 0.03 dB), error 1.89 dB, validation on the real DI
5.33 dB (hand-made fit 5.9), 76 s. Known gap: the low-note hint (`low_notes`) is now strict and empty
on Cryogen: pYIN cannot tell the real 62 Hz octave from subharmonic locking in a separated stem. The
fit does not depend on it (all templates are screened); a better detector is backlog.

### 14.5 Wave 2 contract: protocol v4 and HTTP

All messages are serde `tag = "type"`, snake_case, like the existing ones. `PROTOCOL_VERSION` 3 → 4.

Client → server:

| Message | Fields | Answer |
|---|---|---|
| `tonelab_caps` | | `tonelab_caps` |
| `tonelab_jobs` | | `tonelab_jobs` |
| `tonelab_start` | `source: String` (a URL, or a path returned by `POST /tonelab/upload`) | `tonelab_progress`..., then `tonelab_job` |
| `tonelab_open` | `job: String` | `tonelab_job` |
| `tonelab_pick` | `job, start: f32, end: f32, channel: "mid"\|"left"\|"right", label: String` | `tonelab_progress`..., `tonelab_job` |
| `tonelab_record_di` | `job, part, seconds: f32` | records the live input into `parts/<part>/di-user.wav`, then `tonelab_job` |
| `tonelab_fit` | `job, part` | `tonelab_progress`..., `tonelab_fit_result` |
| `tonelab_save_preset` | `job, part, name` | saves the fitted preset to the library like `save_preset` (state broadcast) |
| `tonelab_cancel` | `job` | stops the running step; `tonelab_progress` with `stage: "cancelled"` |

Server → client:

| Message | Fields |
|---|---|
| `tonelab_caps` | `available: bool, youtube: bool, device: String, reason: Option<String>` (available = the `tonelab` tool was found and runs) |
| `tonelab_jobs` | `jobs: Vec<{ id, title, created, duration_s }>` |
| `tonelab_progress` | `job, stage: "fetch"\|"separate"\|"analyse"\|"parts"\|"synth"\|"record"\|"fit"\|"cancelled", message: String, fraction: Option<f32>` |
| `tonelab_job` | `job: { id, title, source_kind, duration_s, tempo_bpm: Option<f32>, key: Option<String>, sections: Vec<{ start, end, kind, guitar_db, corr }>, parts: Vec<{ id, label, start, end, channel, notes: u32, has_user_di, has_synth_di, fit: Option<{ template, error_db, holdout_error_db: Option<f32>, validate_error_db: Option<f32> }> }> }` |
| `tonelab_fit_result` | `job, part, fit: { template, error_db, holdout_error_db, validate_error_db }, templates: Vec<{ template, error_db, renders, stage }>` |

Errors use the existing `error` message. HTTP (same axum server): `GET /tonelab/file/{job}/{path}` serves a
job's WAV files (`mix.wav`, `stems/guitar.wav`, `parts/<id>/target.wav`, `parts/<id>/fit/render.wav`, DIs;
reject `..` and absolute paths), `POST /tonelab/upload` with the raw file as body and `X-Filename`
header → `{ "path": "..." }` (saved under the jobs dir in `uploads/`).

The server finds the tool at `tools/tonelab/bin/tonelab` (relative to the repo root it runs from) or
`$OPENPEDAL_TONELAB`. It runs the Python steps as subprocesses (`tonelab caps --json` gives `youtube`,
`device`, `jobs_dir`; steps print `PROGRESS <stage> <fraction|-> <message>` lines on stderr) and the fit
in-process with `openpedal-tonematch` on a background thread. One job step at a time per job.
After `tonelab_pick` the server also runs `synth-di` (matched to the most recent user DI in the jobs
dir if any). The mock server answers the tonelab messages from fixtures so the UI can be tested
without Python or a GPU.
