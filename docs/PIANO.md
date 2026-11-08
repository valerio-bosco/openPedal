# openPedal: piano di sviluppo (grafo di pedali + nuovi effetti)

> Questo documento è il **contesto di partenza** per gli agenti che lavorano al progetto.
> Ogni task ha un brief autosufficiente: un agente legge questo file, il proprio task, e i file indicati.
> Stato al 2026-11-08: commit `1bce1ab`, catena lineare di blocchi, preset TOML, JACK/PipeWire, buffer 96.

---

## 1. Obiettivo

Un editor **a nodi** (come i node editor di Blender o Unreal) in cui:

- ogni **nodo è un pedale**; i nodi si collegano con dei cavi, in serie o in parallelo;
- il segnale parte dal nodo **Chitarra** e diventa suono quando arriva al nodo **Ampli** (uscita);
- ogni manopola si gira **dal vivo**, mentre suoni;
- si possono **creare pedali nuovi da zero**, sia dall'interfaccia sia scrivendo un file:
  - un **pedale composto** è un sotto-grafo di blocchi base, con le sue manopole "esposte". Si crea selezionando dei nodi e scegliendo "Crea pedale", oppure scrivendo `pedals/*.toml`;
  - un **blocco base** è codice Rust (`src/dsp/`). In una fase successiva un blocco `formula` permetterà di scrivere DSP con espressioni matematiche direttamente dall'interfaccia (fase 3);
- funziona su **Linux, Windows e macOS**.

In parallelo arrivano nuovi effetti: compressore, riverbero, drive con oversampling e tone stack.

---

## 2. Scelta tecnologica dell'interfaccia (da approvare)

| Opzione | Pro | Contro |
|---|---|---|
| **Electron** | ecosistema web enorme | si porta dietro Chromium e Node (~150 MB); il motore audio è in Rust, quindi servono due processi e un canale tra loro |
| **Tauri** | finestra nativa, binario piccolo, backend Rust nello stesso processo | su Linux dipende da WebKitGTK, che complica il pacchetto Nix; più lavoro di build per ogni OS |
| **egui** (GUI nativa Rust) | un solo linguaggio, un solo binario | gli editor a nodi disponibili per egui sono molto meno rifiniti di quelli web |
| **★ Web UI servita da openPedal** | un solo binario con la UI incorporata, si apre nel browser; React Flow è il miglior editor a nodi disponibile; funziona anche dal **telefono** sulla rete di casa (pedaliera senza fili) | si apre nel browser e non in una finestra propria. Si risolve in seguito incapsulandola in Tauri **senza riscrivere niente** |

**Proposta:** Web UI (TypeScript + React + **React Flow** / `@xyflow/react`, build con Vite) servita da un server
**axum** dentro openPedal, con gli asset **incorporati nel binario** (`rust-embed`).
- Utente finale: un eseguibile per sistema operativo; `openpedal ui` apre il browser. Node.js serve **solo a chi sviluppa** (nel flake).
- Più avanti, volendo, una finestra nativa con Tauri riusando la stessa UI.

---

## 3. Architettura di arrivo

```
 ┌──────────── Browser / telefono ────────────┐
 │  UI React Flow: palette, nodi, cavi,       │
 │  manopole, indicatori di livello, preset   │
 └──────────────┬─────────────────────────────┘
                │ WebSocket JSON (protocollo §5.4)
 ┌──────────────▼─────────────────────────────┐
 │ src/server/   axum: HTTP (asset) + WS      │
 │               EngineHandle (trait)         │
 ├────────────────────────────────────────────┤
 │ src/graph/    modello del grafo, compilatore│
 │               (pedali composti → blocchi,  │
 │               ordine topologico), esecutore│
 ├────────────────────────────────────────────┤
 │ src/dsp/      blocchi base + catalogo      │
 │               (descrittori dei parametri)  │
 ├────────────────────────────────────────────┤
 │ src/audio/    backend: JACK/PipeWire       │
 │               (Linux), cpal (Win/macOS)    │
 └────────────────────────────────────────────┘
```

Regole del thread audio, sempre valide: in `process` **niente allocazioni, lock, I/O o log**.
Le modifiche arrivano tramite code lock-free (`rtrb`). Le strutture vecchie si liberano fuori dal thread audio.

---

## 4. Concetti

| Termine | Significato |
|---|---|
| **Blocco base** (primitive) | DSP in Rust: `gain`, `drive`, `filter`, `cab`, `compressor`… Si descrive con un `BlockDesc` |
| **Pedale composto** | sotto-grafo di blocchi base e/o altri pedali, con `knob` esposte. File `pedals/<id>.toml` |
| **Grafo** (preset) | nodi + cavi + posizioni sullo schermo. File `presets/<nome>.toml` (formato v2) |
| `input` | nodo speciale: la chitarra. Un solo esemplare, nessun ingresso |
| `output` ("Ampli") | nodo speciale: l'uscita, con `volume_db`. Un solo esemplare. Senza un percorso fino a qui non esce suono |
| Fan-out / fan-in | da un'uscita possono partire più cavi (si copia il segnale); se più cavi entrano in un nodo, i segnali **si sommano** |
| Cicli | **vietati** (il compilatore li rifiuta). Il feedback si fa dentro i blocchi, per esempio nel delay |

Il segnale nel grafo è **mono**. Lo stereo vero è previsto nella fase 3.

---

## 5. Contratti (li scrive Opus nella fase 0; i task li usano senza modificarli)

### 5.1 Parametri e catalogo: `src/dsp/params.rs`

```rust
pub enum ParamKind {
    Number { min: f32, max: f32, default: f32, unit: &'static str, log: bool },
    Choice { options: &'static [&'static str], default: usize },  // valore = indice, come f32
    File   { extensions: &'static [&'static str] },               // cambiarlo ricostruisce il blocco
}
pub struct ParamDesc { pub id: &'static str, pub label: &'static str, pub kind: ParamKind }

pub enum Category { Dynamics, Drive, Filter, Modulation, Time, Amp, Utility }
pub struct BlockDesc {
    pub kind: &'static str,          // "drive": è il `type` nei file
    pub label: &'static str,         // "Drive": il nome nell'interfaccia
    pub category: Category,
    pub params: &'static [ParamDesc],
}

/// Valori dei parametri di un nodo, come arrivano dal file o dalla UI.
pub struct ParamValues { /* id -> Value (Number(f32) | Text(String)) */ }
impl ParamValues {
    pub fn number(&self, desc: &ParamDesc) -> f32;   // default se manca, limitato a min..max
    pub fn choice(&self, desc: &ParamDesc) -> usize; // accetta il nome ("asym") o l'indice
    pub fn text(&self, desc: &ParamDesc) -> Option<&str>;
}
```

### 5.2 Trait dei blocchi: `src/dsp/mod.rs`

```rust
pub trait Block: Send {
    fn process(&mut self, buf: &mut [f32]);
    /// Chiamato DAL THREAD AUDIO: niente allocazioni. `index` = posizione in BlockDesc::params.
    /// Le variazioni brusche vanno smussate (circa 10-20 ms) per non sentire click.
    fn set_param(&mut self, index: usize, value: f32);
    fn reset(&mut self) {}
}

/// Ogni blocco `<nome>` espone, nel suo modulo:
pub const <NOME>_DESC: BlockDesc;
pub fn build_<nome>(values: &ParamValues, sr: f32) -> anyhow::Result<Box<dyn Block>>;

/// Catalogo: l'unico punto in cui si registrano i blocchi (array CATALOG in src/dsp/mod.rs).
pub fn catalog() -> &'static [Entry];   // Entry { desc: &BlockDesc, build: BuildFn }
pub fn build_block(kind, values, sr) -> Result<Box<dyn Block>>;  // valida i parametri e costruisce
```

- `set_param` ha un'implementazione di default che non fa niente: i blocchi non ancora migrati compilano, ma ignorano i cambi dal vivo.
- `Smoothed` (in `src/dsp/mod.rs`) è il valore smussato da usare per guadagni e livelli: `Smoothed::new(v, 15.0, sr)`, poi `.target = …` in `set_param` e `.next()` per ogni campione.
- **Esempi di riferimento completi:** `dynamics::GAIN_DESC` / `build_gain` / `Gain` e tutto `drive.rs`.
- Registrare un blocco nuovo = **una riga** in `CATALOG`. Nella fase 0 le righe dei blocchi nuovi (compressor, reverb, tonestack) ci sono già e puntano a stub che lasciano passare il segnale invariato.

### 5.3 File del grafo (preset v2) e dei pedali

```toml
# presets/crunch.toml
name = "Crunch"
version = 2

[[node]]
id = "in"
type = "input"
pos = [0, 0]

[[node]]
id = "dist"
type = "drive"
pos = [220, 0]
params = { shape = "asym", gain_db = 12 }

[[node]]
id = "speaker"
type = "pedal:cassa-4x12"      # pedale composto: "pedal:" + nome del file in pedals/
pos = [440, 0]
params = { Brillantezza = 0.4 }

[[node]]
id = "out"
type = "output"
pos = [660, 0]
params = { volume_db = -8 }

[[edge]]
from = "in"
to = "dist"

[[edge]]
from = "dist"
to = "speaker"

[[edge]]
from = "speaker"
to = "out"
```

```toml
# pedals/cassa-4x12.toml
name = "Cassa 4x12"
category = "amp"

[[node]]                     # "pedal_in" e "pedal_out" sono impliciti
id = "c"
type = "cab"
params = { high_hz = 4800, seed = 7 }

[[edge]]
from = "pedal_in"
to = "c"

[[edge]]
from = "c"
to = "pedal_out"

[[knob]]                     # manopola esposta: valore 0..1 mappato su un parametro interno
label = "Brillantezza"
target = "c.high_hz"
min = 3500
max = 7000
default = 0.4
```

I preset v1 (`[[block]]` in fila) vengono convertiti automaticamente in un grafo lineare `input → … → output`.
In JSON le chiavi restano **al singolare** (`node`, `edge`, `knob`), come nei file TOML.
Nei `params` le scelte compaiono come testo (`shape = "asym"`) e i file come percorso; con `set_param` invece una scelta si manda come indice.
Tipi Rust: `src/graph/model.rs`. Tipi TypeScript: `ui/src/protocol.ts`.

### 5.4 Protocollo WebSocket (`/ws`, messaggi JSON con campo `type`)

Dal client al server:

| `type` | Campi | Effetto |
|---|---|---|
| `hello` | – | il server risponde con `state` |
| `set_param` | `node`, `param`, `value` | cambio dal vivo, senza ricostruire il grafo |
| `set_graph` | `graph` | nuovo grafo: viene ricompilato e sostituito con una dissolvenza |
| `load_preset` / `save_preset` | `name` (+ `graph` per il salvataggio) | |
| `save_pedal` / `delete_pedal` | `id` + `pedal` / `id` | scrive o cancella `pedals/<id>.toml`, poi rimanda `state` con il catalogo aggiornato |

Dal server al client:

| `type` | Campi |
|---|---|
| `state` | `catalog` (blocchi base + pedali, con i parametri), `graph`, `presets`, `audio` (sample rate, buffer) |
| `meters` | `in_db`, `out_db`, `dsp` (0..1), `xruns`, inviato circa 20 volte al secondo |
| `error` | `message` (in italiano, da mostrare all'utente), `node` (opzionale, se l'errore riguarda un nodo) |

Gli esempi JSON esatti stanno in `ui/src/fixtures/*.json` (`state`, `meters`, `error`, `client-messages`, `catalog`) e si rigenerano con `openpedal dev-fixtures ui/src/fixtures`. Fanno fede quelli.

### 5.5 Confine tra server e motore: `src/server/mod.rs`

```rust
pub trait EngineHandle: Send + Sync + 'static {
    fn catalog(&self) -> Vec<CatalogItem>;
    fn graph(&self) -> GraphFile;
    fn audio(&self) -> AudioInfo;
    fn meters(&self) -> MetersMsg;
    fn set_graph(&self, graph: GraphFile) -> Result<()>;
    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()>;
    fn reload_pedals(&self) -> Result<()>;
}
```
Il **server** legge e scrive i file in `presets/` e `pedals/`, usando `graph::load::read_graph` e `read_pedals`. Il **motore** compila e suona.
L'implementazione di `EngineHandle` per il motore vero la scrive l'integrazione (I2); fino ad allora `openpedal ui` usa il `MockEngine` di B1.

---

## 6. Fasi e task

```
Fase 0 (Opus) ──┬─► A1 grafo (Sonnet) ───────────┐
 contratti      ├─► B1 server (Sonnet) ──────────┤
 + scheletro    ├─► C1 interfaccia (Sonnet) ─────┼─► Fase 2 integrazione (Opus)
 + fixture      ├─► D1 migrazione blocchi (Haiku)┤        │
                ├─► D2 compressore (Haiku) ──────┤        ├─► E1 audio Win/macOS (Sonnet)
                ├─► D3 riverbero (Haiku) ────────┤        └─► Fase 3 (formula, stereo, Tauri, MIDI)
                └─► D4 drive + tone stack (Sonnet)┘
```

**Percorso più veloce per vedere l'interfaccia:** fase 0, poi C1a (editor che lavora sulle fixture, ancora senza audio).
Il suono arriva con A1 + B1 + integrazione.

| ID | Modello | Dipende da | File di sua proprietà (solo questi) | Fatto quando |
|---|---|---|---|---|
| **F0** | Opus | – | tutto lo scheletro | contratti compilati, fixture JSON, stub, `cargo test` verde, `nix develop` con Node |
| **A1** | Sonnet | F0 | `src/graph/**` (tranne `model.rs`), `src/engine.rs`, le funzioni `play`/`render`/`check_presets` di `src/main.rs` | grafi con fan-in/out, pedali composti, dissolvenza, `set_param` dal vivo, conversione v1; test |
| **B1** | Sonnet | F0 | `src/server/**` (tranne `protocol.rs` e il trait), `Cargo.toml`/`Cargo.lock` (solo aggiunte), il comando `ui` in `src/main.rs` | `openpedal ui` serve la UI e il WS secondo §5.4; test con `MockEngine` |
| **C1** | Sonnet | F0 | `ui/**` | vedi brief: C1a editor su fixture, C1b WS reale, C1c crea pedale |
| **D1** | Haiku | F0 | `src/dsp/{dynamics,filter,modulation,time,cab}.rs` (in dynamics solo la parte Gate) | `set_param` implementato in gate, filter, tremolo, delay, chorus, cab; vecchi test verdi |
| **D2** | Haiku | F0 | `src/dsp/compressor.rs` | specifica §7.D2 rispettata; test |
| **D3** | Haiku | F0 | `src/dsp/reverb.rs` | specifica §7.D3 rispettata; test |
| **D4** | Sonnet | F0 | `src/dsp/{drive,oversample,tonestack}.rs` (il parametro `quality` esiste già in `DRIVE_DESC`) | aliasing ridotto (test misurato), tone stack; test |
| **I2** | Opus | tutti | – | merge, verifica realtime, prova dal vivo con la Scarlett |
| **E1** | Sonnet | I2 | `src/audio/**` | backend cpal compilabile per Windows e macOS, JACK invariato su Linux |

### Regole per tutti gli agenti
1. Lavora nel **tuo worktree** e modifica **solo i file di tua proprietà**. Se ti serve cambiare un contratto (§5), **fermati e scrivilo nel rapporto finale** invece di modificarlo.
2. Stile: commenti e messaggi d'errore **in italiano**, nomi in inglese, come il codice esistente. Commenti brevi e solo dove servono.
3. Prima di finire: `nix develop -c cargo test` e `nix develop -c cargo clippy` senza errori, e senza avvisi nei tuoi file (per C1: `cd ui && npm install && npm run build`; `npm run lint` è `tsc --noEmit`). Esegui ogni comando con `nix develop -c …` dalla radice del worktree.
6. Alla fine fai **un commit** nel tuo worktree con un messaggio chiaro in italiano, così l'integrazione può fare il merge.
4. Non lanciare l'audio dal vivo (`openpedal` senza `render` o `check`): le prove con la scheda le fa l'integrazione.
5. Rapporto finale: cosa hai fatto, file toccati, test aggiunti, dubbi o deviazioni dal contratto.

---

## 7. Brief dei task

### A1 · Grafo ed esecutore (Sonnet)
**Leggi:** §3-§5, `src/engine.rs`, `src/main.rs` (`play`, `render`), `src/dsp/mod.rs`, `src/dsp/params.rs`, `src/graph/model.rs`, `src/graph/load.rs`, `docs/esempi/crunch-grafo.toml`, `pedals/cassa-4x12.toml`.
**Fai:**
1. `graph/load.rs`: `read_graph` riconosce e converte anche i preset v1 (`[[block]]` → grafo lineare con `input` e `output`; `output_db` → `volume_db` dell'output; posizioni automatiche ogni 220 px). Errori chiari (nodo inesistente, tipo sconosciuto, ciclo, più di un `input`/`output`).
2. `graph/compile.rs`: espande i pedali composti (anche annidati, rilevando le ricorsioni) e applica le `knob` (0..1 → `min..max` del target); esegue l'ordinamento topologico e scarta i nodi che non portano all'`output`. Produce un `Plan` con buffer preallocati (8192 campioni per nodo).
3. `graph/exec.rs`: esegue il `Plan` (copia per il fan-out, somma per il fan-in) **senza allocare**.
4. `engine.rs` e `main.rs`: `play`, `render` e `check` usano i grafi (tutti i preset in `presets/` devono continuare a funzionare e suonare **identici**); `Chain` può sparire. In `engine.rs` sostituisci `Chain` con `Plan`; la sostituzione avviene con una **dissolvenza di 20 ms** (tieni vivi entrambi i piani durante la dissolvenza); aggiungi una coda `set_param(node_index, param_index, value)` verso il thread audio. La mappa id → indice la tiene il thread di controllo.
Esponi in `graph/` una API pulita che l'integrazione userà per implementare `EngineHandle`: per esempio `compile(&GraphFile, &[(String, PedalFile)], sr) -> Result<(Plan, NodeMap)>`, dove `NodeMap` traduce (id del nodo, id del parametro) in indici per `set_param`. I parametri dei pedali composti (le knob) devono funzionare anche dal vivo.
**Test:** grafo lineare uguale alla vecchia catena (campione per campione); fan-in somma; un ciclo viene rifiutato; un pedale annidato viene espanso; una knob mappa correttamente; la dissolvenza non produce salti (> 0,1) su un seno.

### B1 · Server (Sonnet)
**Leggi:** §2, §5.4-§5.5, `ui/src/fixtures/*.json`, `src/server/mod.rs` (il trait `EngineHandle`), `src/server/protocol.rs`, `src/graph/load.rs`, `src/main.rs`.
**Fai:** un `MockEngine` (`src/server/mock.rs`) che implementa `EngineHandle` in memoria: catalogo vero (`graph::model::catalog_items`), grafo modificabile, indicatori finti che si muovono, `running: false`. Poi axum + tokio; `GET /` e gli asset da `rust-embed` (cartella `ui/dist`); `GET /ws` con il protocollo §5.4; `meters` a 20 Hz; salvataggio di preset e pedali su disco con nomi file sanificati; `openpedal ui [--port 7777] [--no-browser]` apre il browser (crate `webbrowser`). Ascolta su `127.0.0.1` per default; con `--lan` su `0.0.0.0` (per il telefono), stampando l'indirizzo.
Finché il motore vero non è integrato, `openpedal ui` usa il `MockEngine`. Se `ui/dist` non esiste, il server risponde con una pagina che spiega di lanciare `cd ui && npm run build` (la compilazione di Rust non deve fallire per questo).
**Test:** con `MockEngine`: `hello` → `state` conforme alle fixture; `set_param` arriva all'engine; un nome di file pericoloso (`../x`) viene rifiutato.

### C1 · Interfaccia a nodi (Sonnet)
**Leggi:** §1, §4, §5, `ui/src/fixtures/*.json`, `ui/src/protocol.ts` (tipi TS scritti nella fase 0), `ui/src/App.tsx` (scheletro da sostituire), `ui/package.json`, `ui/vite.config.ts`.
**Stack:** Vite + React + TypeScript + `@xyflow/react`. Niente librerie UI pesanti; CSS proprio, tema scuro, aspetto da pedaliera.
- **C1a (subito, solo fixture):** palette dei blocchi per categoria (trascina nel canvas); nodi con le manopole generate da `ParamDesc` (Number = manopola con trascinamento verticale e doppio clic per scrivere il valore, log se `log`; Choice = menu; File = campo testo); cavi tra nodi; nodi `input` e `output` sempre presenti e non cancellabili; un cavo che creerebbe un ciclo viene rifiutato; cancella, duplica, zoom e minimappa. Barra in alto: preset, salva, indicatori IN/OUT. Modalità mock: `npm run dev` funziona senza server.
- **C1b:** client WS reale (`ws.ts`) con riconnessione; `set_param` limitato a circa 60 messaggi al secondo per manopola; `set_graph` quando cambia la struttura; mostra gli errori sul nodo interessato.
- **C1c — Crea pedale:** seleziona più nodi → "Crea pedale" → finestra con nome, categoria e scelta delle manopole da esporre (parametro, intervallo) → `save_pedal`. Il nuovo pedale compare nella palette e i nodi selezionati vengono sostituiti da lui. Doppio clic su un pedale composto: "entra" nel sotto-grafo e lo modifica.
**Fatto quando:** `npm run build` e `npm run lint` passano; il rapporto include uno screenshot (se hai uno strumento per farlo) oppure una descrizione precisa dell'interfaccia.

### D1 · Migrazione dei blocchi esistenti (Haiku)
**Leggi:** §5.1-§5.2, `src/dsp/mod.rs`, `src/dsp/params.rs`, `src/dsp/drive.rs` e la parte Gain di `src/dsp/dynamics.rs` (**esempi di riferimento già migrati**), il file che migri.
Nella fase 0 ogni blocco ha già il suo `*_DESC` e il suo `build_*`, che traduce i `ParamValues` nella vecchia struct `*Params`. **Non cambiare `*_DESC`** (l'interfaccia dipende da quei nomi).
**Fai, per gate, filter, tremolo, delay, chorus, cab:** implementa `set_param` (l'indice segue l'ordine dei parametri nel `*_DESC`; usa costanti con nome per gli indici, come in `drive.rs`):
- gate, tremolo: aggiorna subito il valore (sono già smussati);
- filter: ricalcola i coefficienti (niente allocazioni);
- delay: la memoria si alloca per 2 s (il massimo del `DELAY_DESC`) alla costruzione; il tempo cambia con un glide di circa 50 ms;
- chorus: aggiorna i parametri; `base_ms + depth_ms` è limitato da una memoria preallocata di 50 ms;
- cab: solo `level_db` cambia dal vivo (smussato); gli altri parametri richiedono una ricostruzione, quindi in `set_param` vengono ignorati, con un commento che lo spiega.
**Non** cambiare gli algoritmi. **Test:** quelli esistenti devono passare; aggiungi per ogni blocco un test in cui `set_param` cambia il risultato.

### D2 · Compressore (Haiku)
**Leggi:** §5.1-§5.2, `src/dsp/mod.rs`, `src/dsp/params.rs`, `src/dsp/drive.rs` (esempio di riferimento), lo stub.
**File:** `src/dsp/compressor.rs` (lo stub esiste già, con `COMPRESSOR_DESC` definitivo e la riga nel catalogo). Implementa anche `set_param` per ogni parametro.
**Parametri (`DESC`):** `threshold_db` (−60..0, −20), `ratio` (1..20, 4, log), `attack_ms` (0.1..100, 5, log), `release_ms` (10..1000, 120, log), `knee_db` (0..12, 6), `makeup_db` (0..24, 0), `mix` (0..1, 1).
**Algoritmo** (feed-forward, rilevatore di picco nel dominio dei dB):
```
x_db  = 20*log10(max(|x|, 1e-9))
// gain computer con ginocchio morbido (Giannoulis, Massberg, Reiss 2012)
se 2*(x_db - T) < -W:        y_db = x_db
se 2*|x_db - T| <= W:        y_db = x_db + (1/R - 1) * (x_db - T + W/2)^2 / (2W)
altrimenti:                  y_db = T + (x_db - T) / R
g = y_db - x_db                                  // <= 0
// smoothing: attack se g < g_s, altrimenti release
g_s = coef * g_s + (1 - coef) * g ,  coef = exp(-1 / (ms * 0.001 * sr))
out = x * db_to_lin(g_s + makeup) ; out = mix*out + (1-mix)*x
```
**Test:** un seno a −10 dBFS con T = −20 e R = 4, a regime, esce a circa −17,5 dBFS (±0,5); sotto soglia non cambia niente (±0,1 dB); `mix = 0` dà il segnale identico all'ingresso.

### D3 · Riverbero (Haiku)
**Leggi:** §5.1-§5.2, `src/dsp/mod.rs`, `src/dsp/params.rs`, `src/dsp/drive.rs` (esempio di riferimento), lo stub.
**File:** `src/dsp/reverb.rs` (lo stub esiste già, con `REVERB_DESC` definitivo). Algoritmo **Freeverb** (Jezar), mono. Implementa anche `set_param` (per `predelay_ms` la memoria si alloca per il massimo, 100 ms).
**Parametri:** `room` (0..1, 0.5), `damp` (0..1, 0.5), `mix` (0..1, 0.25), `predelay_ms` (0..100, 10), `width_hz` (taglio passa-basso del riverbero, 2000..12000, 8000, log).
**Struttura:** pre-delay → 8 comb filter in parallelo (con passa-basso nella retroazione) → somma → 4 allpass in serie → passa-basso → mix.
Lunghezze originali a 44,1 kHz: comb `1116 1188 1277 1356 1422 1491 1557 1617`, allpass `556 441 341 225`, da scalare per `sr/44100`.
`feedback_comb = room*0.28 + 0.7`, `damp1 = damp*0.4`, allpass `g = 0.5`, guadagno d'ingresso `0.015`.
Le memorie si allocano **solo** in `build`.
**Test:** un impulso produce una coda che decade (energia tra 1,0 e 1,5 s < energia tra 0,1 e 0,6 s); `mix = 0` dà il segnale identico all'ingresso; nessun NaN dopo 10 s di rumore a 0 dBFS con `room = 1`.

### D4 · Drive di qualità + tone stack (Sonnet)
**Leggi:** §5.1-§5.2, `src/dsp/mod.rs`, `src/dsp/params.rs`, i tre file.
**File:** `src/dsp/drive.rs` (già migrato nella fase 0), `src/dsp/oversample.rs`, `src/dsp/tonestack.rs` (stub con `TONESTACK_DESC` definitivo).
1. **Oversampling 4x** attorno al waveshaper: due stadi 2x con FIR half-band polifase (≥ 60 dB di attenuazione in banda bloccata). Il parametro `quality` (Choice: `eco` = 1x, `alta` = 4x; default `alta`) esiste già in `DRIVE_DESC`; fallo funzionare anche dal vivo. Buffer interni preallocati per blocchi fino a 8192 campioni.
2. **Tone stack** (`tonestack`): `bass`, `mid`, `treble` (0..1, 0.5) e `model` (Choice: `fender`, `marshall`, `vox`). Modello di D. T. Yeh, *"Automated physical modeling of nonlinear audio circuits"* (TMB, filtro del 3° ordine discretizzato con trasformazione bilineare); coefficienti ricalcolati in `set_param` senza allocare.
**Test:** seno a 5 kHz con `gain_db = 30`: l'energia aliasata sotto i 4 kHz (le frequenze che non sono armoniche) scende di almeno 20 dB tra `eco` e `alta`. Tone stack: `bass = 0` attenua i 100 Hz rispetto a `bass = 1`, stabile per ogni combinazione 0/0,5/1.

### E1 · Audio su Windows e macOS (Sonnet, dopo l'integrazione)
`src/audio/mod.rs` con il trait `AudioBackend`; `jack.rs` (codice attuale) usato di default su Linux; `cpal.rs` per WASAPI e CoreAudio. Ingresso e uscita sono stream separati, uniti da una coda (`rtrb`) di 2 buffer; con la feature `asio` si usa ASIO. Verifica la compilazione per `x86_64-pc-windows-gnu` e `aarch64-apple-darwin` (almeno `cargo check`).

---

## 8. Fase 0: cosa prepara Opus (fatta)

1. `src/dsp/params.rs`, il trait `Block` con `set_param`, il catalogo; migrazione **di riferimento** di `gain` e `drive`.
2. Stub di `compressor`, `reverb`, `tonestack`, `oversample` già registrati, che passano il segnale senza modificarlo.
3. `src/graph/model.rs` (tipi del grafo e dei pedali), scheletri di `src/server/` (trait `EngineHandle`, `MockEngine`), dipendenze in `Cargo.toml` (axum, tokio, rust-embed, webbrowser, serde_json).
4. Comando `openpedal catalog --json` e fixture `ui/src/fixtures/{catalog,state,meters}.json` generate dal codice vero; `ui/src/protocol.ts` scritto a mano, coerente con le fixture.
5. Scheletro `ui/` (Vite + React + TS + xyflow) che compila; `nodejs` aggiunto al flake.
6. Commit "fase 0", poi lancio di A1, B1, C1, D1, D2, D3 e D4 in parallelo, ciascuno nel suo worktree.

## 9. Integrazione (I2, Opus) — fatta l'8/11/2026

Aggiunte durante l'integrazione: `src/live.rs` (EngineHandle del motore vero, con ricostruzione automatica per i parametri non modificabili dal vivo, come la cassa procedurale), `pedals` nel messaggio `state`, nodo indicato negli errori del grafo, filtro con glide dei parametri.

Merge nell'ordine D1 → D2/D3/D4 → A1 → B1 → C1, risolvendo i conflitti sul catalogo; revisione delle regole realtime in ogni `process` / `set_param`; `render` sulle registrazioni in `recordings/` per verificare i livelli; prova dal vivo con la Scarlett insieme all'utente.

## 10. Fase 3 (dopo)
- **Blocco `formula`:** DSP scritto come espressione (es. `tanh(x * drive) * 0.5`), compilato in un piccolo bytecode al cambio e valutato senza allocazioni. È il "pedale da zero" anche a livello di algoritmo.
- Stereo vero (riverbero e chorus larghi), MIDI (pedali e controller), finestra nativa con Tauri, pacchetti e CI per i tre sistemi operativi.
