//! Audio engine: one mono input and two outputs, independent of the audio API.
//! The realtime callback (`AudioCore::process`) runs the current plan (compiled graph);
//! who calls it is an `AudioBackend` (JACK on Linux, see `crate::audio`) or a host
//! that owns the audio callback (the C ABI in `crate::ffi`).
//! New plans and parameter changes arrive through a single lock-free queue,
//! so in order; retired plans are sent back to be deallocated outside the audio
//! thread.
//!
//! A new plan comes in with a 20 ms crossfade: both plans run during that time.

use std::any::Any;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::audio::AudioBackend;
use crate::graph::exec::MAX_BLOCK;
use crate::graph::{ParamChange, Plan};

/// Duration of the fade between two plans.
const FADE_SECONDS: f32 = 0.02;

/// Values measured in the audio thread and read by the interface.
#[derive(Default)]
pub struct Meters {
    in_peak: AtomicU32,
    out_peak: AtomicU32,
    /// Maximum callback load, in thousandths of the available time.
    load_permille: AtomicU32,
    pub xruns: AtomicUsize,
    /// Buffer size of the last cycle: PipeWire can change it on the fly.
    pub buffer_size: AtomicU32,
}

impl Meters {
    /// Returns (input peak, output peak, load 0..1) and resets them.
    pub fn take(&self) -> (f32, f32, f32) {
        (
            f32::from_bits(self.in_peak.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.out_peak.swap(0, Ordering::Relaxed)),
            self.load_permille.swap(0, Ordering::Relaxed) as f32 / 1000.0,
        )
    }
}

pub(crate) fn store_max(cell: &AtomicU32, v: f32) {
    // Only valid for positive f32 values, for which bit order matches numeric order.
    cell.fetch_max(v.to_bits(), Ordering::Relaxed);
}


/// Message to the audio thread. Arrival order matters: a parameter change
/// applies to the plan that was the latest to arrive when it was sent.
enum Msg {
    Plan(Box<Plan>),
    #[allow(dead_code)] // sent by `Engine::set_param`, which the integration (I2) will use
    Param(ParamChange),
}

struct Fade {
    old: Box<Plan>,
    /// Fade samples already processed.
    pos: usize,
}

/// Runs the current plan and handles the fade. Does not depend on JACK, so it is tested in unit tests.
struct Runner {
    plan: Box<Plan>,
    fade: Option<Fade>,
    fade_len: usize,
    /// Output of the old plan during the fade (preallocated).
    scratch: Vec<f32>,
    incoming: Consumer<Msg>,
    trash: Producer<Box<Plan>>,
}

impl Runner {
    fn new(plan: Box<Plan>, sample_rate: f32, incoming: Consumer<Msg>, trash: Producer<Box<Plan>>) -> Runner {
        let fade_len = ((FADE_SECONDS * sample_rate) as usize).max(1);
        Runner { plan, fade: None, fade_len, scratch: vec![0.0; MAX_BLOCK], incoming, trash }
    }

    /// Reads messages. A new plan waits for the current fade to end
    /// (and with it, the messages that follow it).
    fn poll(&mut self) {
        loop {
            if self.fade.is_some() && matches!(self.incoming.peek(), Ok(Msg::Plan(_))) {
                break;
            }
            match self.incoming.pop() {
                Ok(Msg::Plan(new)) => {
                    let old = std::mem::replace(&mut self.plan, new);
                    self.fade = Some(Fade { old, pos: 0 });
                }
                Ok(Msg::Param(c)) => self.plan.set_param(c.step, c.param, c.value),
                Err(_) => break,
            }
        }
    }

    fn run(&mut self, input: &[f32], out: &mut [f32]) {
        self.poll();
        for (inp, out) in input.chunks(MAX_BLOCK).zip(out.chunks_mut(MAX_BLOCK)) {
            out.copy_from_slice(inp);
            let Some(fade) = &mut self.fade else {
                self.plan.process(out);
                continue;
            };
            let n = out.len();
            let old = &mut self.scratch[..n];
            old.copy_from_slice(inp);
            fade.old.process(old);
            self.plan.process(out);
            for (k, (o, a)) in out.iter_mut().zip(old.iter()).enumerate() {
                let t = ((fade.pos + k + 1) as f32 / self.fade_len as f32).min(1.0);
                *o = a * (1.0 - t) + *o * t;
            }
            fade.pos += n;
            if fade.pos >= self.fade_len {
                if let Some(done) = self.fade.take() {
                    // If the queue were full, the plan would be freed here: that never happens,
                    // because the main thread drains it continuously.
                    let _ = self.trash.push(done.old);
                }
            }
        }
    }
}

/// The audio-thread half of the engine: everything a backend (or a host) needs to turn
/// input samples into output samples. `process` never allocates, locks or logs.
pub struct AudioCore {
    runner: Runner,
    meters: Arc<Meters>,
    sample_rate: f32,
    /// Optional recording: interleaved samples (dry, processed).
    record: Option<Producer<f32>>,
}

impl AudioCore {
    pub fn meters(&self) -> &Arc<Meters> {
        &self.meters
    }

    /// `input` (mono) is processed into `out_l`, which is copied to `out_r`.
    /// All three slices must have the same length.
    pub fn process(&mut self, input: &[f32], out_l: &mut [f32], out_r: &mut [f32]) {
        let start = Instant::now();
        store_max(&self.meters.in_peak, input.iter().fold(0.0f32, |m, s| m.max(s.abs())));

        self.runner.run(input, out_l);

        // Protection for ears and speakers: no NaN and nothing above 0 dBFS.
        let mut peak = 0.0f32;
        for s in out_l.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
            *s = s.clamp(-1.0, 1.0);
            peak = peak.max(s.abs());
        }
        store_max(&self.meters.out_peak, peak);
        out_r.copy_from_slice(out_l);

        if let Some(rec) = &mut self.record {
            // If the writer thread cannot keep up, samples are dropped, never real time.
            for (dry, wet) in input.iter().zip(out_l.iter()) {
                let _ = rec.push(*dry);
                let _ = rec.push(*wet);
            }
        }

        self.meters.buffer_size.store(input.len() as u32, Ordering::Relaxed);
        let budget = input.len() as f32 / self.sample_rate;
        let load = start.elapsed().as_secs_f32() / budget;
        self.meters.load_permille.fetch_max((load * 1000.0) as u32, Ordering::Relaxed);
    }
}

/// The control half of the engine, used from normal threads.
pub struct Engine {
    /// Keeps the backend's stream alive; dropping it stops the audio.
    _backend: Option<Box<dyn Any + Send>>,
    to_audio: Producer<Msg>,
    from_audio: Consumer<Box<Plan>>,
    pub meters: Arc<Meters>,
    pub sample_rate: f32,
}

impl Engine {
    /// Creates both halves without a backend: the caller drives `AudioCore::process`
    /// (a host that owns the audio callback, or a test).
    pub fn new(plan: Plan, sample_rate: f32, record: Option<Producer<f32>>) -> (Engine, AudioCore) {
        let (to_audio, incoming) = RingBuffer::new(1024);
        let (trash, from_audio) = RingBuffer::new(64);
        let meters = Arc::new(Meters::default());
        let core = AudioCore {
            runner: Runner::new(Box::new(plan), sample_rate, incoming, trash),
            meters: meters.clone(),
            sample_rate,
            record,
        };
        (Engine { _backend: None, to_audio, from_audio, meters, sample_rate }, core)
    }

    /// Starts the engine on an audio backend.
    pub fn start(backend: Box<dyn AudioBackend>, plan: Plan, record: Option<Producer<f32>>) -> Result<Engine> {
        let (mut engine, core) = Engine::new(plan, backend.sample_rate(), record);
        engine._backend = Some(backend.start(core)?);
        Ok(engine)
    }

    /// Sends a new plan to the audio thread, which introduces it with a 20 ms fade.
    pub fn swap_plan(&mut self, plan: Plan) -> Result<()> {
        self.to_audio.push(Msg::Plan(Box::new(plan))).map_err(|_| anyhow::anyhow!("queue towards the audio thread is full"))
    }

    /// Changes a parameter of the most recent plan (indices from `NodeMap::resolve`).
    #[allow(dead_code)] // the integration (I2) will use it
    pub fn set_param(&mut self, change: ParamChange) -> Result<()> {
        self.to_audio.push(Msg::Param(change)).map_err(|_| anyhow::anyhow!("queue towards the audio thread is full"))
    }

    /// Frees the retired plans returned by the audio thread. Must be called regularly.
    pub fn collect_garbage(&mut self) {
        while self.from_audio.pop().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{compile, load::parse_graph};

    const SR: f32 = 48000.0;

    fn plan(db: f32) -> Plan {
        let g = parse_graph(&format!("name = \"p\"\n[[block]]\ntype = \"gain\"\ndb = {db}\n")).unwrap();
        compile(&g, &[], SR).unwrap().0
    }

    fn runner(first: Plan) -> (Runner, Producer<Msg>, Consumer<Box<Plan>>) {
        let (tx, rx) = RingBuffer::new(16);
        let (trash, from_audio) = RingBuffer::new(16);
        (Runner::new(Box::new(first), SR, rx, trash), tx, from_audio)
    }

    fn sine(range: std::ops::Range<usize>) -> Vec<f32> {
        range.map(|i| 0.8 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    #[test]
    fn crossfade_produces_no_jumps() {
        let (mut r, mut tx, mut garbage) = runner(plan(0.0));
        let mut all = Vec::new();
        for b in 0..200 {
            if b == 50 {
                tx.push(Msg::Plan(Box::new(plan(-20.0)))).ok().unwrap();
            }
            let input = sine(b * 64..(b + 1) * 64);
            let mut out = vec![0.0; 64];
            r.run(&input, &mut out);
            all.extend(out);
        }
        let max_jump = all.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(max_jump < 0.1, "jump of {max_jump}");
        // Before the fade: 0 dB; after it: -20 dB.
        let expected = 0.8 * crate::dsp::db_to_lin(-20.0);
        assert!((peak(&all[all.len() - 200..]) - expected).abs() < 0.01);
        // The old plan is sent back to be freed outside the audio thread.
        assert!(garbage.pop().is_ok());
    }

    #[test]
    fn changes_arrive_in_order_relative_to_plans() {
        let (mut r, mut tx, _g) = runner(plan(0.0));
        let g = parse_graph("name = \"p\"\n[[block]]\ntype = \"gain\"\n").unwrap();
        let (new, map) = compile(&g, &[], SR).unwrap();
        tx.push(Msg::Plan(Box::new(new))).ok().unwrap();
        // Right after the plan: it goes to the new plan, even if the fade is in progress.
        let c = map.resolve("gain1", "db", -40.0).unwrap().unwrap();
        tx.push(Msg::Param(c)).ok().unwrap();
        let mut out = vec![0.0; 64];
        for b in 0..100 {
            r.run(&sine(b * 64..(b + 1) * 64), &mut out);
        }
        assert!(peak(&out) < 0.8 * crate::dsp::db_to_lin(-30.0));
    }
}
