//! Preset e pedali su disco: nomi sanificati, elenco, lettura e scrittura.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::graph::model::{GraphFile, PedalFile};

/// Accetta solo lettere, cifre, `-` e `_` (niente percorsi, niente `..`).
pub fn sanitize(name: &str) -> Result<&str> {
    let ok = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("nome non valido \"{name}\": usa solo lettere, cifre, - e _ (massimo 64 caratteri)");
    }
    Ok(name)
}

pub fn file_path(dir: &Path, name: &str) -> Result<PathBuf> {
    Ok(dir.join(format!("{}.toml", sanitize(name)?)))
}

/// Nomi dei file `.toml` della cartella, senza estensione, in ordine alfabetico.
pub fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .collect();
    names.sort();
    names
}

fn write(dir: &Path, name: &str, text: &str) -> Result<()> {
    let path = file_path(dir, name)?;
    std::fs::create_dir_all(dir).with_context(|| format!("impossibile creare la cartella {}", dir.display()))?;
    std::fs::write(&path, text).with_context(|| format!("impossibile scrivere {}", path.display()))
}

pub fn save_graph(dir: &Path, name: &str, graph: &GraphFile) -> Result<()> {
    write(dir, name, &toml::to_string_pretty(graph).context("impossibile serializzare il preset")?)
}

pub fn save_pedal(dir: &Path, id: &str, pedal: &PedalFile) -> Result<()> {
    write(dir, id, &toml::to_string_pretty(pedal).context("impossibile serializzare il pedale")?)
}

pub fn delete(dir: &Path, name: &str) -> Result<()> {
    let path = file_path(dir, name)?;
    std::fs::remove_file(&path).with_context(|| format!("impossibile cancellare {}", path.display()))
}
