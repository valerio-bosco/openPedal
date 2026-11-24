// C ABI around demucs.cpp. No exception ever crosses it: failures return a nonzero code and fill `err`.
#pragma once
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

typedef struct dmc_model dmc_model;

// Progress: fraction 0..1 and a short message. Always called from the thread that called dmc_separate.
typedef void (*dmc_progress_fn)(float fraction, const char *message, void *user);

// 0 = ok. `err` receives a message on failure (always NUL terminated).
int dmc_load(const char *weights_path, dmc_model **out, char *err, size_t err_len);
// 4 or 6 sources (drums, bass, other, vocals[, guitar, piano]).
int dmc_n_sources(const dmc_model *m);
// Separates stereo 44.1 kHz audio. `out` holds n_sources * 2 * n floats, laid out [source][channel][sample].
// `cancel` (may be NULL) points to a byte; nonzero cancels (checked inside segments), return code 2.
int dmc_separate(const dmc_model *m, const float *left, const float *right, int64_t n, int threads,
                 dmc_progress_fn progress, void *user, const uint8_t *cancel, float *out, char *err,
                 size_t err_len);
void dmc_free(dmc_model *m);

#ifdef __cplusplus
}
#endif
