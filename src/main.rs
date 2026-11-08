mod dsp;
mod engine;
mod graph;
mod server;
mod wav;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};
use std::{fs, thread};

use anyhow::{Context, Result};
use serde::Deserialize;

use dsp::{lin_to_db, Chain, Preset};
use engine::Engine;

const USAGE: &str = "\
openPedal - effetti per chitarra in tempo reale

uso:
  openpedal [PRESET] [--in REGEX] [--out REGEX] [--buffer N]
                                                  suona con un preset (default: presets/clean.toml)
  openpedal ports                                 elenca le porte audio delle schede
  openpedal check PRESET...                       controlla i preset senza aprire l'audio
  openpedal render PRESET IN.wav OUT.wav          applica un preset a una registrazione (canale 1 = pulito)

opzioni di registrazione (mentre suoni):
  --record FILE.wav    salva pulito (sinistra) ed elaborato (destra)
  --seconds N          durata della registrazione (default 20)

Il preset viene ricaricato automaticamente ogni volta che salvi il file.
Ingresso e uscita si impostano in openpedal.toml oppure con --in / --out.";

/// Impostazioni del dispositivo (openpedal.toml).
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    /// Regex sul nome della porta da cui arriva la chitarra.
    input: String,
    /// Regex sulle porte di uscita (si usano le prime due: sinistra e destra).
    output: String,
    /// Campioni per ciclo chiesti a PipeWire: meno = meno latenza, ma più rischio di crepitii.
    buffer: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { input: "Scarlett.*capture_AUX1".into(), output: "Scarlett.*playback".into(), buffer: 256 }
    }
}

fn load_config() -> Result<Config> {
    match fs::read_to_string("openpedal.toml") {
        Ok(text) => toml::from_str(&text).context("errore in openpedal.toml"),
        Err(_) => Ok(Config::default()),
    }
}

fn load_preset(path: &Path) -> Result<Preset> {
    let text = fs::read_to_string(path).with_context(|| format!("impossibile leggere {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("errore nel preset {}", path.display()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nerrore: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    let mut config = load_config()?;
    let mut preset_path = PathBuf::from("presets/clean.toml");
    let mut record = None;
    let mut seconds = 20.0f32;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "ports" => return list_ports(),
            "catalog" => return print_catalog(),
            "dev-fixtures" => return write_fixtures(&PathBuf::from(args.next().context("uso: dev-fixtures CARTELLA")?)),
            "check" => return check_presets(args.map(PathBuf::from).collect()),
            "--in" => config.input = args.next().context("--in vuole una regex")?,
            "--out" => config.output = args.next().context("--out vuole una regex")?,
            "--record" => record = Some(PathBuf::from(args.next().context("--record vuole un file")?)),
            "--buffer" => {
                config.buffer = args.next().context("--buffer vuole un numero")?.parse().context("--buffer vuole un numero")?
            }
            "--seconds" => {
                seconds = args.next().context("--seconds vuole un numero")?.parse().context("--seconds vuole un numero")?
            }
            "render" => {
                let mut next = || args.next().map(PathBuf::from).context("uso: render PRESET IN.wav OUT.wav");
                return render(&next()?, &next()?, &next()?);
            }
            _ => preset_path = PathBuf::from(arg),
        }
    }

    play(&config, &preset_path, record.map(|path| (path, seconds)))
}

/// Il catalogo in JSON, come lo riceve l'interfaccia.
fn print_catalog() -> Result<()> {
    let pedals = graph::load::read_pedals(Path::new("pedals"))?;
    println!("{}", serde_json::to_string_pretty(&graph::model::catalog_items(&pedals))?);
    Ok(())
}

/// Genera gli esempi JSON del protocollo per lo sviluppo dell'interfaccia.
fn write_fixtures(dir: &Path) -> Result<()> {
    use server::protocol::{AudioInfo, ClientMsg, MetersMsg, ServerMsg, StateMsg};

    fs::create_dir_all(dir)?;
    let pedals = graph::load::read_pedals(Path::new("pedals"))?;
    let catalog = graph::model::catalog_items(&pedals);
    let graph = graph::load::read_graph(Path::new("docs/esempi/crunch-grafo.toml"))?;
    let write = |name: &str, json: String| -> Result<()> {
        fs::write(dir.join(name), json + "\n").with_context(|| format!("scrittura di {name}"))
    };

    write("catalog.json", serde_json::to_string_pretty(&catalog)?)?;
    let state = ServerMsg::State(StateMsg {
        catalog,
        graph: graph.clone(),
        presets: vec!["ambient".into(), "clean".into(), "crunch".into(), "crunch-grafo".into()],
        audio: AudioInfo { sample_rate: 48000, buffer: 96, running: true },
    });
    write("state.json", serde_json::to_string_pretty(&state)?)?;
    let meters = ServerMsg::Meters(MetersMsg { in_db: -18.5, out_db: -12.0, dsp: 0.02, xruns: 0 });
    write("meters.json", serde_json::to_string_pretty(&meters)?)?;
    let error = ServerMsg::Error { message: "il grafo contiene un ciclo: dist -> room -> dist".into(), node: Some("room".into()) };
    write("error.json", serde_json::to_string_pretty(&error)?)?;

    let (_, pedal) = pedals.first().context("serve almeno un pedale in pedals/")?.clone();
    let client = vec![
        ClientMsg::Hello,
        ClientMsg::SetParam { node: "dist".into(), param: "gain_db".into(), value: 18.0 },
        ClientMsg::SetParam { node: "dist".into(), param: "shape".into(), value: 3.0 },
        ClientMsg::SetGraph { graph: graph.clone() },
        ClientMsg::LoadPreset { name: "clean".into() },
        ClientMsg::SavePreset { name: "mio-suono".into(), graph },
        ClientMsg::SavePedal { id: "cassa-4x12".into(), pedal },
        ClientMsg::DeletePedal { id: "vecchio-pedale".into() },
    ];
    write("client-messages.json", serde_json::to_string_pretty(&client)?)?;
    println!("fixture scritte in {}", dir.display());
    Ok(())
}

fn list_ports() -> Result<()> {
    let client = engine::open_client("openPedal-ports")?;
    println!("ingressi (da usare con --in):");
    for p in engine::hardware_ports(&client, true) {
        println!("  {p}");
    }
    println!("uscite (da usare con --out):");
    for p in engine::hardware_ports(&client, false) {
        println!("  {p}");
    }
    Ok(())
}

fn check_presets(paths: Vec<PathBuf>) -> Result<()> {
    let mut ok = true;
    for path in &paths {
        match load_preset(path) {
            Ok(p) => match Chain::build(&p, 48000.0) {
                Ok(_) => println!("ok  {} \"{}\" ({} blocchi)", path.display(), p.name, p.blocks.len()),
                Err(e) => {
                    println!("ERR {}: {e:#}", path.display());
                    ok = false;
                }
            },
            Err(e) => {
                println!("ERR {e:#}");
                ok = false;
            }
        }
    }
    anyhow::ensure!(ok, "alcuni preset non sono validi");
    Ok(())
}

/// Elabora il primo canale di un WAV e salva pulito (sinistra) ed elaborato (destra).
fn render(preset_path: &Path, input: &Path, output: &Path) -> Result<()> {
    let preset = load_preset(preset_path)?;
    let data = wav::read(input)?;
    let dry = &data.channels[0];
    let mut wet = dry.clone();
    let mut chain = Chain::build(&preset, data.sample_rate as f32)?;
    // A blocchi da 64 come dal vivo, così il risultato è identico.
    for block in wet.chunks_mut(64) {
        chain.process(block);
        for s in block.iter_mut() {
            *s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
        }
    }
    let mut writer = wav::WavWriter::create(output, 2, data.sample_rate)?;
    for (d, w) in dry.iter().zip(&wet) {
        writer.write(*d)?;
        writer.write(*w)?;
    }
    writer.finish()?;
    println!("\"{}\" -> {}", preset.name, output.display());
    Ok(())
}

/// Scrive su disco i campioni registrati dal thread audio, poi chiude il file.
fn spawn_recorder(path: PathBuf, seconds: f32, sample_rate: u32, mut rx: rtrb::Consumer<f32>) -> Result<()> {
    let mut writer = wav::WavWriter::create(&path, 2, sample_rate)?;
    let total = (seconds * sample_rate as f32) as usize * 2;
    thread::spawn(move || {
        let mut written = 0;
        while written < total {
            match rx.pop() {
                Ok(s) => {
                    if writer.write(s).is_err() {
                        break;
                    }
                    written += 1;
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        match writer.finish() {
            Ok(()) => eprintln!("\r\x1b[2Kregistrazione salvata: {}", path.display()),
            Err(e) => eprintln!("\r\x1b[2Kerrore nel salvare la registrazione: {e}"),
        }
        // Qui `rx` viene chiuso: il thread audio smette di riempire la coda.
    });
    Ok(())
}

fn play(config: &Config, preset_path: &Path, record: Option<(PathBuf, f32)>) -> Result<()> {
    let preset = load_preset(preset_path)?;
    // PipeWire legge questa variabile quando il client si collega.
    std::env::set_var("PIPEWIRE_QUANTUM", format!("{}/48000", config.buffer));
    let client = engine::open_client("openPedal")?;
    let sr = client.sample_rate() as f32;
    let chain = Chain::build(&preset, sr)?;

    eprintln!("openPedal");
    let recorder = match record {
        Some((path, seconds)) => {
            // 4 secondi di margine: il disco può avere pause, il thread audio no.
            let (tx, rx) = rtrb::RingBuffer::new(sr as usize * 2 * 4);
            spawn_recorder(path.clone(), seconds, sr as u32, rx)?;
            eprintln!("  registro {seconds} s in {}", path.display());
            Some(tx)
        }
        None => None,
    };
    let mut engine = Engine::start(client, chain, &config.input, &config.output, recorder)?;
    eprintln!("  {} Hz", engine.sample_rate);
    eprintln!("  preset: \"{}\" ({} blocchi) - salva il file per ricaricarlo, Ctrl+C per uscire\n", preset.name, preset.blocks.len());

    let mut last_mtime = modified(preset_path);
    let mut tick = 0u32;
    loop {
        thread::sleep(Duration::from_millis(100));
        engine.collect_garbage();

        tick += 1;
        if tick.is_multiple_of(3) {
            let mtime = modified(preset_path);
            if mtime != last_mtime {
                last_mtime = mtime;
                let built = load_preset(preset_path).and_then(|p| Ok((Chain::build(&p, engine.sample_rate)?, p)));
                match built {
                    Ok((chain, p)) => {
                        engine.swap_chain(chain);
                        eprintln!("\r\x1b[2Kricaricato \"{}\" ({} blocchi)", p.name, p.blocks.len());
                    }
                    // Un errore di battitura non deve interrompere il suono: resta il preset vecchio.
                    Err(e) => eprintln!("\r\x1b[2K{e:#} - resta attivo il preset precedente"),
                }
            }
        }

        let (inp, out, load) = engine.meters.take();
        let xruns = engine.meters.xruns.load(Ordering::Relaxed);
        let buffer = engine.meters.buffer_size.load(Ordering::Relaxed);
        eprint!(
            "\r\x1b[2KIN {} {:>6.1} dB   OUT {} {:>6.1} dB   DSP {:>3.0}%   buffer {} ({:.2} ms)   xrun {}",
            meter_bar(inp),
            lin_to_db(inp),
            meter_bar(out),
            lin_to_db(out),
            load * 100.0,
            buffer,
            buffer as f32 / engine.sample_rate * 1000.0,
            xruns
        );
    }
}

/// Barra da -60 a 0 dBFS; diventa "!" quando si arriva al clip.
fn meter_bar(peak: f32) -> String {
    const WIDTH: usize = 20;
    let db = lin_to_db(peak).clamp(-60.0, 0.0);
    let filled = (((db + 60.0) / 60.0) * WIDTH as f32).round() as usize;
    let mut bar: String = "█".repeat(filled) + &"·".repeat(WIDTH - filled);
    if peak >= 0.999 {
        bar.pop();
        bar.push('!');
    }
    bar
}
