// SwiftPM needs at least one source file in a C target. The real code is the Rust static
// library (lib/libopenpedal.a, built by scripts/build-core.sh) linked through this target.

// Spike (docs/spikes/ios-cxx.md): when the core was built with OPENPEDAL_CXX_PROBE=1, keep a
// reference to the C++ entry point so the linker pulls it (and libc++) in. No app behaviour.
#ifdef OPENPEDAL_CXX_PROBE
#include "openpedal.h"
float (*const openpedal_cxx_probe_keep)(int) = openpedal_cxx_probe;
#endif
