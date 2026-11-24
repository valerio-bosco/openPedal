// Our own driver around demucs.cpp: same maths as demucscpp::demucs_inference (normalise, shift, split into
// overlapping segments with a triangular transition), but the segments run on up to 4 std::threads, each with
// its own buffers, and a cancel flag is checked inside every segment (via the library's progress callback).
#include "shim.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <exception>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "model.hpp"

struct dmc_model {
    demucscpp::demucs_model model;
};

namespace {

struct Cancelled {};

void set_err(char *err, size_t len, const std::string &msg) {
    if (!err || len == 0) return;
    std::snprintf(err, len, "%s", msg.c_str());
}

// Same as the upstream symmetric_zero_padding (a static function there).
void zero_pad(Eigen::MatrixXf &padded, const Eigen::MatrixXf &original, int total) {
    int left = total / 2;
    int right = total - left;
    int n = (int)original.cols();
    padded.block(0, left, 2, n) = original;
    padded.block(0, 0, 2, left).setZero();
    padded.block(0, n + left, 2, right).setZero();
}

} // namespace

extern "C" int dmc_load(const char *path, dmc_model **out, char *err, size_t err_len) {
    try {
        if (!path || !out) {
            set_err(err, err_len, "null argument");
            return 1;
        }
        std::unique_ptr<dmc_model> m(new dmc_model());
        if (!demucscpp::load_demucs_model(path, &m->model)) {
            set_err(err, err_len, std::string("cannot load Demucs weights from ") + path);
            return 1;
        }
        *out = m.release();
        return 0;
    } catch (const std::exception &e) {
        set_err(err, err_len, e.what());
        return 1;
    } catch (...) {
        set_err(err, err_len, "unknown C++ exception");
        return 1;
    }
}

extern "C" int dmc_n_sources(const dmc_model *m) { return m && m->model.is_4sources ? 4 : 6; }

extern "C" void dmc_free(dmc_model *m) { delete m; }

extern "C" int dmc_separate(const dmc_model *m, const float *left, const float *right, int64_t n_in, int threads,
                            dmc_progress_fn progress, void *user, const uint8_t *cancel, float *out, char *err,
                            size_t err_len) {
    try {
        if (!m || !left || !right || !out || n_in <= 0) {
            set_err(err, err_len, "invalid arguments");
            return 1;
        }
        const int length = (int)n_in;
        const int nsrc = m->model.is_4sources ? 4 : 6;
        const auto cancelled = [&] { return cancel && __atomic_load_n(cancel, __ATOMIC_RELAXED) != 0; };

        // Normalise like Python Demucs: ref = mono mean, wav = (wav - ref.mean) / ref.std.
        Eigen::MatrixXf audio(2, length);
        for (int i = 0; i < length; ++i) {
            audio(0, i) = left[i];
            audio(1, i) = right[i];
        }
        Eigen::VectorXf ref = audio.colwise().mean();
        float ref_mean = ref.mean();
        float ref_std = length > 1 ? std::sqrt((ref.array() - ref_mean).square().sum() / (float)(length - 1)) : 1.0f;
        if (!(ref_std > 1e-9f)) ref_std = 1.0f; // silence
        audio = ((audio.array() - ref_mean) / ref_std).matrix();

        // Time-shift trick (fixed offset: deterministic output, unlike upstream's rand()).
        const int max_shift = (int)(demucscpp::MAX_SHIFT_SECS * demucscpp::SUPPORTED_SAMPLE_RATE);
        const int offset = max_shift / 3;
        Eigen::MatrixXf padded(2, length + 2 * max_shift);
        zero_pad(padded, audio, 2 * max_shift);
        audio.resize(0, 0);
        const int slen = length + max_shift - offset;
        const Eigen::MatrixXf shifted = padded.block(0, offset, 2, slen);
        padded.resize(0, 0);

        const int seg = (int)(demucscpp::SEGMENT_LEN_SECS * demucscpp::SUPPORTED_SAMPLE_RATE);
        const int stride = (int)((1 - demucscpp::OVERLAP) * seg);
        std::vector<int> offsets;
        for (int o = 0; o < slen; o += stride) offsets.push_back(o);
        const int chunks = (int)offsets.size();

        Eigen::VectorXf weight(seg);
        weight.setZero();
        weight.head(seg / 2) = Eigen::VectorXf::LinSpaced(seg / 2, 1, (float)(seg / 2));
        weight.tail(seg / 2) = weight.head(seg / 2).reverse();
        weight /= weight.maxCoeff();
        weight = weight.array().pow(demucscpp::TRANSITION_POWER);

        // Accumulators over the shifted signal: [source][channel][sample] and the weight sum.
        std::vector<float> acc((size_t)nsrc * 2 * slen, 0.0f);
        std::vector<float> wsum(slen, 0.0f);
        std::mutex acc_mutex;

        std::vector<std::atomic<float>> frac(chunks);
        for (auto &f : frac) f.store(0.0f);
        std::atomic<int> next{0};
        std::atomic<bool> failed{false};
        std::string failure;
        std::mutex failure_mutex;

        auto worker = [&] {
            try {
                demucscpp::demucs_segment_buffers buffers(2, seg, nsrc);
                demucscpp::stft_buffers stft(buffers.padded_segment_samples);
                for (;;) {
                    if (failed.load() || cancelled()) throw Cancelled();
                    int idx = next.fetch_add(1);
                    if (idx >= chunks) return;
                    const int o = offsets[idx];
                    const int clen = std::min(seg, slen - o);
                    Eigen::MatrixXf chunk = shifted.block(0, o, 2, clen);
                    zero_pad(buffers.mix, chunk, seg - clen);
                    const int left_pad = (seg - clen) / 2;
                    auto cb = [&](float f, const std::string &) {
                        if (failed.load() || cancelled()) throw Cancelled();
                        frac[idx].store(std::min(f, 1.0f), std::memory_order_relaxed);
                    };
                    demucscpp::model_inference(m->model, buffers, stft, cb, 0.0f, 1.0f);
                    {
                        std::lock_guard<std::mutex> lock(acc_mutex);
                        for (int s = 0; s < nsrc; ++s)
                            for (int c = 0; c < 2; ++c) {
                                float *dst = &acc[((size_t)s * 2 + c) * slen + o];
                                for (int k = 0; k < clen; ++k)
                                    dst[k] += weight(k) * buffers.targets_out(s, c, k + left_pad);
                            }
                        for (int k = 0; k < clen; ++k) wsum[o + k] += weight(k);
                    }
                    frac[idx].store(1.0f);
                }
            } catch (const Cancelled &) {
                failed.store(true);
            } catch (const std::exception &e) {
                std::lock_guard<std::mutex> lock(failure_mutex);
                if (failure.empty()) failure = e.what();
                failed.store(true);
            } catch (...) {
                std::lock_guard<std::mutex> lock(failure_mutex);
                if (failure.empty()) failure = "unknown C++ exception";
                failed.store(true);
            }
        };

        const int nthreads = std::max(1, std::min({threads <= 0 ? 4 : threads, 4, chunks}));
        std::vector<std::thread> pool;
        for (int t = 0; t < nthreads; ++t) pool.emplace_back(worker);

        // This thread reports progress, so the caller's callback never runs on a worker.
        auto report = [&] {
            if (!progress) return;
            float sum = 0;
            int done = 0;
            for (auto &f : frac) {
                float v = f.load(std::memory_order_relaxed);
                sum += v;
                if (v >= 1.0f) ++done;
            }
            std::string msg = "Separating segment " + std::to_string(std::min(done + 1, chunks)) + "/" + std::to_string(chunks);
            progress(0.98f * sum / (float)chunks, msg.c_str(), user);
        };
        for (;;) {
            bool all_done = next.load() >= chunks;
            if (all_done) {
                bool finished = true;
                for (auto &f : frac) finished = finished && f.load() >= 1.0f;
                if (finished || failed.load()) break;
            }
            if (failed.load()) break;
            report();
            std::this_thread::sleep_for(std::chrono::milliseconds(50));
        }
        for (auto &t : pool) t.join();

        if (failed.load()) {
            if (!failure.empty()) {
                set_err(err, err_len, failure);
                return 1;
            }
            set_err(err, err_len, "cancelled");
            return 2;
        }

        // Undo weighting, shift and normalisation.
        for (int s = 0; s < nsrc; ++s)
            for (int c = 0; c < 2; ++c) {
                const float *src = &acc[((size_t)s * 2 + c) * slen + (max_shift - offset)];
                const float *w = &wsum[max_shift - offset];
                float *dst = out + ((size_t)s * 2 + c) * length;
                for (int k = 0; k < length; ++k) dst[k] = src[k] / w[k] * ref_std + ref_mean;
            }
        if (progress) progress(1.0f, "Separation done", user);
        return 0;
    } catch (const std::exception &e) {
        set_err(err, err_len, e.what());
        return 1;
    } catch (...) {
        set_err(err, err_len, "unknown C++ exception");
        return 1;
    }
}
