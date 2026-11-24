# Tone Lab (Python side)

Tone Lab turns a song into a guitar part you can play through openPedal. It fetches the audio,
separates the stems with Demucs (GPU), analyses where the guitar plays, transcribes a chosen part
and makes a synthetic clean DI. The Rust side (`openpedal tone-match`) then fits a chain template to
that part. Every command reads and writes a job folder. For the design, see
[`docs/PLAN.md`](../../docs/PLAN.md) section 14. For the user guide, see [`docs/tonelab.md`](../../docs/tonelab.md).

## Requirements

- NixOS or another Linux with `nix` (the wrapper builds `ffmpeg-headless`, `zlib` and `libstdc++` from nixpkgs).
- [uv](https://docs.astral.sh/uv/). Python 3.12 is required; uv fetches it when missing.
- An NVIDIA GPU with CUDA for separation. The pinned PyTorch is 2.7.1+cu126 because the GTX 1060
  (Pascal) is not supported by the newer CUDA wheels.
- CPU only if you set `device = "cpu"` explicitly. There is no silent fallback.

## Install and first run

```
bin/tonelab --help
```

The first run creates the venv in `tools/tonelab/.venv` (`uv sync`), gitignored. To reuse an existing
venv, set `TONELAB_VENV` (for example the developer's GPU venv):

```
export TONELAB_VENV=~/.local/share/openpedal-gpu
```

`TONELAB_UV_ARGS` replaces the `uv sync` arguments on the first run. Tests: `uv run pytest` in this folder.

## Config

`tonelab.toml` holds the committed defaults. `tonelab.local.toml` (gitignored) overrides any key.

| Key | Default | Meaning |
|---|---|---|
| `[sources] youtube` | `true` | Allow YouTube links (see the note below). `false` disables them. |
| `[gpu] device` | `"cuda"` | `"cuda"` fails loudly if CUDA is missing. `"cpu"` must be set on purpose (slow). |
| `[paths] jobs` | `~/.local/share/openpedal/tonelab/jobs` | Where job folders are created. |
| `[separate] mode` | `"auto"` | `auto` skips Demucs when the source is already guitar only; `always`; `never` (guitar.wav = mix). |
| `[separate] segment` | `7` | Seconds per Demucs chunk, small enough for 3 GB of VRAM. |

Example `tonelab.local.toml`:

```toml
[paths]
jobs = "/data/tonelab/jobs"

[separate]
mode = "always"
```

> **YouTube: read this.** Downloading from YouTube may violate YouTube's Terms of Service. The
> option is there for the personal use of whoever runs this repo. It is not meant for redistribution.
> To turn it off, set `[sources] youtube = false`. To remove it entirely, delete
> `tonelab/sources/youtube.py` and the `youtube` extra in `pyproject.toml`.

## Commands

`bin/tonelab <command>`. `<job>` is a job id (a folder under `paths.jobs`) or a job folder path.

| Command | What it does | Example |
|---|---|---|
| `fetch <path\|url> [--title T]` | Creates a job, writes `mix.wav`, prints the job id. URLs need `youtube = true`. | `bin/tonelab fetch ~/Music/song.mp3 --title "Song"` |
| `separate <job> [--force\|--skip]` | Demucs `htdemucs_6s` into `stems/`. `--force` always separates, `--skip` never does. Without flags, the config mode applies. | `bin/tonelab separate <job> --force` |
| `analyse <job>` | Writes `analysis.json` and prints the section table. | `bin/tonelab analyse <job>` |
| `parts <job> --start S --end E --channel mid\|left\|right [--label L]` | Cuts a section of the guitar stem into a part (`p1`, `p2`, ...), writes `target.wav` and the notes. | `bin/tonelab parts <job> --start 8 --end 28 --channel mid --label riff` |
| `synth-di <job> --part ID [--match-di FILE]` | Makes the synthetic DI from the part's notes. `--match-di` matches its spectrum to a real DI. | `bin/tonelab synth-di <job> --part p1 --match-di di.wav` |
| `run <path\|url> [--title T]` | `fetch` + `separate` + `analyse`. Prints the job id last. | `bin/tonelab run ~/Music/song.mp3` |
| `caps [--json]` | What this install supports: `youtube`, `device`, `jobs_dir`. The openPedal server reads the JSON to answer `tonelab_caps`. | `bin/tonelab caps --json` |

### Progress lines

Long steps (`fetch`, `separate`, `analyse`, `parts`, `synth-di`) print lines on stderr that the openPedal server turns into progress bars:

```
PROGRESS <stage> <fraction|-> <message>
```

`<stage>` is `fetch`, `separate`, `analyse`, `parts` or `synth`; `<fraction>` is 0 to 1, or `-` when unknown; `<message>` is the rest of the line.

### Job folder

Default root: `~/.local/share/openpedal/tonelab/jobs/<id>/`, where `<id>` is the date and a slug of the title.

| File | Written by | Content |
|---|---|---|
| `job.json` | fetch | id, source (kind, ref, title), created, sample rate, duration |
| `mix.wav` | fetch | the song, float32, original rate, stereo |
| `stems/{guitar,bass,drums,vocals,piano,other}.wav` | separate | Demucs output (or `guitar.wav` = mix when skipped) |
| `analysis.json` | analyse | tempo, key, sections (`kind`: centre, double, wide, none), low notes |
| `parts.json` | parts | the parts: id, label, start, end, channel, notes |
| `parts/<id>/target.wav` | parts | the separated part, mono, 48 kHz |
| `parts/<id>/di-synth.wav` | synth-di | synthetic DI, mono, 48 kHz |
| `parts/<id>/di-user.wav` | you | your recording of the part (optional) |
| `parts/<id>/fit/` | tone-match | `preset.toml`, `report.json`, `render.wav` |

## The fit

Run from the repository root (the default templates path is relative to it):

```
openpedal tone-match <job-dir> --part <id> [--di FILE] [--validate-di FILE] [--templates DIR] [--template NAME] [--budget N]
```

`<job-dir>` is a folder path, not a job id.

| Option | Meaning |
|---|---|
| `--part ID` | Required. The part from `parts.json`, e.g. `p1`. |
| `--di FILE` | A real DI to fit through (a recording at 48 kHz; the first channel is used). Enables the hold-out. |
| `--validate-di FILE` | Another real DI. The fitted preset is rendered on it and compared with the target. |
| `--templates DIR` | Template folder. Default `content/tonematch/templates`. |
| `--template NAME` | Fit one template only, skipping the screening. |
| `--budget N` | Evaluations of the full fit per template. Default 350. |

The DI is chosen in this order: `--di`, then `parts/<id>/di-user.wav`, then `parts/<id>/di-synth.wav`.
The fit runs on the CPU at low priority (`nice 10`).

### Reading the summary

The summary has one row per template (`error dB`, `holdout`, `renders`, `stage`), then the best one.
Lower is better for all the numbers.

- **error dB**: the spectral error on the fit window, the RMS of the differences in 1/3-octave bands (dB).
- **holdout**: error on the last 30 % of the audio, which the fit never saw. Only with a real DI (`--di` or `di-user.wav`).
- **validation**: error on the `--validate-di` file, rendered with the finished preset. Only if you pass that option.

A held-out number is the honest one. A fit that only saw a synthetic DI can look perfect and generalise
badly, so check `validation` with a real DI.

The result goes to `parts/<id>/fit/`: `preset.toml` (a normal openPedal preset), `report.json` (all
values, including `crest_db`, `diff_db` and the chosen `params`) and `render.wav` (the DI through the preset).

### Templates

Each template is a chain in `content/tonematch/templates/<name>.toml` with its knob ranges in `<name>.fit.toml`.

| Template | Chain |
|---|---|
| `clean` | gate, highpass, tonestack, low-gain amp (soft), cab, room |
| `crunch` | as `clean`, amp with asymmetric clipping at medium gain |
| `overdrive` | pedal-style soft drive before the stack and the amp |
| `fuzz` | transistor-style fuzz before the stack and the amp |
| `fuzz-octave` | an octave-down (and dry) stage before the fuzz |
| `highgain` | low-gain soft boost before a hard, high-gain amp |

Each template ends with the cab and reverb (the reverb mix is fitted). The `.fit.toml` ranges say which knobs are searched.

## Known limits

- Modulation (phaser, chorus) and delay are not detected yet.
- The low-note hint (`low_notes`) can miss an octave-down effect. The fit does not depend on it.
- Two guitars in the same place at the same time cannot be separated.
