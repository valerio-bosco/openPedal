//! Description of the block parameters: this is what lets the interface generate
//! the knobs by itself and lets the files be validated.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParamKind {
    /// Numeric value. `log` = the knob moves on a logarithmic scale;
    /// `step` > 0 = stepped values (e.g. 1.0 for integers).
    Number { min: f32, max: f32, default: f32, unit: &'static str, log: bool, step: f32 },
    /// Choice among options; the value is the option index (as f32).
    Choice { options: &'static [&'static str], default: usize },
    /// File path; changing it requires rebuilding the block.
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

    /// Clamps a numeric value into the valid range (choices become a valid index).
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
    /// Identifier used in files (`type = "drive"`).
    pub kind: &'static str,
    /// Name shown in the interface.
    pub label: &'static str,
    pub category: Category,
    pub params: &'static [ParamDesc],
}

impl BlockDesc {
    pub fn param_index(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }
}

/// A value as it arrives from a TOML file or from the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Number(f32),
    Text(String),
}

/// The parameters of a node: id -> value. Missing ones take the default.
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

    /// Numeric value clamped to the range; default if missing.
    pub fn number(&self, desc: &ParamDesc) -> f32 {
        match (self.0.get(desc.id), desc.kind) {
            (Some(Value::Number(v)), _) => desc.clamp(*v),
            (_, ParamKind::Number { default, .. }) => default,
            _ => 0.0,
        }
    }

    /// Index of the chosen option: accepts the name ("asym") or the index (2).
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

    /// Checks names and types: a typo in a preset must be a clear error.
    pub fn validate(&self, block: &BlockDesc) -> Result<()> {
        for (id, value) in &self.0 {
            let Some(desc) = block.params.iter().find(|p| p.id == id) else {
                let valid: Vec<_> = block.params.iter().map(|p| p.id).collect();
                bail!("'{}' has no parameter '{id}' (valid parameters: {})", block.kind, valid.join(", "));
            };
            match (desc.kind, value) {
                (ParamKind::Number { .. }, Value::Number(_)) | (ParamKind::File { .. }, Value::Text(_)) => {}
                (ParamKind::Choice { options, .. }, Value::Text(name)) => {
                    if !options.contains(&name.as_str()) {
                        bail!("'{}.{id}' cannot be \"{name}\" (choices: {})", block.kind, options.join(", "));
                    }
                }
                (ParamKind::Choice { .. }, Value::Number(_)) => {}
                (ParamKind::Number { .. }, Value::Text(_)) => bail!("'{}.{id}' must be a number", block.kind),
                (ParamKind::File { .. }, Value::Number(_)) => {
                    bail!("'{}.{id}' must be a quoted path", block.kind)
                }
            }
        }
        Ok(())
    }
}
