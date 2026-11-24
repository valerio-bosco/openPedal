
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};
use std::{fs, thread};

use anyhow::{Context, Result};
use serde::Deserialize;

use openpedal_engine::audio::AudioBackend;
#[cfg(target_os = "linux")]
use openpedal_engine::audio::jack;
use openpedal_dsp::lin_to_db;
use openpedal_engine::engine::Engine;
use openpedal_graph::model::GraphFile;
use openpedal_library::Library;
use openpedal_graph::{self as graph, NodeMap, Plan};
use openpedal_dsp::wav;
use openpedal_engine::live;
use openpedal_server as server;

const USAGE: &str = "\
openPedal - real-time guitar effects

usage:
  openpedal [PRESET] [--in REGEX] [--out REGEX] [--buffer N]
                                                  play with a preset (default: content/presets/clean.toml)
  openpedal ui [--port 7777] [--no-browser] [--lan]
                                                  web interface (node editor) in the browser
  openpedal ports                                 list the audio ports of the sound cards
  openpedal check PRESET...                       check presets without opening the audio
  openpedal render PRESET IN.wav OUT.wav          apply a preset to a recording (channel 1 = dry)
  openpedal tone-match JOB-DIR --part ID [--di FILE] [--validate-di FILE] [--templates DIR] [--template NAME] [--budget N]
                                                  fit a chain template to a part of a Tone Lab job

recording options (while playing):
  --record FILE.wav    save dry (left) and processed (right)
  --seconds N          recording length (default 20)

The preset is reloaded automatically every time you save the file.
Input and output are set in openpedal.toml or with --in / --out.";

/// Device settings (openpedal.toml).
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    /// Regex matching the name of the port the guitar comes from.
    input: String,
    /// Regex matching the output ports (the first two are used: left and right).
    output: String,
    /// Samples per cycle requested from PipeWire: fewer = less latency, but more risk of crackling.
    buffer: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { input: "Scarlett.*capture_AUX1".into(), output: "Scarlett.*playback".into(), buffer: 256 }
    }
}

fn load_config() -> Result<Config> {
    match fs::read_to_string("openpedal.toml") {
        Ok(text) => toml::from_str(&text).context("error in openpedal.toml"),
        Err(_) => Ok(Config::default()),
    }
}

/// The content folder (`presets/` and `pedals/`).
const CONTENT_DIR: &str = "content";

fn library() -> Library {
    Library::new(CONTENT_DIR)
}

/// Folder of composite pedals.
const PEDALS_DIR: &str = "content/pedals";

/// Reads a preset (v1 or v2) and the pedals, and compiles the graph.
fn load_plan(path: &Path, sr: f32) -> Result<(GraphFile, Plan, NodeMap)> {
    let graph = graph::load::read_graph(path)?;
    let pedals = library().load_pedals()?;
    let (plan, map) = graph::compile(&graph, &pedals, sr).with_context(|| format!("preset {}", path.display()))?;
    Ok((graph, plan, map))
}

/// Latest modification time between the preset and the pedals: used to reload when either changes.
fn watch_stamp(preset: &Path) -> (Option<SystemTime>, Option<SystemTime>) {
    let pedals = fs::read_dir(PEDALS_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| modified(&e.path()))
        .max();
    (modified(preset), pedals)
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nerror: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    let mut config = load_config()?;
    let mut preset_path = PathBuf::from("content/presets/clean.toml");
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
            "ui" => return run_ui(args.collect()),
            "dev-fixtures" => return write_fixtures(&PathBuf::from(args.next().context("usage: dev-fixtures DIR")?)),
            "check" => return check_presets(args.map(PathBuf::from).collect()),
            "--in" => config.input = args.next().context("--in needs a regex")?,
            "--out" => config.output = args.next().context("--out needs a regex")?,
            "--record" => record = Some(PathBuf::from(args.next().context("--record needs a file")?)),
            "--buffer" => {
                config.buffer = args.next().context("--buffer needs a number")?.parse().context("--buffer needs a number")?
            }
            "--seconds" => {
                seconds = args.next().context("--seconds needs a number")?.parse().context("--seconds needs a number")?
            }
            "tone-match" => return tone_match(args.collect()),
            "render" => {
                let mut next = || args.next().map(PathBuf::from).context("usage: render PRESET IN.wav OUT.wav");
                return render(&next()?, &next()?, &next()?);
            }
            _ => preset_path = PathBuf::from(arg),
        }
    }

    play(&config, &preset_path, record.map(|path| (path, seconds)))
}

/// `openpedal ui`: web interface server (for now with the fake engine).
fn run_ui(args: Vec<String>) -> Result<()> {
    let mut port = 7777u16;
    let mut open_browser = true;
    let mut lan = false;
    let mut audio = true;
    let mut preset_path = PathBuf::from("content/presets/clean.toml");
    let mut config = load_config()?;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().context("--port needs a number")?.parse().context("--port needs a number")?,
            "--no-browser" => open_browser = false,
            "--lan" => lan = true,
            "--no-audio" => audio = false,
            "--in" => config.input = args.next().context("--in needs a regex")?,
            "--out" => config.output = args.next().context("--out needs a regex")?,
            "--buffer" => {
                config.buffer = args.next().context("--buffer needs a number")?.parse().context("--buffer needs a number")?
            }
            a if a.starts_with("--") => anyhow::bail!("unknown option for ui: {arg}"),
            _ => preset_path = PathBuf::from(arg),
        }
    }
    let library = library();
    let pedals_dir = library.pedals_dir();
    let graph = match graph::load::read_graph(&preset_path) {
        Ok(mut g) => {
            // as in load_preset: the name is the file's name, so the interface recognizes it
            if let Some(stem) = preset_path.file_stem().and_then(|s| s.to_str()) {
                g.name = stem.to_string();
            }
            g
        }
        Err(e) => {
            eprintln!("{e:#}: starting from an empty graph");
            server::mock::default_graph()
        }
    };

    let engine: std::sync::Arc<dyn server::EngineHandle> = if audio {
        match start_live(&config, graph.clone(), &library) {
            Ok(live) => live,
            Err(e) => {
                eprintln!("audio unavailable ({e:#}): editor only, no sound");
                std::sync::Arc::new(server::mock::MockEngine::new(&pedals_dir, graph))
            }
        }
    } else {
        std::sync::Arc::new(server::mock::MockEngine::new(&pedals_dir, graph))
    };
    server::run(engine, server::ServerOptions { port, open_browser, lan, library })
}

/// Starts the real audio engine with the initial graph.
fn start_live(config: &Config, graph: GraphFile, library: &Library) -> Result<std::sync::Arc<live::LiveEngine>> {
    let backend = open_backend(config)?;
    let sr = backend.sample_rate();
    let pedals = library.load_pedals()?;
    let (plan, map) = graph::compile(&graph, &pedals, sr)?;
    let engine = Engine::start(backend, plan, None)?;
    eprintln!("  {} Hz, preset \"{}\"", engine.sample_rate, graph.name);
    Ok(live::LiveEngine::new(engine, map, graph, pedals, library.pedals_dir()))
}

/// The catalog in JSON, as the interface receives it.
fn print_catalog() -> Result<()> {
    let pedals = library().load_pedals()?;
    println!("{}", serde_json::to_string_pretty(&graph::model::catalog_items(&pedals))?);
    Ok(())
}

/// Generates the protocol's JSON examples for developing the interface.
fn write_fixtures(dir: &Path) -> Result<()> {
    use openpedal_protocol::{AudioInfo, ClientMsg, MetersMsg, ServerMsg, StateMsg};

    fs::create_dir_all(dir)?;
    let pedals = library().load_pedals()?;
    let catalog = graph::model::catalog_items(&pedals);
    let graph = graph::load::read_graph(Path::new("docs/examples/crunch-graph.toml"))?;
    let write = |name: &str, json: String| -> Result<()> {
        fs::write(dir.join(name), json + "\n").with_context(|| format!("writing {name}"))
    };

    write("catalog.json", serde_json::to_string_pretty(&catalog)?)?;
    let state = ServerMsg::State(StateMsg {
        protocol: openpedal_protocol::PROTOCOL_VERSION,
        catalog,
        graph: graph.clone(),
        presets: vec!["ambient".into(), "clean".into(), "crunch".into(), "crunch-graph".into()],
        pedals: pedals.iter().cloned().collect(),
        audio: AudioInfo { sample_rate: 48000, buffer: 96, running: true },
        can_undo: true,
        can_redo: false,
        dirty: true,
    });
    write("state.json", serde_json::to_string_pretty(&state)?)?;
    let meters = ServerMsg::Meters(MetersMsg { in_db: -18.5, out_db: -12.0, dsp: 0.02, xruns: 0 });
    write("meters.json", serde_json::to_string_pretty(&meters)?)?;
    let error = ServerMsg::Error { message: "the graph contains a cycle: dist -> room -> dist".into(), node: Some("room".into()) };
    write("error.json", serde_json::to_string_pretty(&error)?)?;
    write("param.json", serde_json::to_string_pretty(&ServerMsg::Param { node: "dist".into(), param: "gain_db".into(), value: 18.0 })?)?;
    // A short synthetic example (a real response has about 128 points).
    let response = ServerMsg::Response {
        freq: vec![20.0, 100.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 20000.0],
        db: vec![-18.5, -3.2, 4.1, 5.0, 3.4, -1.8, -9.6, -22.0],
    };
    write("response.json", serde_json::to_string_pretty(&response)?)?;
    write("added.json", serde_json::to_string_pretty(&ServerMsg::Added { node: "delay2".into() })?)?;

    // Tone Lab (protocol v4): the same fake job the mock server answers with.
    use server::tonelab_mock::{demo_fit, demo_job, demo_templates, DEMO_JOB};
    use openpedal_protocol::{TonelabJobSummary, TonelabStage};
    let job = demo_job();
    let tonelab: Vec<(&str, ServerMsg)> = vec![
        ("tonelab_caps.json", ServerMsg::TonelabCaps { available: true, youtube: true, device: "cuda".into(), reason: None }),
        ("tonelab_caps_unavailable.json", ServerMsg::TonelabCaps { available: false, youtube: false, device: String::new(), reason: Some("the tonelab tool was not found at tools/tonelab/bin/tonelab (run openpedal from the repository root, or set OPENPEDAL_TONELAB)".into()) }),
        ("tonelab_jobs.json", ServerMsg::TonelabJobs { jobs: vec![TonelabJobSummary { id: job.id.clone(), title: job.title.clone(), created: "2026-11-24T10:00:00+01:00".into(), duration_s: job.duration_s }] }),
        ("tonelab_progress.json", ServerMsg::TonelabProgress { job: DEMO_JOB.into(), stage: TonelabStage::Separate, message: "separating the stems (a minute or two on the GPU)".into(), fraction: None }),
        ("tonelab_progress_fit.json", ServerMsg::TonelabProgress { job: DEMO_JOB.into(), stage: TonelabStage::Fit, message: "  screen fuzz            cost 2.31 (84 renders)".into(), fraction: Some(0.25) }),
        ("tonelab_job.json", ServerMsg::TonelabJob { job }),
        ("tonelab_fit_result.json", ServerMsg::TonelabFitResult { job: DEMO_JOB.into(), part: "p1".into(), fit: demo_fit(), templates: demo_templates() }),
    ];
    for (name, msg) in tonelab {
        write(name, serde_json::to_string_pretty(&msg)?)?;
    }

    let (_, pedal) = pedals.iter().find(|(id, _)| id == "cab-4x12").context("need content/pedals/cab-4x12.toml")?.clone();
    let (_, custom) = pedals.iter().find(|(id, _)| id == "grit-box").context("need content/pedals/grit-box.toml")?.clone();
    let client = vec![
        ClientMsg::Hello,
        ClientMsg::SetParam { node: "dist".into(), param: "gain_db".into(), value: 18.0 },
        ClientMsg::SetParam { node: "dist".into(), param: "shape".into(), value: 3.0 },
        ClientMsg::SetBypass { node: "dist".into(), bypass: true },
        ClientMsg::SetGraph { graph: graph.clone() },
        ClientMsg::LoadPreset { name: "clean".into() },
        ClientMsg::SavePreset { name: "my-sound".into(), graph },
        ClientMsg::SavePedal { id: "cab-4x12".into(), pedal },
        ClientMsg::SavePedal { id: "grit-box".into(), pedal: custom.clone() },
        ClientMsg::DeletePreset { name: "old-sound".into() },
        ClientMsg::DeletePedal { id: "old-pedal".into() },
        ClientMsg::AddNode { kind: "delay".into(), after: Some("dist".into()) },
        ClientMsg::AddNode { kind: "pedal:cab-4x12".into(), after: None },
        ClientMsg::RemoveNode { node: "dist".into() },
        ClientMsg::MoveNode { node: "dist".into(), to: 0 },
        ClientMsg::Undo,
        ClientMsg::Redo,
        ClientMsg::AnalyzePedal { pedal: custom, values: [("grit".to_string(), 0.8), ("voice".to_string(), 1.0), ("boost".to_string(), 1.0)].into() },
        ClientMsg::AnalyzeNode { node: "dist".into() },
        ClientMsg::EditBegin,
        ClientMsg::TonelabCaps,
        ClientMsg::TonelabJobs,
        ClientMsg::TonelabStart { source: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into() },
        ClientMsg::TonelabOpen { job: DEMO_JOB.into() },
        ClientMsg::TonelabPick { job: DEMO_JOB.into(), start: 8.0, end: 28.0, channel: openpedal_protocol::TonelabChannel::Mid, label: "riff".into() },
        ClientMsg::TonelabRecordDi { job: DEMO_JOB.into(), part: "p1".into(), seconds: 20.0 },
        ClientMsg::TonelabFit { job: DEMO_JOB.into(), part: "p1".into() },
        ClientMsg::TonelabSavePreset { job: DEMO_JOB.into(), part: "p1".into(), name: "cryogen-riff".into() },
        ClientMsg::TonelabCancel { job: DEMO_JOB.into() },
    ];
    write("client-messages.json", serde_json::to_string_pretty(&client)?)?;
    println!("fixtures written to {}", dir.display());
    Ok(())
}

/// Opens the platform's audio backend (JACK/PipeWire on Linux).
#[cfg(target_os = "linux")]
fn open_backend(config: &Config) -> Result<Box<dyn AudioBackend>> {
    // PipeWire reads this variable when the client connects.
    std::env::set_var("PIPEWIRE_QUANTUM", format!("{}/48000", config.buffer));
    let client = jack::open_client("openPedal")?;
    Ok(Box::new(jack::JackBackend::new(client, &config.input, &config.output)))
}

#[cfg(not(target_os = "linux"))]
fn open_backend(_: &Config) -> Result<Box<dyn AudioBackend>> {
    anyhow::bail!("this OS has no audio backend yet (Windows/macOS: task E1)")
}

#[cfg(target_os = "linux")]
fn list_ports() -> Result<()> {
    let client = jack::open_client("openPedal-ports")?;
    println!("inputs (use with --in):");
    for p in jack::hardware_ports(&client, true) {
        println!("  {p}");
    }
    println!("outputs (use with --out):");
    for p in jack::hardware_ports(&client, false) {
        println!("  {p}");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn list_ports() -> Result<()> {
    anyhow::bail!("this OS has no audio backend yet (Windows/macOS: task E1)")
}

fn check_presets(paths: Vec<PathBuf>) -> Result<()> {
    let mut ok = true;
    for path in &paths {
        match load_plan(path, 48000.0) {
            Ok((g, _, _)) => println!("ok  {} \"{}\" ({} nodes)", path.display(), g.name, g.nodes.len()),
            Err(e) => {
                println!("ERR {}: {e:#}", path.display());
                ok = false;
            }
        }
    }
    anyhow::ensure!(ok, "some presets are not valid");
    Ok(())
}

/// `openpedal tone-match`: fits chain templates to a part of a Tone Lab job folder.
fn tone_match(args: Vec<String>) -> Result<()> {
    use openpedal_tonematch::{fit_job, FitOptions};
    let usage = "usage: tone-match JOB-DIR --part ID [--di FILE] [--validate-di FILE] [--templates DIR] [--template NAME] [--budget N]";
    let mut opts = FitOptions::default();
    let (mut job, mut part) = (None, None);
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |what: &str| it.next().with_context(|| format!("{what} needs a value"));
        match arg.as_str() {
            "--part" => part = Some(value("--part")?),
            "--di" => opts.di = Some(PathBuf::from(value("--di")?)),
            "--validate-di" => opts.validate_di = Some(PathBuf::from(value("--validate-di")?)),
            "--templates" => opts.templates_dir = PathBuf::from(value("--templates")?),
            "--template" => opts.template = Some(value("--template")?),
            "--budget" => opts.budget = value("--budget")?.parse().context("--budget needs a number")?,
            _ if arg.starts_with("--") => anyhow::bail!("unknown option {arg}\n{usage}"),
            _ => job = Some(PathBuf::from(arg)),
        }
    }
    let (job, part) = (job.context(usage)?, part.context(usage)?);
    openpedal_tonematch::lower_priority();
    let summary = fit_job(&job, &part, &opts, &|line| println!("{line}"))?;
    println!("\n{:<16} {:>9} {:>9} {:>8}  stage", "template", "error dB", "holdout", "renders");
    for r in &summary.rows {
        let hold = r.holdout_error_db.map(|h| format!("{h:.2}")).unwrap_or_else(|| "-".into());
        println!("{:<16} {:>9.2} {:>9} {:>8}  {}", r.template, r.error_db, hold, r.renders, r.stage);
    }
    let r = &summary.report;
    println!(
        "\nbest: {} (cost {:.2}, error {:.2} dB{}{}) in {:.0} s, {} renders",
        r.template,
        summary.cost,
        r.error_db,
        r.holdout_error_db.map(|h| format!(", holdout {h:.2} dB")).unwrap_or_default(),
        r.validate_error_db.map(|v| format!(", validation {v:.2} dB")).unwrap_or_default(),
        r.seconds,
        r.renders
    );
    if let Some(dir) = &summary.out_dir {
        println!("written to {}", dir.display());
    }
    Ok(())
}

/// Processes the first channel of a WAV and saves dry (left) and processed (right).
fn render(preset_path: &Path, input: &Path, output: &Path) -> Result<()> {
    let data = wav::read(input)?;
    let (graph, mut plan, _) = load_plan(preset_path, data.sample_rate as f32)?;
    let dry = &data.channels[0];
    let mut wet = dry.clone();
    // In blocks of 64, as in live use, so the result is identical.
    for block in wet.chunks_mut(64) {
        plan.process(block);
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
    println!("\"{}\" -> {}", graph.name, output.display());
    Ok(())
}

/// Writes to disk the samples recorded by the audio thread, then closes the file.
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
            Ok(()) => eprintln!("\r\x1b[2Krecording saved: {}", path.display()),
            Err(e) => eprintln!("\r\x1b[2Kerror saving the recording: {e}"),
        }
        // `rx` is dropped here: the audio thread stops filling the queue.
    });
    Ok(())
}

fn play(config: &Config, preset_path: &Path, record: Option<(PathBuf, f32)>) -> Result<()> {
    let backend = open_backend(config)?;
    let sr = backend.sample_rate();
    let (graph, plan, _) = load_plan(preset_path, sr)?;

    eprintln!("openPedal");
    let recorder = match record {
        Some((path, seconds)) => {
            // 4 seconds of headroom: the disk may stall, the audio thread cannot.
            let (tx, rx) = rtrb::RingBuffer::new(sr as usize * 2 * 4);
            spawn_recorder(path.clone(), seconds, sr as u32, rx)?;
            eprintln!("  recording {seconds} s to {}", path.display());
            Some(tx)
        }
        None => None,
    };
    let mut engine = Engine::start(backend, plan, recorder)?;
    eprintln!("  {} Hz", engine.sample_rate);
    eprintln!("  preset: \"{}\" ({} nodes) - save the file to reload it, Ctrl+C to quit\n", graph.name, graph.nodes.len());

    let mut last_stamp = watch_stamp(preset_path);
    let mut tick = 0u32;
    loop {
        thread::sleep(Duration::from_millis(100));
        engine.collect_garbage();

        tick += 1;
        if tick.is_multiple_of(3) {
            let stamp = watch_stamp(preset_path);
            if stamp != last_stamp {
                last_stamp = stamp;
                match load_plan(preset_path, engine.sample_rate) {
                    Ok((g, plan, _)) => match engine.swap_plan(plan) {
                        Ok(()) => eprintln!("\r\x1b[2Kreloaded \"{}\" ({} nodes)", g.name, g.nodes.len()),
                        Err(e) => eprintln!("\r\x1b[2K{e:#} - the previous preset stays active"),
                    },
                    // A typo must not interrupt the sound: the old preset stays.
                    Err(e) => eprintln!("\r\x1b[2K{e:#} - the previous preset stays active"),
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

/// Bar from -60 to 0 dBFS; becomes "!" when it reaches clipping.
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
