//! Motore audio: un client JACK (servito da PipeWire su Linux) con un ingresso
//! mono e due uscite. Il callback realtime esegue il piano (grafo compilato)
//! corrente. Piani nuovi e cambi di parametro arrivano da un'unica coda
//! lock-free, quindi in ordine; i piani dismessi tornano indietro per essere
//! deallocati fuori dal thread audio.
//!
//! Un piano nuovo entra con una dissolvenza incrociata di 20 ms: per questo
//! tempo girano entrambi i piani.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, Port, PortFlags, ProcessScope};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::graph::exec::MAX_BLOCK;
use crate::graph::{ParamChange, Plan};

/// Durata della dissolvenza tra due piani.
const FADE_SECONDS: f32 = 0.02;

/// Valori misurati nel thread audio e letti dall'interfaccia.
#[derive(Default)]
pub struct Meters {
    in_peak: AtomicU32,
    out_peak: AtomicU32,
    /// Carico massimo del callback, in millesimi del tempo disponibile.
    load_permille: AtomicU32,
    pub xruns: AtomicUsize,
    /// Dimensione del buffer dell'ultimo ciclo: PipeWire può cambiarla al volo.
    pub buffer_size: AtomicU32,
}

impl Meters {
    /// Restituisce (picco ingresso, picco uscita, carico 0..1) e azzera.
    pub fn take(&self) -> (f32, f32, f32) {
        (
            f32::from_bits(self.in_peak.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.out_peak.swap(0, Ordering::Relaxed)),
            self.load_permille.swap(0, Ordering::Relaxed) as f32 / 1000.0,
        )
    }
}

fn store_max(cell: &AtomicU32, v: f32) {
    // Vale solo per f32 positivi, per i quali l'ordine dei bit coincide.
    cell.fetch_max(v.to_bits(), Ordering::Relaxed);
}


/// Messaggio verso il thread audio. L'ordine di arrivo conta: un cambio di parametro
/// riguarda il piano che era l'ultimo arrivato quando è stato mandato.
enum Msg {
    Plan(Box<Plan>),
    #[allow(dead_code)] // inviato da `Engine::set_param`, che userà l'integrazione (I2)
    Param(ParamChange),
}

struct Fade {
    old: Box<Plan>,
    /// Campioni di dissolvenza già eseguiti.
    pos: usize,
}

/// Esegue il piano corrente e gestisce la dissolvenza. Non dipende da JACK, quindi si prova nei test.
struct Runner {
    plan: Box<Plan>,
    fade: Option<Fade>,
    fade_len: usize,
    /// Uscita del piano vecchio durante la dissolvenza (preallocata).
    scratch: Vec<f32>,
    incoming: Consumer<Msg>,
    trash: Producer<Box<Plan>>,
}

impl Runner {
    fn new(plan: Box<Plan>, sample_rate: f32, incoming: Consumer<Msg>, trash: Producer<Box<Plan>>) -> Runner {
        let fade_len = ((FADE_SECONDS * sample_rate) as usize).max(1);
        Runner { plan, fade: None, fade_len, scratch: vec![0.0; MAX_BLOCK], incoming, trash }
    }

    /// Legge i messaggi. Un piano nuovo aspetta la fine della dissolvenza in corso
    /// (e con lui i messaggi che lo seguono).
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
                    // Se la coda fosse piena il piano verrebbe liberato qui: non succede,
                    // perché il thread principale la svuota di continuo.
                    let _ = self.trash.push(done.old);
                }
            }
        }
    }
}

struct Processor {
    input: Port<AudioIn>,
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    runner: Runner,
    meters: Arc<Meters>,
    sample_rate: f32,
    /// Registrazione opzionale: campioni interleaved (pulito, elaborato).
    record: Option<Producer<f32>>,
}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        let start = Instant::now();

        let input = self.input.as_slice(ps);
        let out = self.out_l.as_mut_slice(ps);
        store_max(&self.meters.in_peak, input.iter().fold(0.0f32, |m, s| m.max(s.abs())));

        self.runner.run(input, out);

        // Protezione per orecchie e casse: niente NaN e niente oltre 0 dBFS.
        let mut peak = 0.0f32;
        for s in out.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
            *s = s.clamp(-1.0, 1.0);
            peak = peak.max(s.abs());
        }
        store_max(&self.meters.out_peak, peak);
        self.out_r.as_mut_slice(ps).copy_from_slice(out);

        if let Some(rec) = &mut self.record {
            // Se il thread di scrittura non tiene il passo si perdono campioni, mai il tempo reale.
            for (dry, wet) in input.iter().zip(out.iter()) {
                let _ = rec.push(*dry);
                let _ = rec.push(*wet);
            }
        }

        self.meters.buffer_size.store(ps.n_frames(), Ordering::Relaxed);
        let budget = ps.n_frames() as f32 / self.sample_rate;
        let load = start.elapsed().as_secs_f32() / budget;
        self.meters.load_permille.fetch_max((load * 1000.0) as u32, Ordering::Relaxed);
        Control::Continue
    }
}

struct Notifications {
    meters: Arc<Meters>,
}

impl jack::NotificationHandler for Notifications {
    fn xrun(&mut self, _: &Client) -> Control {
        self.meters.xruns.fetch_add(1, Ordering::Relaxed);
        Control::Continue
    }
}

/// Le porte JACK delle schede audio, filtrate per direzione.
pub fn hardware_ports(client: &Client, capture: bool) -> Vec<String> {
    // Visto da JACK, ciò che la scheda "cattura" è una porta di uscita.
    let flags = if capture { PortFlags::IS_OUTPUT } else { PortFlags::IS_INPUT };
    client.ports(None, Some("32 bit float mono audio"), flags | PortFlags::IS_PHYSICAL)
}

pub fn open_client(name: &str) -> Result<Client> {
    let (client, _status) = Client::new(name, ClientOptions::NO_START_SERVER)
        .context("impossibile collegarsi a JACK/PipeWire (sei dentro `nix develop`?)")?;
    Ok(client)
}

pub struct Engine {
    _active: jack::AsyncClient<Notifications, Processor>,
    to_audio: Producer<Msg>,
    from_audio: Consumer<Box<Plan>>,
    pub meters: Arc<Meters>,
    pub sample_rate: f32,
}

impl Engine {
    /// Avvia il motore e collega le porte che corrispondono alle regex.
    pub fn start(
        client: Client,
        plan: Plan,
        input_pattern: &str,
        output_pattern: &str,
        record: Option<Producer<f32>>,
    ) -> Result<Engine> {
        let sample_rate = client.sample_rate() as f32;
        let input = client.register_port("in", AudioIn::default())?;
        let out_l = client.register_port("out_L", AudioOut::default())?;
        let out_r = client.register_port("out_R", AudioOut::default())?;
        let my_in = input.name()?;
        let my_out = [out_l.name()?, out_r.name()?];

        let sources = client.ports(Some(input_pattern), None, PortFlags::IS_OUTPUT);
        let Some(source) = sources.first().cloned() else {
            bail!("nessuna porta di ingresso corrisponde a '{input_pattern}' (prova `openpedal ports`)");
        };
        let sinks = client.ports(Some(output_pattern), None, PortFlags::IS_INPUT);
        if sinks.is_empty() {
            bail!("nessuna porta di uscita corrisponde a '{output_pattern}' (prova `openpedal ports`)");
        }

        let (to_audio, incoming) = RingBuffer::new(1024);
        let (trash, from_audio) = RingBuffer::new(64);
        let meters = Arc::new(Meters::default());

        let processor = Processor {
            input,
            out_l,
            out_r,
            runner: Runner::new(Box::new(plan), sample_rate, incoming, trash),
            meters: meters.clone(),
            sample_rate,
            record,
        };
        let active = client.activate_async(Notifications { meters: meters.clone() }, processor)?;

        let c = active.as_client();
        c.connect_ports_by_name(&source, &my_in)
            .with_context(|| format!("collegamento {source} -> {my_in}"))?;
        // Una sola uscita trovata (es. casse mono): ci mandiamo il canale sinistro.
        for (i, sink) in sinks.iter().take(2).enumerate() {
            let from = &my_out[i.min(1)];
            c.connect_ports_by_name(from, sink)
                .with_context(|| format!("collegamento {from} -> {sink}"))?;
        }
        eprintln!("  ingresso: {source}");
        for sink in sinks.iter().take(2) {
            eprintln!("  uscita:   {sink}");
        }

        Ok(Engine { _active: active, to_audio, from_audio, meters, sample_rate })
    }

    /// Manda un nuovo piano al thread audio, che lo introduce con una dissolvenza di 20 ms.
    pub fn swap_plan(&mut self, plan: Plan) -> Result<()> {
        self.to_audio.push(Msg::Plan(Box::new(plan))).map_err(|_| anyhow::anyhow!("coda verso il thread audio piena"))
    }

    /// Cambia un parametro del piano più recente (indici da `NodeMap::resolve`).
    #[allow(dead_code)] // lo userà l'integrazione (I2)
    pub fn set_param(&mut self, change: ParamChange) -> Result<()> {
        self.to_audio.push(Msg::Param(change)).map_err(|_| anyhow::anyhow!("coda verso il thread audio piena"))
    }

    /// Libera i piani dismessi dal thread audio. Va chiamato regolarmente.
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
    fn la_dissolvenza_non_produce_salti() {
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
        assert!(max_jump < 0.1, "salto di {max_jump}");
        // Prima 0 dB, dopo la dissolvenza -20 dB.
        let expected = 0.8 * crate::dsp::db_to_lin(-20.0);
        assert!((peak(&all[all.len() - 200..]) - expected).abs() < 0.01);
        // Il piano vecchio torna indietro per essere liberato fuori dal thread audio.
        assert!(garbage.pop().is_ok());
    }

    #[test]
    fn i_cambi_arrivano_in_ordine_rispetto_ai_piani() {
        let (mut r, mut tx, _g) = runner(plan(0.0));
        let g = parse_graph("name = \"p\"\n[[block]]\ntype = \"gain\"\n").unwrap();
        let (new, map) = compile(&g, &[], SR).unwrap();
        tx.push(Msg::Plan(Box::new(new))).ok().unwrap();
        // Subito dopo il piano: va al piano nuovo, anche se la dissolvenza è in corso.
        let c = map.resolve("gain1", "db", -40.0).unwrap().unwrap();
        tx.push(Msg::Param(c)).ok().unwrap();
        let mut out = vec![0.0; 64];
        for b in 0..100 {
            r.run(&sine(b * 64..(b + 1) * 64), &mut out);
        }
        assert!(peak(&out) < 0.8 * crate::dsp::db_to_lin(-30.0));
    }
}
