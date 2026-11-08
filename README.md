# openPedal

Effetti per chitarra in tempo reale, senza DAW: chitarra → scheda audio → openPedal → cuffie/casse.
Gli effetti sono **preset in TOML** costruiti combinando blocchi base; il file si ricarica da solo ogni volta che lo salvi.

## Interfaccia a nodi

```sh
nix develop
(cd ui && npm install && npm run build)   # solo la prima volta o dopo modifiche a ui/
cargo run --release -- ui                 # apre il browser su http://127.0.0.1:7777
```

- Trascina i pedali dalla palette, collegali con i cavi da **Chitarra** ad **Ampli**: suona ciò che arriva all'Ampli.
- Le manopole agiscono dal vivo. Più cavi in un nodo si sommano: si possono fare catene parallele.
- **Crea pedale**: seleziona più nodi e salvali come pedale composto (`pedals/<nome>.toml`), con le manopole che scegli. Doppio clic su un pedale composto per modificarlo.
- Opzioni: `ui PRESET` (preset iniziale, default `presets/clean.toml`), `--lan` (raggiungibile dal telefono), `--port N`, `--no-browser`, `--no-audio` (solo editor), `--in`/`--out`/`--buffer` come sotto.
- Se la scheda audio non è disponibile, l'interfaccia parte comunque in modalità solo editor.
- Sviluppo dell'interfaccia: `cd ui && npm run dev` (con `openpedal ui --no-browser` acceso per l'audio, altrimenti usa dati di esempio).

## Avvio da riga di comando

```sh
nix develop                                  # Rust + libjack di PipeWire, buffer 64
cargo run --release                          # preset di default: presets/clean.toml
cargo run --release -- presets/crunch.toml
cargo run --release -- ports                 # nomi delle porte di schede e casse
cargo run --release -- check presets/*.toml  # controlla i preset senza audio
```

Prima di suonare: **spegni "Direct Monitor"** sulla Scarlett, altrimenti senti anche il segnale pulito.
Chitarra nel jack frontale con **INST** attivo; regola il gain finché `IN` arriva a circa -12 dB sulle note forti.

La riga di stato mostra i livelli di ingresso e uscita, il carico DSP, il buffer reale e gli xrun (i "buchi" audio: devono restare a 0).

## Dispositivi

`openpedal.toml` sceglie ingresso e uscita con regex sui nomi delle porte (vedi `ports`). Da riga di comando: `--in REGEX` / `--out REGEX`.

`buffer` (default 256 = 5,3 ms per ciclo) è quanti campioni elabora ogni ciclo: meno campioni = meno latenza, ma se la scheda non ce la fa l'ingresso **crepita**. Con la Scarlett Solo a 64 l'ingresso perde continuamente pezzi di segnale. `tools/prova-buffer.sh` registra con vari buffer per trovare il minimo pulito. Da riga di comando: `--buffer N`.

## Registrare e rielaborare

```sh
cargo run --release -- presets/clean.toml --record recordings/take.wav --seconds 20   # sinistra pulito, destra elaborato
cargo run --release -- render presets/crunch.toml recordings/take.wav recordings/take-crunch.wav
```
`render` applica un preset al canale pulito di una registrazione: si possono provare i preset senza suonare.

## Blocchi disponibili

| `type`    | Parametri (default)                                                     |
|-----------|-------------------------------------------------------------------------|
| `gain`    | `db` (0)                                                                |
| `gate`    | `threshold_db` (-60), `attack_ms` (0.5), `hold_ms` (40), `release_ms` (80) |
| `drive`   | `shape` = soft/hard/asym/fuzz/fold (soft), `gain_db` (12), `level_db` (-6), `bias` (0.2, solo asym) |
| `filter`  | `kind` = lowpass/highpass/bandpass/notch/peak/lowshelf/highshelf, `freq` (1000), `q` (0.707), `gain_db` (0) |
| `delay`   | `time_ms` (350), `feedback` (0.35), `mix` (0.3), `damp_hz` (5000)       |
| `chorus`  | `rate_hz` (0.8), `base_ms` (12), `depth_ms` (3), `mix` (0.5)            |
| `tremolo` | `rate_hz` (5), `depth` (0.5), `wave` = sine/triangle/square             |
| `cab`     | simulazione di cassa. Da file: `ir` (percorso WAV), `max_ms` (80). Procedurale: `low_hz` (100), `low_q` (1.3), `high_hz` (5000), `mid_hz` (500), `mid_db` (-3), `presence_hz` (2500), `presence_db` (3), `ripple_db` (3), `seed` (1). Per entrambe: `level_db` (0) |

La cassa va di solito dopo il drive e prima di chorus e delay. Le IR da file vanno in `irs/` (vedi `irs/LEGGIMI.md`).
Con la cassa procedurale ogni `seed` produce irregolarità diverse del cono, cioè una cassa diversa.

Un preset è un elenco ordinato di `[[block]]`, più `name` e `output_db` (opzionali). Un parametro scritto male è un errore: il suono continua con il preset precedente.

## Aggiungere un blocco

1. Crea la struct dei parametri (`#[serde(default, deny_unknown_fields)]` + `Default`) e il blocco che implementa `dsp::Block`.
2. Aggiungi la variante in `BlockSpec` (`src/dsp/mod.rs`).

In `process` non si alloca memoria e non si fa I/O: gira nel thread realtime.

## Architettura

- `src/engine.rs`: client JACK (servito da PipeWire), callback realtime, scambio lock-free delle catene, misuratori.
- `src/dsp/`: blocchi e catena.
- `src/main.rs`: riga di comando, ricaricamento dei preset, riga di stato.
