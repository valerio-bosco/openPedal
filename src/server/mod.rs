//! Server HTTP + WebSocket dell'interfaccia (task B1, vedi docs/PIANO.md §7).
//!
//! Il server non conosce il motore audio: parla solo con un `EngineHandle`.
//! Così si sviluppa e si testa con un motore finto (MockEngine), e l'integrazione
//! collega quello vero.

mod app;
mod assets;
mod files;
pub mod mock;
pub mod protocol;

pub use app::{run, ServerOptions};

use anyhow::Result;

use crate::graph::model::{CatalogItem, GraphFile};
use protocol::{AudioInfo, MetersMsg};

/// Ciò che il server può chiedere al motore. Tutti i metodi sono chiamati da
/// thread normali (mai da quello audio) e devono rispondere in fretta.
/// La lettura e scrittura dei file di preset e pedali la fa il server.
pub trait EngineHandle: Send + Sync + 'static {
    /// Blocchi base, nodi speciali e pedali composti caricati.
    fn catalog(&self) -> Vec<CatalogItem>;
    /// Il grafo che sta suonando adesso.
    fn graph(&self) -> GraphFile;
    fn audio(&self) -> AudioInfo;
    fn meters(&self) -> MetersMsg;
    /// Compila e mette in funzione un grafo. In caso di errore resta quello vecchio;
    /// il messaggio è in italiano e, se possibile, nomina il nodo colpevole.
    fn set_graph(&self, graph: GraphFile) -> Result<()>;
    /// Cambio dal vivo di un parametro (aggiorna anche il grafo restituito da `graph`).
    fn set_param(&self, node: &str, param: &str, value: f32) -> Result<()>;
    /// Da chiamare dopo aver scritto o cancellato file in pedals/.
    fn reload_pedals(&self) -> Result<()>;
}
