//! Motore audio: un client JACK (servito da PipeWire su Linux) con un ingresso
//! mono e due uscite. Il callback realtime applica la catena corrente; le
//! catene nuove arrivano da una coda lock-free e quelle vecchie tornano indietro
//! per essere deallocate fuori dal thread audio.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, Port, PortFlags, ProcessScope};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::dsp::Chain;

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

struct Processor {
    input: Port<AudioIn>,
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    chain: Box<Chain>,
    incoming: Consumer<Box<Chain>>,
    trash: Producer<Box<Chain>>,
    meters: Arc<Meters>,
    sample_rate: f32,
    /// Registrazione opzionale: campioni interleaved (pulito, elaborato).
    record: Option<Producer<f32>>,
}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        let start = Instant::now();

        if let Ok(new) = self.incoming.pop() {
            let old = std::mem::replace(&mut self.chain, new);
            // Se la coda di ritorno fosse piena la catena verrebbe liberata qui:
            // non succede, perché il thread principale la svuota di continuo.
            let _ = self.trash.push(old);
        }

        let input = self.input.as_slice(ps);
        let out = self.out_l.as_mut_slice(ps);
        out.copy_from_slice(input);
        store_max(&self.meters.in_peak, input.iter().fold(0.0f32, |m, s| m.max(s.abs())));

        self.chain.process(out);

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
    to_audio: Producer<Box<Chain>>,
    from_audio: Consumer<Box<Chain>>,
    pub meters: Arc<Meters>,
    pub sample_rate: f32,
}

impl Engine {
    /// Avvia il motore e collega le porte che corrispondono alle regex.
    pub fn start(
        client: Client,
        chain: Chain,
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

        let (to_audio, incoming) = RingBuffer::new(8);
        let (trash, from_audio) = RingBuffer::new(8);
        let meters = Arc::new(Meters::default());

        let processor = Processor {
            input,
            out_l,
            out_r,
            chain: Box::new(chain),
            incoming,
            trash,
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

    /// Manda una nuova catena al thread audio.
    pub fn swap_chain(&mut self, chain: Chain) {
        if self.to_audio.push(Box::new(chain)).is_err() {
            eprintln!("coda piena, cambio preset ignorato");
        }
    }

    /// Libera le catene dismesse dal thread audio. Va chiamato regolarmente.
    pub fn collect_garbage(&mut self) {
        while self.from_audio.pop().is_ok() {}
    }
}
