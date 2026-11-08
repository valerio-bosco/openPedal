//! Grafo di pedali: modello (contratto, fase 0), caricamento, compilazione ed
//! esecuzione (task A1, vedi docs/PIANO.md §7).

pub mod compile;
pub mod exec;
pub mod load;
pub mod model;

#[allow(unused_imports)] // GraphError: per l'integrazione
pub use compile::{compile, GraphError, NodeMap, ParamChange};
pub use exec::Plan;
