//! Chain templates: `<name>.toml` is a regular preset, `<name>.fit.toml` lists the knobs to fit.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openpedal_dsp::Value;
use openpedal_graph::load::read_graph;
use openpedal_graph::model::{GraphFile, PedalFile};
use openpedal_graph::registry::{ControlKind, Registry};
use serde::Deserialize;

/// One fitted knob.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitParam {
    pub node: String,
    pub key: String,
    pub min: f32,
    pub max: f32,
    #[serde(default)]
    pub log: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FitFile {
    #[serde(default, rename = "param")]
    params: Vec<FitParam>,
}

#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub graph: GraphFile,
    pub params: Vec<FitParam>,
}

impl FitParam {
    /// Maps a unit-cube coordinate to the parameter value.
    pub fn from_unit(&self, u: f64) -> f32 {
        let u = u.clamp(0.0, 1.0);
        if self.log {
            (f64::from(self.min).ln() + u * (f64::from(self.max).ln() - f64::from(self.min).ln())).exp() as f32
        } else {
            (f64::from(self.min) + u * f64::from(self.max - self.min)) as f32
        }
    }

    pub fn to_unit(&self, v: f32) -> f64 {
        let v = v.clamp(self.min, self.max);
        let u = if self.log {
            (f64::from(v).ln() - f64::from(self.min).ln()) / (f64::from(self.max).ln() - f64::from(self.min).ln())
        } else {
            f64::from(v - self.min) / f64::from(self.max - self.min)
        };
        u.clamp(0.0, 1.0)
    }

    /// Rounds for a preset file: whole Hz, 1 decimal for dB, 2 decimals otherwise.
    pub fn round(&self, v: f32) -> f32 {
        let key = self.key.as_str();
        let r = if key.ends_with("_hz") || key == "freq" {
            v.round()
        } else if key.ends_with("_db") {
            (v * 10.0).round() / 10.0
        } else {
            (v * 100.0).round() / 100.0
        };
        r.clamp(self.min, self.max)
    }
}

impl Template {
    /// Loads and validates `<dir>/<name>.toml` and `<dir>/<name>.fit.toml`.
    pub fn load(dir: &Path, name: &str, pedals: &[(String, PedalFile)]) -> Result<Template> {
        let graph = read_graph(&dir.join(format!("{name}.toml")))?;
        let fit_path = dir.join(format!("{name}.fit.toml"));
        let text = std::fs::read_to_string(&fit_path).with_context(|| format!("cannot read {}", fit_path.display()))?;
        let fit: FitFile = toml::from_str(&text).with_context(|| format!("error in {}", fit_path.display()))?;
        let template = Template { name: name.to_string(), graph, params: fit.params };
        template.validate(pedals).with_context(|| format!("template {name}"))?;
        Ok(template)
    }

    /// Names of the templates in a folder (files `<name>.fit.toml`), sorted.
    pub fn list(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".fit.toml")).map(str::to_string))
            .collect();
        names.sort();
        names
    }

    /// Every fitted node and key must exist and be numeric, with a sensible range.
    pub fn validate(&self, pedals: &[(String, PedalFile)]) -> Result<()> {
        let registry = Registry::new(pedals);
        if self.params.is_empty() {
            bail!("no [[param]] to fit");
        }
        for p in &self.params {
            let node = self
                .graph
                .nodes
                .iter()
                .find(|n| n.id == p.node)
                .with_context(|| format!("param {}.{}: no node '{}'", p.node, p.key, p.node))?;
            let effect = registry.get(&node.kind).with_context(|| format!("node '{}' has unknown type '{}'", node.id, node.kind))?;
            let control = effect
                .control(&p.key)
                .with_context(|| format!("node '{}' ({}) has no parameter '{}'", node.id, node.kind, p.key))?;
            let ControlKind::Knob { min, max, .. } = &control.kind else {
                bail!("param {}.{} is not numeric", p.node, p.key);
            };
            if p.min.partial_cmp(&p.max) != Some(std::cmp::Ordering::Less) || (p.log && p.min <= 0.0) {
                bail!("param {}.{}: bad range {}..{}", p.node, p.key, p.min, p.max);
            }
            if p.min < *min || p.max > *max {
                bail!("param {}.{}: range {}..{} outside the block's {}..{}", p.node, p.key, p.min, p.max, min, max);
            }
        }
        Ok(())
    }

    /// Current value of each fitted param (the preset's value, else the block default), clamped to its range.
    pub fn initial(&self, pedals: &[(String, PedalFile)]) -> Vec<f32> {
        let registry = Registry::new(pedals);
        self.params
            .iter()
            .map(|p| {
                let node = self.graph.nodes.iter().find(|n| n.id == p.node);
                let v = match node.and_then(|n| n.params.get(&p.key)) {
                    Some(Value::Number(v)) => *v,
                    _ => node
                        .and_then(|n| registry.get(&n.kind))
                        .and_then(|e| e.control(&p.key))
                        .and_then(|c| match c.kind {
                            ControlKind::Knob { default, .. } => Some(default),
                            _ => None,
                        })
                        .unwrap_or(p.min),
                };
                v.clamp(p.min, p.max)
            })
            .collect()
    }

    /// A copy of the graph with the parameter values applied.
    pub fn with_values(&self, values: &[f32]) -> GraphFile {
        let mut g = self.graph.clone();
        for (p, v) in self.params.iter().zip(values) {
            if let Some(n) = g.nodes.iter_mut().find(|n| n.id == p.node) {
                n.params.set(&p.key, Value::Number(*v));
            }
        }
        g
    }

    /// Unit-cube point -> parameter values.
    pub fn values_from_unit(&self, x: &[f64]) -> Vec<f32> {
        self.params.iter().zip(x).map(|(p, u)| p.from_unit(*u)).collect()
    }

    pub fn unit_from_values(&self, values: &[f32]) -> Vec<f64> {
        self.params.iter().zip(values).map(|(p, v)| p.to_unit(*v)).collect()
    }
}
