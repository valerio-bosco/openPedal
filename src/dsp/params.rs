//! Descrizione dei parametri dei blocchi: è ciò che permette all'interfaccia di
//! generare le manopole da sola e ai file di essere validati.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParamKind {
    /// Valore numerico. `log` = la manopola si muove in scala logaritmica;
    /// `step` > 0 = valori a scatti (es. 1.0 per gli interi).
    Number { min: f32, max: f32, default: f32, unit: &'static str, log: bool, step: f32 },
    /// Scelta tra opzioni; il valore è l'indice dell'opzione (come f32).
    Choice { options: &'static [&'static str], default: usize },
    /// Percorso di un file; cambiarlo richiede di ricostruire il blocco.
    File { extensions: &'static [&'static str] },
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamDesc {
    pub id: &'static str,
    pub label: &'static str,
    #[serde(flatten)]
    pub kind: ParamKind,
}

impl ParamDesc {
    pub const fn number(id: &'static str, label: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, label, kind: ParamKind::Number { min, max, default, unit, log: false, step: 0.0 } }
    }

    pub const fn log(id: &'static str, label: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, label, kind: ParamKind::Number { min, max, default, unit, log: true, step: 0.0 } }
    }

    pub const fn int(id: &'static str, label: &'static str, min: f32, max: f32, default: f32) -> Self {
        Self { id, label, kind: ParamKind::Number { min, max, default, unit: "", log: false, step: 1.0 } }
    }

    pub const fn choice(id: &'static str, label: &'static str, options: &'static [&'static str], default: usize) -> Self {
        Self { id, label, kind: ParamKind::Choice { options, default } }
    }

    pub const fn file(id: &'static str, label: &'static str, extensions: &'static [&'static str]) -> Self {
        Self { id, label, kind: ParamKind::File { extensions } }
    }

    /// Porta un valore numerico nell'intervallo valido (le scelte diventano un indice valido).
    pub fn clamp(&self, value: f32) -> f32 {
        match self.kind {
            ParamKind::Number { min, max, step, .. } => {
                let v = if value.is_finite() { value.clamp(min, max) } else { min };
                if step > 0.0 {
                    (v / step).round() * step
                } else {
                    v
                }
            }
            ParamKind::Choice { options, .. } => {
                if value.is_finite() {
                    value.round().clamp(0.0, (options.len() - 1) as f32)
                } else {
                    0.0
                }
            }
            ParamKind::File { .. } => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Io,
    Dynamics,
    Drive,
    Filter,
    Modulation,
    Time,
    Amp,
    Utility,
}

impl Category {
    pub fn parse(s: &str) -> Category {
        match s.to_lowercase().as_str() {
            "io" => Category::Io,
            "dynamics" | "dinamica" => Category::Dynamics,
            "drive" | "distorsione" => Category::Drive,
            "filter" | "filtri" | "eq" => Category::Filter,
            "modulation" | "modulazione" => Category::Modulation,
            "time" | "tempo" => Category::Time,
            "amp" | "ampli" | "cassa" => Category::Amp,
            _ => Category::Utility,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BlockDesc {
    /// Identificatore usato nei file (`type = "drive"`).
    pub kind: &'static str,
    /// Nome mostrato nell'interfaccia.
    pub label: &'static str,
    pub category: Category,
    pub params: &'static [ParamDesc],
}

impl BlockDesc {
    pub fn param_index(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }
}

/// Un valore come arriva da un file TOML o dalla UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Number(f32),
    Text(String),
}

/// I parametri di un nodo: id -> valore. Quelli mancanti valgono il default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamValues(pub BTreeMap<String, Value>);

impl ParamValues {
    pub fn set(&mut self, id: &str, value: Value) {
        self.0.insert(id.to_string(), value);
    }

    pub fn get(&self, id: &str) -> Option<&Value> {
        self.0.get(id)
    }

    /// Valore numerico limitato all'intervallo; default se manca.
    pub fn number(&self, desc: &ParamDesc) -> f32 {
        match (self.0.get(desc.id), desc.kind) {
            (Some(Value::Number(v)), _) => desc.clamp(*v),
            (_, ParamKind::Number { default, .. }) => default,
            _ => 0.0,
        }
    }

    /// Indice dell'opzione scelta: accetta il nome ("asym") o l'indice (2).
    pub fn choice(&self, desc: &ParamDesc) -> usize {
        let ParamKind::Choice { options, default } = desc.kind else { return 0 };
        match self.0.get(desc.id) {
            Some(Value::Text(name)) => options.iter().position(|o| o == name).unwrap_or(default),
            Some(Value::Number(v)) => desc.clamp(*v) as usize,
            None => default,
        }
    }

    pub fn text(&self, desc: &ParamDesc) -> Option<&str> {
        match self.0.get(desc.id) {
            Some(Value::Text(t)) => Some(t),
            _ => None,
        }
    }

    /// Controlla nomi e tipi: un refuso in un preset deve essere un errore chiaro.
    pub fn validate(&self, block: &BlockDesc) -> Result<()> {
        for (id, value) in &self.0 {
            let Some(desc) = block.params.iter().find(|p| p.id == id) else {
                let valid: Vec<_> = block.params.iter().map(|p| p.id).collect();
                bail!("'{}' non ha il parametro '{id}' (parametri validi: {})", block.kind, valid.join(", "));
            };
            match (desc.kind, value) {
                (ParamKind::Number { .. }, Value::Number(_)) | (ParamKind::File { .. }, Value::Text(_)) => {}
                (ParamKind::Choice { options, .. }, Value::Text(name)) => {
                    if !options.contains(&name.as_str()) {
                        bail!("'{}.{id}' non può valere \"{name}\" (scelte: {})", block.kind, options.join(", "));
                    }
                }
                (ParamKind::Choice { .. }, Value::Number(_)) => {}
                (ParamKind::Number { .. }, Value::Text(_)) => bail!("'{}.{id}' deve essere un numero", block.kind),
                (ParamKind::File { .. }, Value::Number(_)) => {
                    bail!("'{}.{id}' deve essere un percorso tra virgolette", block.kind)
                }
            }
        }
        Ok(())
    }
}
