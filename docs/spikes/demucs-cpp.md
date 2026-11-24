# Spike: demucs.cpp (Demucs htdemucs_6s without Python)

Branch `spike/demucs-cpp`, measured 2026-11-24 on Linux. Goal (PLAN §15): Tone Lab standalone on the
iPad needs Demucs `htdemucs_6s`, and PyTorch does not run on iOS.

## Verdict

**Conditional go for quality, no-go for speed as it is.** demucs.cpp reproduces the Python guitar stem
(within Python's own run-to-run noise) and is plain C++17 + Eigen, so it fits the iOS plan. But it is
slow and memory hungry: 86 s for a 24 s excerpt on one Ryzen 5800X core, 2.4 GB peak RAM, and extra
threads do not help in its single-run mode. Next step before committing to it: build it for iOS with
Accelerate BLAS (`-DEIGEN_USE_BLAS`) and measure on the M1. If that is still more than about 60 s per
20 s excerpt, switch to `demucs.onnx` + ONNX Runtime (CoreML provider). The excerpt-first design in
§15 stays valid either way.

## Sources and versions

| What | Value |
|---|---|
| demucs.cpp | https://github.com/sevagh/demucs.cpp (MIT), commit `4edae497a114de1ccb17ac9b4a2b8fc0144714d7` (2026-11-06) |
| submodules | eigen `dd8c71e62852b2fe429edb6682ac91fd1c578a26`, libnyquist `767efd97cdd7a281d193296586e708490eb6e54f` (CLI only), googletest (tests only), demucs `2496b8f7f12b01c8dd0187c040000c46e175b44d` (python, for the converter) |
| htdemucs_6s checkpoint | `5c90dfd2-34c22ccb.th` from `https://dl.fbaipublicfiles.com/demucs/hybrid_transformer/` (55.0 MB) |
| alternative | https://github.com/sevagh/demucs.onnx (not built here; HEAD 2026-02-08 `81fa192e`) |
| CPU | AMD Ryzen 7 5800X (8 cores / 16 threads), 31 GB RAM; other jobs were running (load 3-7), so timings are noisy (+/- 15 %) |

Everything lives in `spikes/demucs-cpp/`; `third_party/` and `work/` are gitignored (sources, weights, audio).

## 1. Support and build

- Supports `htdemucs`, `htdemucs_6s`, `htdemucs_ft`, `hdemucs_mmi` (README). 6s is selected by a flag at
  conversion time (magic `dmc6`); the CLI picks 4 or 6 sources from the weight file.
- Needs only C++17 + Eigen (vendored submodule, including the `unsupported/Tensor` and FFT modules).
  CMake option `USE_OPENBLAS` (default ON) links a BLAS and defines `EIGEN_USE_BLAS`; with it OFF the
  build uses plain Eigen + OpenMP (`find_package(OpenMP REQUIRED)`).
- Build used here: plain Eigen + OpenMP, `-Ofast -march=native` (AVX2 on this CPU), builds in about
  3 minutes. The repo's `cmake_minimum_required(VERSION 3.0)` needs `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`
  with current CMake.
- OpenBLAS build (nixpkgs openblas 0.3.33) configures and links, but `demucs.cpp.main` crashes with
  SIGSEGV right after "load_demucs_model" (also with `ulimit -s unlimited`). Not debugged; it matters
  only as a proxy for Accelerate on iOS, so it is an open item.

## 2. Weights

`scripts/convert-pth-to-ggml.py --six-source` needs `torch.load(weights_only=False)` under torch 2.7;
`spikes/demucs-cpp/convert.py` is a 6-line wrapper that patches that without touching the third-party
tree. It reads the torch hub cache, so the `.th` must be there first (the venv only had the HF
safetensors copy).

| File | Size | Notes |
|---|---|---|
| `5c90dfd2-34c22ccb.th` (original) | 54,996,327 B (52.4 MiB) | 27.4 M params, all float16 already |
| `ggml-model-htdemucs-6s-f16.bin` | 54,855,129 B (52.3 MiB) | what demucs.cpp loads |
| fp32 | not supported, would be about 110 MB | the converter writes the checkpoint dtype (fp16) and the loader reads `Eigen::half`; fp32 gives no quality gain since the source is fp16 |

Weights are small enough to ship in the app bundle (52 MiB). Resident after load: about 400 MB
(weights are expanded to float).

## 3. Quality (24 s excerpt = Cryogen 6-30 s, 44.1 kHz stereo; guitar stem)

Excerpt: 8-28 s of the song plus 2 s margins on each side. "pyexcerpt" = Python Demucs
(`-n htdemucs_6s -d cuda`, default `--shifts 1`) on the same excerpt. "pyfull" = the existing
full-song guitar stem (48 kHz, resampled to 44.1 kHz, cut at 6-30 s). All streams were aligned by
cross-correlation (lag was 0 for all). Metrics are over the whole 24 s: waveform correlation,
SDR = ratio of reference energy to difference energy, rms of the 1/3-octave level difference
(40 Hz-10 kHz, 25 bands).

Single run (`demucs.cpp.main`, "cpp") against the fair reference, plus Python's own spread:

| Candidate vs `pyexcerpt` | corr | SDR dB | 1/3-oct rms dB | level dB |
|---|---|---|---|---|
| **cpp (single run)** | **0.978** | **13.4** | **1.04** | -0.60 |
| Python again, run 2 (random shift) | 0.985 | 15.2 | 0.64 | -0.05 |
| Python `--shifts 0` | 0.978 | 12.7 | 0.94 | +0.69 |
| cpp `demucs_mt` 4 threads | 0.937 | 9.0 | 0.86 | +0.08 |
| cpp `demucs_mt` 1 thread | 0.843 | 3.6 | 2.03 | +1.81 |
| cpp `demucs_mt` 8 threads | 0.804 | 4.4 | 1.35 | -0.92 |
| pyfull (6-30 s of the full song) | 0.784 | 1.1 | 2.93 | +2.95 |

Against `pyexcerpt --shifts 0` (deterministic): cpp single run corr 0.949, SDR 9.7 dB, 1/3-oct 1.85 dB.

Reading:
- The single-run C++ stem is as close to Python as Python is to itself (corr 0.95-0.98 vs 0.97-0.985).
  Spectrum difference about 1-2 dB rms. Quality is fine for tone matching.
- `demucs_mt` (splits the audio into N parts with 0.75 s overlap) is clearly worse and erratic (0.80-0.94).
  Do not use it; parallelism has to come from somewhere else.
- **Excerpt vs full song differs more than C++ vs Python**: the same Python model gives corr only about
  0.78-0.81 between a 24 s excerpt and the full-song run, and the excerpt guitar is 2-3 dB quieter
  (Demucs normalises by the mix mean/std, and the context differs). So the Tone Lab fit must use
  features from the same pass (excerpt) it matches against, not mix excerpt and full-song stems.
  The full-song guitar tone/spectrum difference is about 2-3 dB rms in 1/3-octave bands.

## 4. Speed and memory (24 s excerpt, Ryzen 5800X, `nice -n 10`, one run at a time)

`demucs.cpp.main` (single process, `OMP_NUM_THREADS=n`, plain Eigen + OpenMP build):

| Threads | Wall | User+sys | Peak RSS |
|---|---|---|---|
| 1 | 86.5 s | 68 s + 18 s | 2.43 GB |
| 4 | 88.1 s | 69 s | 2.43 GB |
| 8 | 110.2 s | 83 s | 2.43 GB |

OpenMP gives nothing: the library has no `#pragma omp` and Eigen's Tensor contractions here stay single
threaded without a BLAS or a ThreadPool device (the upstream README says threading comes "through the
BLAS interface"). The 8-thread run was also disturbed by other load.

`demucs_mt.cpp.main n` (n std::threads, each on a slice, `OMP_NUM_THREADS=1`):

| Threads | Wall | CPU time (user) | Peak RSS |
|---|---|---|---|
| 1 | 92.4 s | 68 s | 2.46 GB |
| 4 | 57.1 s | 183 s | 7.28 GB |
| 8 | 59.7 s | 392 s | 13.3 GB |

The mt variant is memory-bound (about 1.7 GB per thread) and wastes work (padding to the 7.8 s model
segment). Python on the GTX 1060 (CUDA) does the same 24 s in about 3 s.

**Real time factor:** about 3.6x slower than real time on one 5800X core (24 s of audio in 86 s).

**Estimate for Apple M1 (not measured, rough):** a 5800X core and an M1 performance core are about equal
in single-thread speed, but demucs.cpp compiled for iOS loses AVX2 (`-march=native` here) for 128-bit
NEON, so expect roughly 1.0-2.0x of the time above: about 90-170 s for a 24 s excerpt (20 s plus
margins) single-threaded, plus the same 2.4 GB RAM. With Accelerate (AMX) as the BLAS the matrix work
could be several times faster, but that is unmeasured (my OpenBLAS build crashed). It is below the
"tens of seconds" hoped for in §15, so it needs the Accelerate/ONNX check above.

## 5. iOS feasibility notes

- **Dependencies:** C++17 standard library + Eigen (header only, vendored). No Linux-only code in
  `src/` (`fopen`/`fread` for the weights, `<cmath>`, Eigen FFT). The CLI (`cli-apps/`) uses
  libnyquist (WAV I/O), `std::filesystem` and `std::thread`; none of that is needed: the Rust side
  passes a float stereo buffer to `demucscpp::demucs_inference(model, audio, progressCallback)` and
  gets a `Tensor3dXf` back (6 x 2 x N).
- **Threads / OpenMP:** the library itself does not use OpenMP or threads (documented policy: it also
  builds for single-threaded WASM). OpenMP is only an optional Eigen backend, and gave no speed-up here.
  So single-threaded iOS builds work as they are. Parallelism would have to come from `EIGEN_USE_BLAS`
  with Accelerate, or from running independent 7.8 s segments on `std::thread`s ourselves (memory: about
  1.7 GB each, too much on iPad).
- **Weights:** loaded only from a file path (`load_demucs_model(std::string path, model*)`, uses
  `fopen`). Fine for an app-bundle path; loading from memory would need a small patch.
- **Standard / flags:** C++17; the CMake uses `-Ofast -march=native` (must be replaced by
  `-O3 -ffast-math`-like flags and no `-march=native` when cross-compiling); `-Ofast` with
  `-fno-unsafe-math-optimizations`. Progress callback exists (`ProgressCallback(float, string)`),
  good for the UI; no cancel hook (needs a small patch or a flag checked in the callback via exception).
- **Memory:** 2.4 GB peak for one 7.8 s segment (the model buffers are allocated up front). iPad M1 with
  8-16 GB can allow that for a foreground app, but it is the biggest risk on smaller iPads. Reducing it
  would mean patching buffer sizes.
- **Build integration:** `cc` crate compiling `src/*.cpp` with `-I vendor/eigen`; that is spike
  `spike/ios-cxx`'s question.

## Recommendation

1. Keep demucs.cpp as the primary candidate: quality parity and an easy build. Single-run mode only.
2. Next measurement (small): build with `-DEIGEN_USE_BLAS` against Accelerate for macOS/iOS (or fix the
   OpenBLAS crash) and time a 24 s excerpt on an M1. Go if it is at most about 60 s.
3. If it is not, use `demucs.onnx` + ONNX Runtime with CoreML (the model is exported without STFT; STFT
   is reused from demucs.cpp). Not measured here: htdemucs_6s export support and CoreML speed must be
   checked, and the ONNX weights would be a larger file (probably 100+ MB, unverified).
4. UI: show a "about a minute or two" progress bar, with a hard excerpt cap of about 20 s.

## Reproduce

```sh
cd spikes/demucs-cpp
git clone https://github.com/sevagh/demucs.cpp third_party/demucs.cpp
(cd third_party/demucs.cpp && git checkout 4edae497a114de1ccb17ac9b4a2b8fc0144714d7 && git submodule update --init --recursive)

# build (plain Eigen + OpenMP)
mkdir -p work/build-omp && cd work/build-omp
nix shell nixpkgs#cmake nixpkgs#gcc nixpkgs#gnumake -c bash -c \
  'cmake ../../third_party/demucs.cpp -DUSE_OPENBLAS=OFF -DCMAKE_POLICY_VERSION_MINIMUM=3.5 && make -j8'
cd ../..

# weights
mkdir -p ~/.cache/torch/hub/checkpoints
curl -fL -o ~/.cache/torch/hub/checkpoints/5c90dfd2-34c22ccb.th \
  https://dl.fbaipublicfiles.com/demucs/hybrid_transformer/5c90dfd2-34c22ccb.th
mkdir -p work/ggml
~/.local/share/openpedal-gpu/py convert.py third_party/demucs.cpp/scripts/convert-pth-to-ggml.py work/ggml --six-source

# excerpt 6-30 s (8-28 s + 2 s margins)
FF=$(nix build --no-link --print-out-paths 'nixpkgs#ffmpeg-headless^bin')/bin/ffmpeg
$FF -y -ss 6 -t 24 -i ~/Projects/openPedal/recordings/Cryogen.mp3 -ac 2 -ar 44100 -c:a pcm_s16le work/excerpt.wav

# C++ separation, timed (time from nixpkgs#time; /usr/bin/time does not exist on this NixOS)
cd work; T=$(nix build --no-link --print-out-paths nixpkgs#time)/bin/time
mkdir -p out-t1
OMP_NUM_THREADS=1 nice -n 10 $T -v build-omp/demucs.cpp.main ggml/ggml-model-htdemucs-6s-f16.bin excerpt.wav out-t1
mkdir -p outmt-t4
OMP_NUM_THREADS=1 nice -n 10 $T -v build-omp/demucs_mt.cpp.main ggml/ggml-model-htdemucs-6s-f16.bin excerpt.wav outmt-t4 4

# Python reference on the excerpt
~/.local/share/openpedal-gpu/py -m demucs -n htdemucs_6s -d cuda -o pyout excerpt.wav
~/.local/share/openpedal-gpu/py -m demucs -n htdemucs_6s -d cuda --shifts 0 -o pyout_s0 excerpt.wav

# compare (needs the full-song stem at ~/.local/share/openpedal/tonelab/jobs/2026-11-24-cryogen/stems/guitar.wav)
cd ..; ~/.local/share/openpedal-gpu/py -I compare.py work
```

Output stems are named `target_4_guitar.wav` etc. (order drums, bass, other, vocals, guitar, piano).
