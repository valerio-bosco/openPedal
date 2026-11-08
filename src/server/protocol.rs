//! Messaggi WebSocket tra interfaccia e openPedal (contratto: docs/PIANO.md §5.4).
//! Gli esempi JSON in ui/src/fixtures/ sono generati da questi tipi
//! (`openpedal dev-fixtures ui/src/fixtures`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::model::{CatalogItem, GraphFile, PedalFile};

/// Dal client (interfaccia) al server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Il server risponde con `state`.
    Hello,
    /// Cambio dal vivo di un parametro, senza ricostruire il grafo.
    /// Per le scelte `value` è l'indice dell'opzione.
    SetParam { node: String, param: String, value: f32 },
    /// Nuovo grafo (struttura cambiata): ricompilato e sostituito con una dissolvenza.
    SetGraph { graph: GraphFile },
    LoadPreset { name: String },
    SavePreset { name: String, graph: GraphFile },
    /// Scrive pedals/<id>.toml; il server poi rimanda `state` con il catalogo aggiornato.
    SavePedal { id: String, pedal: PedalFile },
    DeletePedal { id: String },
}

/// Dal server al client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State(StateMsg),
    Meters(MetersMsg),
    Error {
        /// In italiano, da mostrare all'utente.
        message: String,
        /// Il nodo a cui si riferisce l'errore, se c'è.
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMsg {
    pub catalog: Vec<CatalogItem>,
    pub graph: GraphFile,
    /// Nomi dei preset disponibili (file in presets/ senza estensione).
    pub presets: Vec<String>,
    /// Contenuto dei pedali composti (id -> pedale), per poterli aprire e modificare.
    pub pedals: BTreeMap<String, PedalFile>,
    pub audio: AudioInfo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub buffer: u32,
    /// false = nessuna scheda audio collegata (modalità solo editor).
    pub running: bool,
}

/// Inviato circa 20 volte al secondo.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MetersMsg {
    pub in_db: f32,
    pub out_db: f32,
    /// Carico del thread audio, 0..1 (1 = tempo esaurito).
    pub dsp: f32,
    pub xruns: u64,
}
