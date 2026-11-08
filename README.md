# openPedal

[![CI](https://github.com/valerio-bosco/openPedal/actions/workflows/ci.yml/badge.svg)](https://github.com/valerio-bosco/openPedal/actions/workflows/ci.yml)

Real-time guitar effects without a DAW: guitar → audio interface → openPedal → headphones/speakers.
You build your sound as a **graph of pedals** in a node editor (or in TOML files), and you can create new composite pedals from existing ones.

## Quick start

```sh
nix develop                                # Rust, Node.js and PipeWire's libjack
(cd ui && npm install && npm run build)    # first time only, or after changing ui/
cargo run --release -- ui                  # opens http://127.0.0.1:7777
```

- Drag pedals from the palette and wire them from **Guitar** to **Amp (output)**: whatever reaches the Amp is what you hear.
- Knobs act live. Several cables into one node are summed, so you can build parallel chains.
- **Create pedal**: select several nodes and save them as a composite pedal (`pedals/<name>.toml`) with the knobs you choose. Double-click a composite pedal to edit it.
- Undo/redo (Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y), delete (Del, ×, right-click menu), **New** for an empty graph, **Tidy up** to lay pedals out along the signal path.
- Options: `ui PRESET` (initial preset, default `presets/clean.toml`), `--lan` (reachable from your phone), `--port N`, `--no-browser`, `--no-audio` (editor only), `--in`/`--out`/`--buffer` (see below).
- If no audio interface is available, the UI still starts in editor-only mode.
- UI development: `cd ui && npm run dev` (with `openpedal ui --no-browser` running for audio; otherwise it uses sample data).

Before playing: **turn off "Direct Monitor"** on the interface, or you will also hear the dry signal.
Plug the guitar into the instrument input (**INST** on); set the gain so `IN` peaks around -12 dB.

## Command line

```sh
cargo run --release                          # play presets/clean.toml, with a status line
cargo run --release -- presets/crunch.toml   # the preset reloads every time you save the file
cargo run --release -- ports                 # list the audio ports
cargo run --release -- check presets/*.toml  # validate presets without audio
cargo run --release -- catalog               # blocks and pedals as JSON
```

The status line shows input/output levels, DSP load, the actual buffer size and xruns (audio dropouts: they must stay at 0).

## Devices and latency

`openpedal.toml` picks input and output with regexes on port names (see `ports`); `--in REGEX` / `--out REGEX` override them.

`buffer` is how many samples are processed per cycle: fewer samples means less latency, but if the interface can't keep up the input **crackles**. Without a config file the default is 256 (5.3 ms at 48 kHz). On a Scarlett Solo 3rd gen, 96 (2 ms) is the smallest clean value; 48 and 64 drop audio. `tools/buffer-test.sh` records with several buffer sizes to find yours; `--buffer N` overrides the config.

## Recording and re-processing

```sh
cargo run --release -- presets/clean.toml --record recordings/take.wav --seconds 20   # left dry, right processed
cargo run --release -- render presets/crunch.toml recordings/take.wav recordings/take-crunch.wav
```

`render` runs a preset on the dry channel of a recording, so you can tweak presets without playing.

## Blocks

| `type`       | Parameters (default)                                                    |
|--------------|-------------------------------------------------------------------------|
| `gain`       | `db` (0)                                                                |
| `gate`       | `threshold_db` (-60), `attack_ms` (0.5), `hold_ms` (40), `release_ms` (80) |
| `compressor` | `threshold_db` (-20), `ratio` (4), `attack_ms` (5), `release_ms` (120), `knee_db` (6), `makeup_db` (0), `mix` (1) |
| `drive`      | `shape` = soft/hard/asym/fuzz/fold (soft), `gain_db` (12), `level_db` (-6), `bias` (0.2, asym only), `quality` = eco/alta (alta = 4x oversampling) |
| `tonestack`  | `bass`, `mid`, `treble` (0..1, 0.5), `model` = fender/marshall/vox      |
| `cab`        | from a file: `ir` (WAV path), `max_ms` (80). Procedural: `low_hz` (100), `low_q` (1.3), `high_hz` (5000), `mid_hz` (500), `mid_db` (-3), `presence_hz` (2500), `presence_db` (3), `ripple_db` (3), `seed` (1). Both: `level_db` (0) |
| `filter`     | `kind` = lowpass/highpass/bandpass/notch/peak/lowshelf/highshelf, `freq` (1000), `q` (0.707), `gain_db` (0) |
| `tremolo`    | `rate_hz` (5), `depth` (0.5), `wave` = sine/triangle/square             |
| `chorus`     | `rate_hz` (0.8), `base_ms` (12), `depth_ms` (3), `mix` (0.5)            |
| `delay`      | `time_ms` (350), `feedback` (0.35), `mix` (0.3), `damp_hz` (5000)       |
| `reverb`     | `room` (0.5), `damp` (0.5), `mix` (0.25), `predelay_ms` (10), `width_hz` (8000) |

The cab usually goes after the drive and before modulation and time effects. IR files go in `irs/` (see `irs/README.md`). With the procedural cab, every `seed` gives different cone irregularities, i.e. a different speaker.

## Preset and pedal files

A preset (`presets/*.toml`) is a graph: `[[node]]` entries (`id`, `type`, `pos`, `params`) and `[[edge]]` entries (`from`, `to`), with exactly one `input` and one `output` node. See `docs/examples/crunch-graph.toml`. Older presets made of a linear list of `[[block]]` entries still work: they are converted to a graph on load.

A composite pedal (`pedals/<id>.toml`) is a sub-graph between the implicit nodes `pedal_in` and `pedal_out`, plus `[[knob]]` entries that map a 0..1 knob onto an inner parameter. It is used in a graph as `type = "pedal:<id>"`. See `pedals/cab-4x12.toml`.

A misspelled parameter is an error, and the sound keeps playing with the previous graph.

## Architecture

- `src/dsp/`: the blocks and the catalog (`CATALOG` in `src/dsp/mod.rs`). Each block describes its parameters (`BlockDesc`), so the UI builds its knobs automatically.
- `src/graph/`: graph model, loading, compilation (composite pedals expanded, topological order) and the real-time executor.
- `src/engine.rs`: JACK client (served by PipeWire), real-time callback, lock-free swap of graphs with a 20 ms crossfade, meters.
- `src/server/`: HTTP + WebSocket server for the UI, behind the `EngineHandle` trait; `src/live.rs` implements it on the real engine.
- `ui/`: the node editor (React + React Flow), embedded in the binary at build time.

`docs/PLAN.md` describes the contracts between these parts. See `CONTRIBUTING.md` before sending changes.
