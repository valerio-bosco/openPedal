//! C ABI for hosts that own the audio callback (the iPad's Swift shell; see `ios/` and
//! `ios/OpenPedalCore/include/openpedal.h`, which must stay in sync with this file).
//!
//! Threads and ownership:
//! - `openpedal_start` / `openpedal_stop` are called from a normal thread (never the audio one).
//! - `openpedal_process` is called from the audio thread only, never concurrently with itself
//!   or with `openpedal_stop`. It runs the real-time `Runner`: no allocation, no locks, no logging.
//! - Control (graph and parameter changes from the UI) reaches the audio thread through the
//!   engine's lock-free queues. Plans retired by the audio thread are freed by the thread that
//!   `LiveEngine` already runs for that purpose (it calls `collect_garbage` every 100 ms).
//! - The UI server runs on its own threads, bound to 127.0.0.1 only. `server::run` has no
//!   shutdown hook yet, so `openpedal_stop` stops the audio but the server threads (and the port)
//!   live until the process exits: start the core once per process.

use std::ffi::{c_char, CStr, CString};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::raw::c_float;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

use anyhow::{bail, Context, Result};

use openpedal_engine::engine::{AudioCore, Engine};
use openpedal_graph::model::GraphFile;
use openpedal_graph::{self as graph};
use openpedal_library::Library;
use openpedal_engine::live::LiveEngine;
use openpedal_server::{self as server, ServerOptions};

/// Opaque handle returned by `openpedal_start`.
pub struct OpenPedal {
    core: AudioCore,
    port: u16,
    /// Keeps the engine (and its garbage-collecting thread) alive.
    _live: Arc<LiveEngine>,
}

static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);

fn set_error(message: String) {
    let text = CString::new(message.replace('\0', " ")).unwrap_or_default();
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
}

/// The initial graph: `clean`, else the first preset, else an empty graph.
fn initial_graph(library: &Library) -> GraphFile {
    match library.initial_preset() {
        Some(Ok(g)) => return g,
        Some(Err(e)) => eprintln!("{e:#}: starting from an empty graph"),
        None => {}
    }
    server::mock::default_graph()
}

/// Asks the OS for a free port on the loopback interface.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    Ok(listener.local_addr()?.port())
}

/// Waits until the server accepts connections, so the web view never loads a dead port.
fn wait_for_server(port: u16) -> Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("the UI server did not start on port {port}")
}

fn start(sample_rate: f32, data_dir: &Path, factory_dir: Option<&Path>, port: u16) -> Result<Box<OpenPedal>> {
    let library = Library::new(data_dir);
    library.ensure_dirs()?;
    if let Some(factory) = factory_dir {
        library.seed_from(factory)?;
    }
    let pedals_dir = library.pedals_dir();

    let pedals = library.load_pedals()?;
    let mut graph = initial_graph(&library);
    let compiled = match graph::compile(&graph, &pedals, sample_rate) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("preset \"{}\" does not compile ({e:#}): starting from an empty graph", graph.name);
            graph = server::mock::default_graph();
            graph::compile(&graph, &pedals, sample_rate)?
        }
    };
    let (plan, map) = compiled;
    let (engine, core) = Engine::new(plan, sample_rate, None);
    let live = LiveEngine::new(engine, map, graph, pedals, pedals_dir);

    let port = if port == 0 { free_port()? } else { port };
    let engine: Arc<dyn server::EngineHandle> = live.clone();
    let opts = ServerOptions { port, open_browser: false, lan: false, library };
    thread::Builder::new().name("openpedal-server".into()).spawn(move || {
        if let Err(e) = server::run(engine, opts) {
            eprintln!("openPedal server stopped: {e:#}");
        }
    })?;
    wait_for_server(port)?;
    Ok(Box::new(OpenPedal { core, port, _live: live }))
}

/// Shared by `openpedal_start` and `openpedal_start2`.
///
/// # Safety
/// `data_dir` and `factory_dir` must each be NULL (factory only) or valid NUL-terminated strings.
unsafe fn start_from_c(sample_rate: c_float, max_frames: u32, data_dir: *const c_char, factory_dir: *const c_char, port: u16) -> *mut OpenPedal {
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Box<OpenPedal>> {
        if data_dir.is_null() {
            bail!("data_dir is NULL");
        }
        if !(sample_rate.is_finite() && (8000.0..=384_000.0).contains(&sample_rate)) || max_frames == 0 {
            bail!("invalid sample_rate ({sample_rate}) or max_frames ({max_frames})");
        }
        let dir = CStr::from_ptr(data_dir).to_str().context("data_dir is not valid UTF-8")?;
        let factory = if factory_dir.is_null() { None } else { Some(CStr::from_ptr(factory_dir).to_str().context("factory_dir is not valid UTF-8")?) };
        start(sample_rate, Path::new(dir), factory.map(Path::new), port)
    }));
    match result {
        Ok(Ok(handle)) => Box::into_raw(handle),
        Ok(Err(e)) => {
            set_error(format!("{e:#}"));
            std::ptr::null_mut()
        }
        Err(_) => {
            set_error("openpedal_start panicked".into());
            std::ptr::null_mut()
        }
    }
}

/// Builds the engine with the preset found in `data_dir/presets` and starts the UI server on
/// `127.0.0.1:port` (0 = pick a free port, see `openpedal_port`). `data_dir` holds `presets/`
/// and `pedals/` (created if missing). `max_frames` is the largest buffer the host expects; it
/// is only a hint, `openpedal_process` accepts any size. No factory content is seeded: see
/// `openpedal_start2`.
///
/// Returns NULL on failure; see `openpedal_last_error`.
///
/// # Safety
/// `data_dir` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn openpedal_start(sample_rate: c_float, max_frames: u32, data_dir: *const c_char, port: u16) -> *mut OpenPedal {
    start_from_c(sample_rate, max_frames, data_dir, std::ptr::null(), port)
}

/// Like `openpedal_start`, and first seeds `data_dir` from `factory_dir` (which holds `presets/`
/// and `pedals/`, e.g. the app bundle's resources): each factory file is copied once, a factory
/// preset the user deleted does not come back and user edits are never overwritten (the copied
/// names are remembered in `data_dir/.seeded.toml`). `factory_dir` may be NULL (no seeding).
///
/// # Safety
/// `data_dir` must be a valid NUL-terminated UTF-8 string; `factory_dir` NULL or the same.
#[no_mangle]
pub unsafe extern "C" fn openpedal_start2(sample_rate: c_float, max_frames: u32, data_dir: *const c_char, factory_dir: *const c_char, port: u16) -> *mut OpenPedal {
    start_from_c(sample_rate, max_frames, data_dir, factory_dir, port)
}

/// The core's version (e.g. "0.1.0") as a static NUL-terminated string: never NULL, never freed.
#[no_mangle]
pub extern "C" fn openpedal_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// The port the UI server listens on (127.0.0.1), or 0 if `handle` is NULL.
///
/// # Safety
/// `handle` must be NULL or a live handle from `openpedal_start`.
#[no_mangle]
pub unsafe extern "C" fn openpedal_port(handle: *const OpenPedal) -> u16 {
    handle.as_ref().map_or(0, |h| h.port)
}

/// Processes `frames` samples: mono `input` into `out_left` and `out_right` (same signal).
/// Real-time safe. If any pointer is NULL nothing is done.
///
/// # Safety
/// `handle` comes from `openpedal_start` and is not being stopped; the three buffers hold
/// at least `frames` floats and `input` does not overlap the outputs; the call comes from
/// one audio thread at a time.
#[no_mangle]
pub unsafe extern "C" fn openpedal_process(handle: *mut OpenPedal, input: *const c_float, out_left: *mut c_float, out_right: *mut c_float, frames: u32) {
    let Some(h) = handle.as_mut() else { return };
    if input.is_null() || out_left.is_null() || out_right.is_null() || frames == 0 {
        return;
    }
    let n = frames as usize;
    let input = std::slice::from_raw_parts(input, n);
    let out_l = std::slice::from_raw_parts_mut(out_left, n);
    let out_r = std::slice::from_raw_parts_mut(out_right, n);
    h.core.process(input, out_l, out_r);
}

/// Stops the audio and frees the handle (see the module docs: the UI server keeps running).
/// NULL is ignored.
///
/// # Safety
/// `handle` must be NULL or a handle from `openpedal_start` not used again afterwards; no
/// `openpedal_process` call may be running.
#[no_mangle]
pub unsafe extern "C" fn openpedal_stop(handle: *mut OpenPedal) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// A description of the last failure of `openpedal_start`, or NULL. Valid until the next failure.
#[no_mangle]
pub extern "C" fn openpedal_last_error() -> *const c_char {
    match LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        Some(text) => text.as_ptr(),
        None => std::ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};

    fn http_get(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn start_process_stop() {
        let dir = std::env::temp_dir().join(format!("openpedal-ffi-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let c_dir = CString::new(dir.to_str().unwrap()).unwrap();
        let c_factory = CString::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../content")).unwrap();

        unsafe {
            let h = openpedal_start2(48000.0, 512, c_dir.as_ptr(), c_factory.as_ptr(), 0);
            assert!(!h.is_null(), "start failed: {:?}", CStr::from_ptr(openpedal_last_error()));
            assert!(dir.join("presets/clean.toml").exists() && dir.join(".seeded.toml").exists(), "factory content was not seeded");
            let port = openpedal_port(h);
            assert_ne!(port, 0);
            assert!(http_get(port, "/").starts_with("HTTP/1.1 200"), "the UI server does not answer");

            // 100 ms of a 220 Hz sine in odd-sized buffers (the host may use any size).
            let mut phase = 0usize;
            let mut peak = 0.0f32;
            for frames in [64usize, 128, 100, 512, 37].into_iter().cycle().take(40) {
                let input: Vec<f32> = (0..frames).map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * (phase + i) as f32 / 48000.0).sin()).collect();
                phase += frames;
                let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
                openpedal_process(h, input.as_ptr(), l.as_mut_ptr(), r.as_mut_ptr(), frames as u32);
                assert!(l.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
                assert_eq!(l, r);
                peak = l.iter().fold(peak, |m, s| m.max(s.abs()));
            }
            assert!(peak > 0.01, "no signal came out (peak {peak})");

            // NULL arguments are ignored.
            openpedal_process(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), 64);
            openpedal_stop(h);
            openpedal_stop(std::ptr::null_mut());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_is_the_crate_version() {
        let v = unsafe { CStr::from_ptr(openpedal_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn start_reports_errors() {
        unsafe {
            assert!(openpedal_start(48000.0, 512, std::ptr::null(), 0).is_null());
            assert!(openpedal_start2(48000.0, 512, std::ptr::null(), std::ptr::null(), 0).is_null());
            assert!(!openpedal_last_error().is_null());
            let dir = CString::new("/tmp").unwrap();
            assert!(openpedal_start(0.0, 512, dir.as_ptr(), 0).is_null());
        }
    }
}
