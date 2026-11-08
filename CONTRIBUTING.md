# Contributing to openPedal

Thanks for helping! Read `docs/PLAN.md` first: it describes the architecture and the contracts between the engine, the server and the UI.

## Development setup

The project uses a Nix flake for its toolchain (Rust, Node.js, PipeWire's libjack):

```sh
nix develop
(cd apps/desktop/ui && npm install && npm run build)
cargo test --workspace
cargo run --release -- ui --no-audio      # editor without touching your audio interface
```

Without Nix you need Rust, Node.js, JACK development files and, on Linux with PipeWire, `pw-jack` to run.

## Before sending a change

- `cargo test --workspace` passes and `cargo clippy --workspace --all-targets` has no warnings.
- If you changed `apps/desktop/ui/`: `npm run build` passes (it includes the TypeScript check).
- If you changed the UI behavior: `tools/e2e/run.sh` passes (headless Chromium tests; keep the button labels they rely on or update the tests).
- If you changed the protocol or a block's parameters: regenerate the fixtures with `cargo run -- dev-fixtures apps/desktop/ui/src/fixtures` and keep `apps/desktop/ui/src/protocol.ts` in sync.
- New behavior comes with a test. DSP tests should measure something (levels, frequencies, timing), not just "it runs".

## Language

Everything in the repository is in **English**: code, comments, documentation, error and CLI messages, test names and UI text.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/), in English, imperative mood:

```
type(scope): short description

Optional body explaining why.
```

- Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `build`.
- Scopes: `ui`, `dsp`, `graph`, `engine`, `server`, `nix`, `docs`.
- Examples: `feat(ui): add preset deletion`, `fix(dsp): smooth filter coefficient changes`, `docs: describe the WebSocket protocol`.

Commits before November 2026 are in Italian; history is not rewritten.

## Real-time rules

The audio thread runs `Block::process`, `Block::set_param` and the graph executor. In that code:

- no allocations (`Vec::new`, `vec!`, `Box::new`, `String`, `format!`, `collect`…): allocate everything in `build_*`;
- no locks, no I/O, no logging;
- smooth abrupt parameter changes (about 10-20 ms) to avoid clicks; `Smoothed` in `crates/dsp/src/lib.rs` helps.

Data reaches the audio thread through lock-free `rtrb` queues; anything the audio thread discards is sent back and freed elsewhere.

## Adding a block

1. Create `crates/dsp/src/<name>.rs` with `pub const <NAME>_DESC: BlockDesc` (parameters with ids, labels, ranges, units) and `pub fn build_<name>(values: &ParamValues, sr: f32) -> Result<Box<dyn Block>>`.
2. Implement `Block::process` and `Block::set_param` (`index` follows the order of the parameters in the descriptor; use named constants for the indices).
3. Add one line to `CATALOG` in `crates/dsp/src/lib.rs` and declare the module.
4. The UI picks it up automatically: knobs are generated from the descriptor.

Parameter `id`s, block `kind`s and choice option names end up in users' preset files: once released, don't rename them.

## Adding a composite pedal

No code needed: build it in the UI (*Create pedal*) or write `content/pedals/<id>.toml` (see `content/pedals/cab-4x12.toml` and `docs/PLAN.md` §5.3).

## Golden renders

`crates/graph/tests/golden.rs` renders every tracked preset (`content/presets/*.toml`, except names starting with `test`) over a deterministic synthetic guitar signal (1.5 s, 48 kHz, blocks of 128) and compares the result with `crates/graph/tests/golden/<preset>.f32` (raw little-endian f32, mono; max abs diff 1e-5). It also checks that the output is finite and not silent.

If a sound change is intended, regenerate the references and commit them with the change:

```
OPENPEDAL_BLESS=1 cargo test -p openpedal-graph --test golden
```

Never bless to make an unexpected failure go away: refactors must not change the audio.
