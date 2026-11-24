# Spike: C++17 + Eigen in the iPad core (branch `spike/ios-cxx`, 2026-11-24)

Result: **GO.** C++17 code using Eigen and `std::thread` compiles from this Linux machine into
`libopenpedal.a` for `aarch64-apple-ios`, and the xtool build links it (verified with `xtool.sh dev build`,
build only, nothing installed). Not run on a device: only linked and symbol-checked.

## What was built

- `crates/cxx-probe` (`openpedal-cxx-probe`, workspace member): `build.rs` uses the `cc` crate (new
  dependency, fetched from crates.io; it was not in the lockfile) to compile `cpp/probe.cpp`: a 64x64 Eigen
  product on a `std::thread`, with a try/catch, exposed as `extern "C" float cxx_probe(int)`.
- `crates/ffi`: feature `cxx-probe` (off by default) exports `openpedal_cxx_probe(int)`; declared in
  `apps/ios/OpenPedalCore/include/openpedal.h`.
- `apps/ios/scripts/build-core.sh`: `OPENPEDAL_CXX_PROBE=1` turns it on. Default run is unchanged.
- `apps/ios/OpenPedalCore/stub.c` + `Package.swift`: when the core was built with the flag, build-core.sh
  leaves a marker `OpenPedalCore/lib/.cxx-probe`; Package.swift then defines `OPENPEDAL_CXX_PROBE` for the C
  target (stub.c keeps a reference to the probe so the linker pulls it in) and adds `-lc++`. No Swift or
  app behaviour change.
- `flake.nix` devShell: adds `eigen` (exports `EIGEN_INCLUDE_DIR`), `llvmPackages.clang-unwrapped`, `llvmPackages.llvm`.

## Recipe

```sh
OPENPEDAL_CXX_PROBE=1 nix develop -c apps/ios/scripts/build-core.sh
apps/ios/scripts/xtool.sh dev build            # build only
nix develop -c llvm-nm apps/ios/xtool/OpenPedal.app/OpenPedal | grep cxx_probe   # T _openpedal_cxx_probe
```

What build-core.sh sets for cargo (target `aarch64-apple-ios`, `T` = target with `-` as `_`):

| Env | Value | Why |
|---|---|---|
| `SDKROOT` | `~/.local/share/openpedal-xtool/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk` | `cc` uses it instead of `xcrun` (absent on Linux); `build.rs` passes it as `-isysroot` |
| `CXX_T` | a one-line wrapper: `exec clang++ -target arm64-apple-ios17.0 "$@"` | the nixpkgs clang-unwrapped (21.1.8) is a cross compiler for Apple targets; the *wrapped* nix clang injects Linux glibc/libstdc++ paths, so use the unwrapped one |
| `CXXFLAGS_T` | `-fno-lto -Wno-#warnings` | the object must be a plain Mach-O object (rustc's own LTO does not read clang bitcode); silences "platform no longer supported by libc++" from the SDK 27 headers at deployment target 17 |
| `AR_T` | `llvm-ar` | GNU `ar` makes a GNU-format archive; rustc warns "built as GNU format, but the target expects Darwin" and it may fail to link. `llvm-ar` writes Darwin archives |
| `EIGEN_INCLUDE_DIR` | from the nix shell | Eigen is header-only, same headers for Linux and iOS |

`build.rs` for iOS targets: `-std=c++17 -stdlib=libc++ -isysroot $SDKROOT -isystem $SDKROOT/usr/include/c++/v1`
and `cargo:rustc-link-lib=c++`. libc++ headers come from the SDK (it ships `usr/include/c++/v1` and
`libc++.tbd`), so no nixpkgs libcxx is needed. The Simulator target (`aarch64-apple-ios-sim`) is wired the same
way (iPhoneSimulator SDK, `-simulator` triple) but was not built.

## What failed and how it was solved

1. GNU-format archive warning: fixed with `AR_T=llvm-ar`.
2. SDK libc++ `#warning` about unsupported platform: harmless, silenced with `-Wno-#warnings`. A bash quoting
   slip (`\#`) made clang ignore the flag first.
3. Without a reference the linker would drop the probe from the static library (members are pulled only when
   referenced): the stub.c reference solves it for this spike. The real crate is referenced from Rust, so no
   action there.
4. clippy `needless_borrows_for_generic_args` in `build.rs`: fixed.

## Notes for iOS

- **libc++:** use the SDK's (`/usr/lib/libc++.1.dylib` at runtime, already linked by Swift apps: the baseline app
  already lists it). Never mix in libstdc++ or nixpkgs libc++ headers. Link flag `-lc++` (done in Package.swift).
- **std::thread:** works (libc++ over pthreads); `_pthread_join` resolves from libSystem. Thread count for
  Demucs is a plain `std::thread` / OpenMP-free design; OpenMP is not available on iOS (do not use `-fopenmp`).
- **Exceptions/RTTI:** on by default and linked (`___cxa_throw` resolves to libc++abi inside libc++.1.dylib).
  Do not throw across the `extern "C"` boundary: catch in C++ and return an error code (the probe does).
- **Panics:** the Rust side is unchanged; keep `catch_unwind` out of the C++ calls' way.
- **Eigen:** no BLAS/LAPACK needed. Do not define `EIGEN_USE_BLAS`; NEON is picked up by default on arm64.
  Consider `-O3 -ffast-math` only after checking Demucs parity. Accelerate (`vDSP`/BLAS) is in the SDK if matmul
  speed matters later (`-framework Accelerate`, add to Package.swift linkerSettings).
- **Deployment target:** the C++ is compiled for iOS 17.0 (`Package.swift` says iOS 26). Change the triple in
  build-core.sh if a newer libc++ API is needed (SDK headers guard by availability).
- **Alignment/allocation:** Eigen allocates through `malloc`; fine off the audio thread (Demucs is not real time).

## Cost

| | bytes |
|---|---|
| `libopenpedal.a`, flag off | 12,277,952 |
| `libopenpedal.a`, flag on | 12,322,976 (+45 KB) |
| App binary `OpenPedal`, flag off | 11,054,424 |
| App binary, flag on | 11,093,384 (+39 KB) |

The probe is tiny; the real size is Demucs code (small) plus its weights (separate: `chore/ios-weights`).
Build time: the C++ file compiles in ~2 s; `xtool dev build` after build-core takes ~13 s.

## Recipe for the real Demucs crate (`openpedal-separate`)

1. Vendor demucs.cpp sources under `third_party/`; vendor Eigen 3.4.1 headers too (or keep `EIGEN_INCLUDE_DIR`)
   so CI and other machines do not depend on the nix shell.
2. Copy `crates/cxx-probe/build.rs` almost as is: add all `.cpp` files, `-O3`, `-std=c++17`. Keep the iOS
   branch (SDKROOT, `-isysroot`, `-isystem .../c++/v1`, `-stdlib=libc++`, `rustc-link-lib=c++`).
3. Move the iOS environment (SDKROOT, `CXX_T` wrapper, `AR_T`, `CXXFLAGS_T`) from the `OPENPEDAL_CXX_PROBE`
   block into the default part of build-core.sh once the crate is a default dependency of `openpedal-ffi`.
   Desktop builds then need a C++ compiler (the nix shell has one).
4. Expose a C ABI from C++ (`extern "C"`, no exceptions out, no C++ types) wrapped by a safe Rust API.
5. Fall back to `-mcpu`/NEON defaults; do not pass `-march=native` (that is the host x86).
6. After adding the dependency, check that `apps/ios` still links: the Swift side needs no change besides `-lc++`
   (already conditional; make it unconditional with the move in step 3, or Package.swift reads the marker).

## Caveats

- Only linking was verified, not running on the iPad. The first device run is the real test of libc++ symbols
  (`xtool dev` with a call to `openpedal_cxx_probe(8)` expecting 128).
- `cargo test --workspace` and clippy now compile C++ on the desktop (crate is a workspace member): a C++
  compiler and Eigen (nix shell) are required. Without Eigen, set `EIGEN_INCLUDE_DIR` or install `eigen3`.
- Release LTO (`lto = true`) only covers Rust; the C++ objects are not LTO'd (flag `-fno-lto` is deliberate).
