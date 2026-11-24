//! Source separation for Tone Lab: Demucs `htdemucs_6s` through the vendored demucs.cpp (C++17 + Eigen).
//!
//! The C++ side is a thin shim (`cpp/shim.cpp`) that runs Demucs' overlapping 7.8 s segments on up to
//! 4 threads. Input at another sample rate than 44.1 kHz is resampled in and out (`resample`).

mod resample;

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{bail, ensure, Context, Result};

pub use resample::resample;

/// Separated stems at `sample_rate` (the rate of the input), `[left, right]` each.
pub struct Stems {
    pub sample_rate: u32,
    /// "drums", "bass", "other", "vocals", "guitar", "piano" (first four for a 4-source model).
    pub stems: Vec<(String, [Vec<f32>; 2])>,
}

impl Stems {
    pub fn get(&self, name: &str) -> Option<&[Vec<f32>; 2]> {
        self.stems.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }
}

pub trait Separator: Send + Sync {
    /// `progress(fraction 0..1, message)` is called on the calling thread. Setting `cancel` aborts
    /// with an error whose message is "cancelled".
    fn separate(
        &self,
        left: &[f32],
        right: &[f32],
        sample_rate: u32,
        progress: &mut dyn FnMut(f32, &str),
        cancel: &AtomicBool,
    ) -> Result<Stems>;
}

/// True when the C++ side is linked and reports an error (not a crash) for a missing weights file.
pub fn linked() -> bool {
    DemucsCpp::load(Path::new("/nonexistent/openpedal-linked-check")).is_err()
}

/// Demucs via demucs.cpp. Model input is 44.1 kHz stereo.
pub struct DemucsCpp {
    model: *mut c_void,
    threads: usize,
}

// The model is only read during inference (const in C++); every call has its own buffers.
unsafe impl Send for DemucsCpp {}
unsafe impl Sync for DemucsCpp {}

const MODEL_RATE: u32 = 44_100;
const NAMES: [&str; 6] = ["drums", "bass", "other", "vocals", "guitar", "piano"];

type ProgressFn = extern "C" fn(f32, *const c_char, *mut c_void);

extern "C" {
    fn dmc_load(path: *const c_char, out: *mut *mut c_void, err: *mut c_char, err_len: usize) -> i32;
    fn dmc_n_sources(m: *const c_void) -> i32;
    fn dmc_separate(
        m: *const c_void,
        left: *const f32,
        right: *const f32,
        n: i64,
        threads: i32,
        progress: Option<ProgressFn>,
        user: *mut c_void,
        cancel: *const u8,
        out: *mut f32,
        err: *mut c_char,
        err_len: usize,
    ) -> i32;
    fn dmc_free(m: *mut c_void);
}

extern "C" fn progress_trampoline(fraction: f32, msg: *const c_char, user: *mut c_void) {
    // SAFETY: `user` is the `&mut &mut dyn FnMut` of the dmc_separate call below, used on its thread only.
    let cb = unsafe { &mut *(user as *mut &mut dyn FnMut(f32, &str)) };
    let msg = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
    // A panic must not unwind through C++.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(fraction, &msg)));
}

fn err_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl DemucsCpp {
    /// Loads the converted weights (demucs.cpp ggml format, ~52 MiB for htdemucs_6s).
    pub fn load(weights: &Path) -> Result<Self> {
        let path = CString::new(weights.to_string_lossy().as_bytes()).context("path contains NUL")?;
        let mut model = std::ptr::null_mut();
        let mut err = [0u8; 512];
        let rc = unsafe { dmc_load(path.as_ptr(), &mut model, err.as_mut_ptr() as *mut c_char, err.len()) };
        ensure!(rc == 0 && !model.is_null(), "{}", err_string(&err));
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4);
        Ok(Self { model, threads })
    }

    /// Number of segment threads, 1 to 4 (default: cores, at most 4). Each thread needs about 1.7 GB.
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads.clamp(1, 4);
        self
    }

    pub fn threads(&self) -> usize {
        self.threads
    }
}

impl Drop for DemucsCpp {
    fn drop(&mut self) {
        unsafe { dmc_free(self.model) }
    }
}

impl Separator for DemucsCpp {
    fn separate(
        &self,
        left: &[f32],
        right: &[f32],
        sample_rate: u32,
        progress: &mut dyn FnMut(f32, &str),
        cancel: &AtomicBool,
    ) -> Result<Stems> {
        ensure!(left.len() == right.len(), "left and right differ in length");
        ensure!(sample_rate > 0, "sample rate is 0");
        if left.is_empty() {
            bail!("no audio to separate");
        }
        let resampled = sample_rate != MODEL_RATE;
        let (l, r);
        let (lref, rref): (&[f32], &[f32]) = if resampled {
            progress(0.0, "Resampling to 44.1 kHz");
            l = resample(left, sample_rate, MODEL_RATE);
            r = resample(right, sample_rate, MODEL_RATE);
            (&l, &r)
        } else {
            (left, right)
        };
        let n = lref.len();
        let nsrc = unsafe { dmc_n_sources(self.model) } as usize;
        let mut out = vec![0f32; nsrc * 2 * n];
        let mut err = [0u8; 512];
        let mut cb: &mut dyn FnMut(f32, &str) = progress;
        let rc = unsafe {
            dmc_separate(
                self.model,
                lref.as_ptr(),
                rref.as_ptr(),
                n as i64,
                self.threads as i32,
                Some(progress_trampoline),
                &mut cb as *mut &mut dyn FnMut(f32, &str) as *mut c_void,
                cancel as *const AtomicBool as *const u8,
                out.as_mut_ptr(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if rc != 0 {
            bail!("{}", err_string(&err));
        }
        let mut stems = Vec::with_capacity(nsrc);
        for (s, name) in NAMES.iter().take(nsrc).enumerate() {
            let mut ch = [Vec::new(), Vec::new()];
            for (c, dst) in ch.iter_mut().enumerate() {
                let slice = &out[(s * 2 + c) * n..(s * 2 + c + 1) * n];
                *dst = if resampled { resample(slice, MODEL_RATE, sample_rate) } else { slice.to_vec() };
                dst.resize(left.len(), 0.0);
            }
            stems.push((name.to_string(), ch));
        }
        Ok(Stems { sample_rate, stems })
    }
}
