# Tone Lab (Python side)

Turns a song into a guitar part: fetch, separate (Demucs), analyse, transcribe, synthetic DI.
Every command reads and writes a job folder (see `docs/PLAN.md` section 14.2).

## Setup

```
bin/tonelab --help        # first run creates the venv in ~/.local/share/openpedal-gpu (uv sync)
```

`bin/tonelab` is the entry point on NixOS: it resolves libstdc++, zlib and ffmpeg from nixpkgs
(cached in the venv dir), sets `LD_LIBRARY_PATH`, runs at `nice 10` with one BLAS thread.
PyTorch is pinned to 2.7.1+cu126 (the GTX 1060 is Pascal; newer CUDA wheels dropped it).
Override the venv with `TONELAB_VENV`. YouTube support: `TONELAB_UV_ARGS="--extra youtube"` on first run,
or `uv sync --extra youtube`.

## Config

`tonelab.toml` has the defaults; `tonelab.local.toml` (gitignored) overrides them. `gpu.device = "cuda"`
fails loudly when CUDA is missing; `"cpu"` only when you set it. `[sources] youtube = false` disables links
(or delete `tonelab/sources/youtube.py` and the `youtube` extra). Downloading from YouTube may break its
Terms of Service; it is meant for your own use.

## Commands

```
bin/tonelab fetch <path|url> [--title T]      # prints the job id
bin/tonelab separate <job> [--force|--skip]   # Demucs htdemucs_6s; default mode "auto"
bin/tonelab analyse <job>                     # analysis.json + section table
bin/tonelab parts <job> --start 8 --end 28 --channel mid [--label riff]
bin/tonelab synth-di <job> --part p1 [--match-di di.wav]
bin/tonelab run <path|url>                    # fetch + separate + analyse
```

`<job>` is a job id (under `paths.jobs`) or a job folder path. Tests: `uv run pytest` in this folder.
