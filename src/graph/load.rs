//! Lettura di preset e pedali dal disco.
//! Fase 0: solo preset v2 e pedali. Task A1: conversione dei preset v1, errori dettagliati.

use std::path::Path;

use anyhow::{Context, Result};

use super::model::{GraphFile, PedalFile};

/// Legge un preset v2 (grafo).
pub fn read_graph(path: &Path) -> Result<GraphFile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("impossibile leggere {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("errore nel preset {}", path.display()))
}

/// Legge tutti i pedali composti di una cartella: (id = nome del file senza estensione, pedale).
pub fn read_pedals(dir: &Path) -> Result<Vec<(String, PedalFile)>> {
    let mut pedals = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(pedals) };
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let text = std::fs::read_to_string(&path)?;
        let pedal: PedalFile = toml::from_str(&text).with_context(|| format!("errore nel pedale {}", path.display()))?;
        pedals.push((id, pedal));
    }
    pedals.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pedals)
}
