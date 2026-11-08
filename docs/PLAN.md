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

Server → client:

| `type` | Fields |
|---|---|
| `state` | `protocol` (`PROTOCOL_VERSION`, currently 1), `catalog` (blocks + pedals, with parameters), `graph`, `presets`, `pedals` (content of composite pedals), `audio` (sample rate, buffer, running) |
| `meters` | `in_db`, `out_db`, `dsp` (0..1), `xruns`, about 20 times per second |
| `error` | `message` (shown to the user), `node` (optional: the node the error is about) |

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
desktop". Adding/removing/reordering on a chain is done client-side by rewriting `node`/`edge` and
sending `set_graph`.

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
   generated (typeshare). The JSON fixtures stay as contract tests.
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
