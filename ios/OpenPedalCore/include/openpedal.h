/* openPedal core: C ABI for hosts that own the audio callback.
 * Hand-written; keep in sync with src/ffi.rs.
 *
 * Threads: start/stop/port/last_error from a normal thread, process from the audio thread
 * only (never concurrently with itself or with stop). process is real-time safe: it does
 * not allocate, lock or log. */
#ifndef OPENPEDAL_H
#define OPENPEDAL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle. */
typedef struct OpenPedal OpenPedal;

/* Builds the engine with the preset found in `data_dir`/presets (clean.toml, else the first
 * one, else an empty graph) and starts the UI server on 127.0.0.1:`port` (0 = pick a free
 * port, see openpedal_port). `data_dir` holds presets/ and pedals/ (created if missing).
 * `max_frames` is only a hint: openpedal_process accepts any buffer size.
 * Returns NULL on failure (see openpedal_last_error). When it returns, the server accepts
 * connections. Call it once per process. */
OpenPedal *openpedal_start(float sample_rate, uint32_t max_frames, const char *data_dir, uint16_t port);

/* The port the UI server listens on (127.0.0.1), or 0 if handle is NULL. */
uint16_t openpedal_port(const OpenPedal *handle);

/* Processes `frames` samples: mono `input` into `out_left` and `out_right` (same signal).
 * Real-time safe. Does nothing if any pointer is NULL. `input` must not overlap the outputs. */
void openpedal_process(OpenPedal *handle, const float *input, float *out_left, float *out_right, uint32_t frames);

/* Stops the audio and frees the handle (NULL is ignored). The UI server keeps running until
 * the process exits. */
void openpedal_stop(OpenPedal *handle);

/* Description of the last failure of openpedal_start, or NULL. Valid until the next failure. */
const char *openpedal_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* OPENPEDAL_H */
