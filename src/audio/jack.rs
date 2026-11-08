//! JACK backend (PipeWire on Linux): one mono input and two outputs, connected to
//! the ports whose names match two regexes.

use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use jack::{AudioIn, AudioOut, Client, ClientOptions, Control, Port, PortFlags, ProcessScope};

use super::AudioBackend;
use crate::engine::{AudioCore, Meters};

struct Processor {
    input: Port<AudioIn>,
    out_l: Port<AudioOut>,
    out_r: Port<AudioOut>,
    core: AudioCore,
}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        self.core.process(self.input.as_slice(ps), self.out_l.as_mut_slice(ps), self.out_r.as_mut_slice(ps));
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

/// The JACK ports of the audio cards, filtered by direction.
pub fn hardware_ports(client: &Client, capture: bool) -> Vec<String> {
    // Seen from JACK, what the card "captures" is an output port.
    let flags = if capture { PortFlags::IS_OUTPUT } else { PortFlags::IS_INPUT };
    client.ports(None, Some("32 bit float mono audio"), flags | PortFlags::IS_PHYSICAL)
}

pub fn open_client(name: &str) -> Result<Client> {
    let (client, _status) = Client::new(name, ClientOptions::NO_START_SERVER)
        .context("cannot connect to JACK/PipeWire (are you inside `nix develop`?)")?;
    Ok(client)
}

pub struct JackBackend {
    client: Client,
    input_pattern: String,
    output_pattern: String,
}

impl JackBackend {
    pub fn new(client: Client, input_pattern: &str, output_pattern: &str) -> Self {
        Self { client, input_pattern: input_pattern.into(), output_pattern: output_pattern.into() }
    }
}

impl AudioBackend for JackBackend {
    fn sample_rate(&self) -> f32 {
        self.client.sample_rate() as f32
    }

    /// Registers the ports, connects the ones matching the regexes and activates the client.
    fn start(self: Box<Self>, core: AudioCore) -> Result<Box<dyn Any + Send>> {
        let Self { client, input_pattern, output_pattern } = *self;
        let input = client.register_port("in", AudioIn::default())?;
        let out_l = client.register_port("out_L", AudioOut::default())?;
        let out_r = client.register_port("out_R", AudioOut::default())?;
        let my_in = input.name()?;
        let my_out = [out_l.name()?, out_r.name()?];

        let sources = client.ports(Some(&input_pattern), None, PortFlags::IS_OUTPUT);
        let Some(source) = sources.first().cloned() else {
            bail!("no input port matches '{input_pattern}' (try `openpedal ports`)");
        };
        let sinks = client.ports(Some(&output_pattern), None, PortFlags::IS_INPUT);
        if sinks.is_empty() {
            bail!("no output port matches '{output_pattern}' (try `openpedal ports`)");
        }

        let meters = core.meters().clone();
        let active = client.activate_async(Notifications { meters }, Processor { input, out_l, out_r, core })?;

        let c = active.as_client();
        c.connect_ports_by_name(&source, &my_in).with_context(|| format!("connecting {source} -> {my_in}"))?;
        // Only one output found (e.g. mono speakers): we send it the left channel.
        for (i, sink) in sinks.iter().take(2).enumerate() {
            let from = &my_out[i.min(1)];
            c.connect_ports_by_name(from, sink).with_context(|| format!("connecting {from} -> {sink}"))?;
        }
        eprintln!("  input:  {source}");
        for sink in sinks.iter().take(2) {
            eprintln!("  output: {sink}");
        }
        Ok(Box::new(active))
    }
}
