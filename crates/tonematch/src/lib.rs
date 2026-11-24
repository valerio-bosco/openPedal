//! Tone matching: fit the knobs of a chain template (a preset plus parameter ranges) so that its
//! output, played from a clean DI, has the long-term spectrum and crest factor of a target.
//!
//! * [`features`]: 1/3-octave long-term spectrum and crest factor, and the cost between two.
//! * [`optim`]: Nelder-Mead in the unit cube with restarts.
//! * [`template`]: loading and validating `<name>.toml` + `<name>.fit.toml`.
//! * [`render`]: in-process rendering of a graph over a DI, exactly like `openpedal render`.
//! * [`fit`]: screening, full fit, hold-out, and the job folder output.

pub mod features;
pub mod fit;
pub mod optim;
pub mod render;
pub mod template;

pub use features::{Features, BANDS};
pub use fit::{fit_job, FitOptions, FitSummary, Progress};
pub use template::Template;

/// Lowers the priority of the process (nice 10) so a fit does not starve the audio. Errors are ignored.
pub fn lower_priority() {
    #[cfg(unix)]
    // SAFETY: plain syscall with integer arguments.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
}
