//! Effects: the single description of every node type.
//!
//! A node's `type` names an **effect**. An effect has user-facing **controls** (knobs, choices, files)
//! and a **source** that says what it is made of:
//!
//! - `Builtin`: one DSP block (or the `input`/`output` junctions). Its controls are the block's
//!   parameters and each one drives the same-named parameter of that block (identity routes).
//! - `Composite`: a sub-graph of other effects. Every control has one or more **routes**, each
//!   writing a mapped control value into a parameter of an inner node. A pedal file (format v2,
//!   docs/PLAN.md §12) defines knob, switch and selector controls; they are projected to the same
//!   descriptors as a block's parameters (knob -> number 0..1, switch -> choice `off`/`on`,
//!   selector -> choice with its options).
//!
//! Later sources (formula blocks) and map kinds (drawn curves, envelope/LFO controls) plug in as new
//! `EffectSource` and `Map` variants; the compiler only talks to this module, never to file formats.
//!
//! The `Registry` is built from `dsp::catalog()` plus the loaded pedal files. `catalog_items`
//! (what the interface receives) is a projection of it.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use openpedal_dsp::params::{BlockDesc, Category, ParamDesc, ParamKind, Value};

use crate::model::{
    CatalogItem, ControlDefault, ControlSpec, ControlType, EdgeSpec, MapKind, NodeSpec, PedalFile, RouteSpec, INPUT_DESC, OUTPUT_DESC,
    PEDAL_PREFIX,
};

pub use crate::model::RouteMode;

/// Name of the implicit inner node of a built-in effect (its own block), as seen by its routes.
pub const SELF_NODE: &str = "self";

/// How a control value is turned into the value of the parameter a route targets.
///
/// Designed to grow (tables, drawn curves...): add a variant and its arm in `value`.
#[derive(Debug, Clone, PartialEq)]
pub enum Map {
    /// The value is passed through unchanged.
    Identity,
    /// 0..1 -> `min..max` (the input is clamped to 0..1 first). `min > max` inverts the control.
    Linear { min: f32, max: f32 },
    /// 0..1 -> `min * (max / min) ^ n` (clamped first). Needs `min` and `max` greater than 0.
    Log { min: f32, max: f32 },
    /// 0..1 -> `max` at 0 down to `min` at 1 (clamped first): a linear map run backwards.
    Inverse { min: f32, max: f32 },
    /// A switch (value >= 0.5 is on): `off` or `on`.
    Switch { off: Value, on: Value },
    /// A selector: the value (rounded, clamped) is an index into the table.
    Table(Vec<Value>),
}

impl Map {
    /// The mapped value. Numeric maps give numbers; `Switch` and `Table` give what they hold
    /// (a number, or an option name of a choice parameter).
    pub fn value(&self, v: f32) -> Value {
        match self {
            Map::Identity => Value::Number(v),
            Map::Linear { min, max } => Value::Number(min + (max - min) * v.clamp(0.0, 1.0)),
            Map::Log { min, max } => {
                if *min > 0.0 && *max > 0.0 {
                    Value::Number(min * (max / min).powf(v.clamp(0.0, 1.0)))
                } else {
                    Value::Number(min + (max - min) * v.clamp(0.0, 1.0))
                }
            }
            Map::Inverse { min, max } => Value::Number(max + (min - max) * v.clamp(0.0, 1.0)),
            Map::Switch { off, on } => if v >= 0.5 { on } else { off }.clone(),
            Map::Table(values) => {
                let last = values.len().saturating_sub(1);
                let i = if v.is_finite() { (v.round().max(0.0) as usize).min(last) } else { 0 };
                values.get(i).cloned().unwrap_or(Value::Number(0.0))
            }
        }
    }

    /// `value` as a number (NaN when the map yields an option name).
    pub fn apply(&self, v: f32) -> f32 {
        match self.value(v) {
            Value::Number(n) => n,
            Value::Text(_) => f32::NAN,
        }
    }
}

/// Where a control value goes: parameter `param` of the inner node `node`, through `map`.
/// If the inner node is itself a composite effect, `param` is one of its controls.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub node: String,
    pub param: String,
    pub map: Map,
    pub mode: RouteMode,
}

impl Route {
    /// A plain `set` route.
    pub fn new(node: &str, param: &str, map: Map) -> Route {
        Route { node: node.into(), param: param.into(), map, mode: RouteMode::Set }
    }
}

/// What a control looks like and how it behaves (mirrors `ParamKind`, with owned strings so that
/// controls defined at run time are possible).
#[derive(Debug, Clone, PartialEq)]
pub enum ControlKind {
    Knob { min: f32, max: f32, default: f32, unit: String, log: bool, step: f32 },
    /// The value is the option index (as f32).
    Choice { options: Vec<String>, default: usize },
    /// File path; changing it requires rebuilding the graph.
    File { extensions: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub kind: ControlKind,
}

impl Control {
    pub fn from_param(p: &ParamDesc) -> Control {
        let kind = match p.kind {
            ParamKind::Number { min, max, default, unit, log, step } => {
                ControlKind::Knob { min, max, default, unit: unit.into(), log, step }
            }
            ParamKind::Choice { options, default } => {
                ControlKind::Choice { options: options.iter().map(|o| o.to_string()).collect(), default }
            }
            ParamKind::File { extensions } => ControlKind::File { extensions: extensions.iter().map(|e| e.to_string()).collect() },
        };
        Control { id: p.id.into(), label: p.label.into(), kind }
    }

    /// The interface descriptor (`ParamDesc` wants static strings: they are interned, so
    /// repeated calls do not leak more).
    pub fn to_param_desc(&self) -> ParamDesc {
        let kind = match &self.kind {
            ControlKind::Knob { min, max, default, unit, log, step } => {
                ParamKind::Number { min: *min, max: *max, default: *default, unit: intern(unit), log: *log, step: *step }
            }
            ControlKind::Choice { options, default } => ParamKind::Choice { options: intern_list(options), default: *default },
            ControlKind::File { extensions } => ParamKind::File { extensions: intern_list(extensions) },
        };
        ParamDesc { id: intern(&self.id), label: intern(&self.label), kind }
    }

    /// The control of a pedal file.
    pub fn from_spec(spec: &ControlSpec) -> Control {
        let label = if spec.label.is_empty() { spec.id.clone() } else { spec.label.clone() };
        let kind = match spec.kind {
            ControlType::Knob => {
                let default = match spec.default {
                    Some(ControlDefault::Number(n)) => n,
                    Some(ControlDefault::Bool(b)) => f32::from(u8::from(b)),
                    None => 0.5,
                };
                ControlKind::Knob { min: 0.0, max: 1.0, default, unit: String::new(), log: false, step: 0.0 }
            }
            ControlType::Switch | ControlType::Selector => {
                let options: Vec<String> = if spec.kind == ControlType::Switch { vec!["off".into(), "on".into()] } else { spec.options.clone() };
                let raw = match spec.default {
                    Some(ControlDefault::Number(n)) if n.is_finite() => n.round().max(0.0) as usize,
                    Some(ControlDefault::Bool(b)) => usize::from(b),
                    _ => 0,
                };
                let default = raw.min(options.len().saturating_sub(1));
                ControlKind::Choice { options, default }
            }
        };
        Control { id: spec.id.clone(), label, kind }
    }
}

/// The body of a composite effect: inner nodes and cables (with the implicit `pedal_in` /
/// `pedal_out`), plus the routes of each control (`routes[i]` belongs to `controls[i]`).
#[derive(Debug, Clone)]
pub struct Composite {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
    pub routes: Vec<Vec<Route>>,
    /// The first structural problem found in the definition (reported by `Registry::check_effect`
    /// and by the compiler, with the pedal's node). `from_pedal` itself never fails.
    pub invalid: Option<String>,
}

#[derive(Debug, Clone)]
pub enum EffectSource {
    Builtin(&'static BlockDesc),
    Composite(Composite),
}

#[derive(Debug, Clone)]
pub struct EffectDesc {
    /// The node `type`: `drive`, `input`, `pedal:<id>`...
    pub kind: String,
    pub label: String,
    pub category: Category,
    pub description: String,
    /// Enamel colour (`#rrggbb`) of a custom pedal.
    pub color: Option<String>,
    pub controls: Vec<Control>,
    pub source: EffectSource,
}

impl EffectDesc {
    /// A built-in block: one inner block, identity routes.
    pub fn builtin(desc: &'static BlockDesc) -> EffectDesc {
        EffectDesc {
            kind: desc.kind.into(),
            label: desc.label.into(),
            category: desc.category,
            description: String::new(),
            color: None,
            controls: desc.params.iter().map(Control::from_param).collect(),
            source: EffectSource::Builtin(desc),
        }
    }

    /// A composite pedal file: each control becomes a control (knob -> 0..1 number, switch ->
    /// `off`/`on`, selector -> its options) with its routes.
    /// Never fails: a malformed definition is recorded in `Composite::invalid` (and a malformed
    /// target, not `node.parameter`, becomes a route with an empty `node`), which the compiler
    /// reports with the node to highlight.
    pub fn from_pedal(id: &str, pedal: &PedalFile) -> EffectDesc {
        let mut invalid = None;
        let mut problem = |msg: String| {
            invalid.get_or_insert(msg);
        };
        let mut seen = HashSet::new();
        let mut controls = Vec::new();
        let mut routes = Vec::new();
        for spec in &pedal.controls {
            if spec.id.is_empty() {
                problem("a control has an empty id".into());
            } else if !seen.insert(spec.id.as_str()) {
                problem(format!("the control id '{}' is used more than once", spec.id));
            }
            let control = Control::from_spec(spec);
            match build_routes(spec) {
                Ok(r) => routes.push(r),
                Err(msg) => {
                    problem(format!("control '{}': {msg}", control.label));
                    routes.push(Vec::new());
                }
            }
            controls.push(control);
        }
        EffectDesc {
            kind: format!("{PEDAL_PREFIX}{id}"),
            label: pedal.name.clone(),
            category: Category::parse(&pedal.category),
            description: pedal.description.clone(),
            color: pedal.color.clone(),
            controls,
            source: EffectSource::Composite(Composite { nodes: pedal.nodes.clone(), edges: pedal.edges.clone(), routes, invalid }),
        }
    }

    pub fn is_composite(&self) -> bool {
        matches!(self.source, EffectSource::Composite(_))
    }

    pub fn control(&self, id: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.id == id)
    }

    /// The routes of control `id`: for a built-in effect, one identity route to its own block.
    pub fn routes(&self, id: &str) -> Vec<Route> {
        match &self.source {
            EffectSource::Builtin(_) => {
                self.control(id).map(|c| vec![Route::new(SELF_NODE, &c.id, Map::Identity)]).unwrap_or_default()
            }
            EffectSource::Composite(c) => {
                self.controls.iter().position(|k| k.id == id).map(|i| c.routes[i].clone()).unwrap_or_default()
            }
        }
    }

    /// What the interface sees.
    pub fn catalog_item(&self) -> CatalogItem {
        let params = match &self.source {
            EffectSource::Builtin(desc) => desc.params.to_vec(),
            EffectSource::Composite(_) => self.controls.iter().map(Control::to_param_desc).collect(),
        };
        CatalogItem {
            kind: self.kind.clone(),
            label: self.label.clone(),
            category: self.category,
            params,
            composite: self.is_composite(),
            description: self.description.clone(),
            color: self.color.clone(),
        }
    }
}

/// Splits "node.param" (a malformed target gives an empty node).
fn split_target(target: &str) -> (&str, &str) {
    target.split_once('.').unwrap_or(("", target))
}

/// The routes of a control, checked against the control's kind (what the route's fields mean).
fn build_routes(spec: &ControlSpec) -> Result<Vec<Route>, String> {
    spec.routes.iter().map(|r| build_route(spec, r)).collect()
}

fn build_route(spec: &ControlSpec, r: &RouteSpec) -> Result<Route, String> {
    let ctx = |msg: &str| format!("route to '{}': {msg}", r.target);
    let (node, param) = split_target(&r.target);
    let map = match spec.kind {
        ControlType::Knob => {
            if r.off.is_some() || r.on.is_some() || r.values.is_some() {
                return Err(ctx("a knob route uses map, min and max (off/on belong to a switch, values to a selector)"));
            }
            let (Some(min), Some(max)) = (r.min, r.max) else { return Err(ctx("a knob route needs min and max")) };
            match r.map {
                MapKind::Linear => Map::Linear { min, max },
                MapKind::Inverse => Map::Inverse { min, max },
                MapKind::Log => {
                    if min <= 0.0 || max <= 0.0 {
                        return Err(ctx("a log map needs min and max greater than 0"));
                    }
                    Map::Log { min, max }
                }
            }
        }
        ControlType::Switch => {
            if r.min.is_some() || r.max.is_some() || r.values.is_some() || r.map != MapKind::Linear {
                return Err(ctx("a switch route uses off and on (map, min and max belong to a knob)"));
            }
            let (Some(off), Some(on)) = (&r.off, &r.on) else { return Err(ctx("a switch route needs off and on")) };
            Map::Switch { off: off.clone(), on: on.clone() }
        }
        ControlType::Selector => {
            if r.min.is_some() || r.max.is_some() || r.off.is_some() || r.on.is_some() || r.map != MapKind::Linear {
                return Err(ctx("a selector route uses values (map, min, max, off and on belong to other controls)"));
            }
            let Some(values) = &r.values else { return Err(ctx("a selector route needs values")) };
            if values.len() != spec.options.len() {
                return Err(ctx(&format!("values has {} entries but the selector has {} options", values.len(), spec.options.len())));
            }
            Map::Table(values.clone())
        }
    };
    Ok(Route { node: node.into(), param: param.into(), map, mode: r.mode })
}

/// Every effect a graph can use: `input`, `output`, the DSP blocks and the loaded pedals, in
/// catalog order.
#[derive(Debug, Clone)]
pub struct Registry {
    effects: Vec<EffectDesc>,
    index: HashMap<String, usize>,
}

impl Registry {
    pub fn new(pedals: &[(String, PedalFile)]) -> Registry {
        let mut effects = vec![EffectDesc::builtin(&INPUT_DESC), EffectDesc::builtin(&OUTPUT_DESC)];
        effects.extend(openpedal_dsp::catalog().iter().map(|e| EffectDesc::builtin(e.desc)));
        effects.extend(pedals.iter().map(|(id, p)| EffectDesc::from_pedal(id, p)));
        let mut index = HashMap::new();
        for (i, e) in effects.iter().enumerate() {
            index.entry(e.kind.clone()).or_insert(i);
        }
        Registry { effects, index }
    }

    /// Adds (or, for an already registered kind, shadows nothing: the first one wins) an effect
    /// defined at run time, e.g. a user-defined pedal.
    pub fn with_effect(mut self, effect: EffectDesc) -> Registry {
        self.index.entry(effect.kind.clone()).or_insert(self.effects.len());
        self.effects.push(effect);
        self
    }

    pub fn get(&self, kind: &str) -> Option<&EffectDesc> {
        self.index.get(kind).map(|&i| &self.effects[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &EffectDesc> {
        self.effects.iter()
    }

    /// The catalog as the interface receives it.
    pub fn catalog(&self) -> Vec<CatalogItem> {
        self.effects.iter().map(EffectDesc::catalog_item).collect()
    }

    /// Checks a composite effect's definition: the problem recorded by `from_pedal`, then every
    /// route against the inner nodes (the node and the parameter exist, a number is driven by a
    /// knob and a choice by a switch or selector with valid values, `add` only on numbers).
    /// The message is in English and starts with the pedal and control it is about.
    pub fn check_effect(&self, effect: &EffectDesc) -> Result<(), String> {
        let EffectSource::Composite(comp) = &effect.source else { return Ok(()) };
        let id = effect.kind.strip_prefix(PEDAL_PREFIX).unwrap_or(&effect.kind);
        if let Some(msg) = &comp.invalid {
            return Err(format!("pedal '{id}': {msg}"));
        }
        for (control, routes) in effect.controls.iter().zip(&comp.routes) {
            let at = format!("pedal '{id}', control '{}'", control.label);
            for route in routes {
                if route.node.is_empty() {
                    return Err(format!("{at}: target '{}' must be 'node.parameter'", route.param));
                }
                let Some(node) = comp.nodes.iter().find(|n| n.id == route.node) else {
                    return Err(format!("{at}: node '{}' does not exist", route.node));
                };
                let Some(inner) = self.get(&node.kind) else {
                    return Err(match node.kind.strip_prefix(PEDAL_PREFIX) {
                        Some(p) => format!("{at}: pedal '{p}' (node '{}') does not exist (missing pedals/{p}.toml)", node.id),
                        None => format!("{at}: node '{}' has an unknown type '{}'", node.id, node.kind),
                    });
                };
                let Some(target) = inner.control(&route.param) else {
                    return Err(match inner.is_composite() {
                        true => format!("{at}: '{}' has no control '{}'", route.node, route.param),
                        false => format!("{at}: '{}' ({}) has no parameter '{}'", route.node, inner.kind, route.param),
                    });
                };
                let name = format!("{}.{}", route.node, route.param);
                match &target.kind {
                    ControlKind::File { .. } => return Err(format!("{at}: '{name}' is a file and cannot be driven by a control")),
                    ControlKind::Knob { .. } => {
                        let wrong = match &route.map {
                            Map::Switch { off, on } => [off, on].into_iter().any(|v| !matches!(v, Value::Number(_))),
                            Map::Table(values) => values.iter().any(|v| !matches!(v, Value::Number(_))),
                            _ => false,
                        };
                        if wrong {
                            return Err(format!("{at}: '{name}' is a number: its values must be numbers, not option names"));
                        }
                    }
                    ControlKind::Choice { options, .. } => {
                        let values: Vec<&Value> = match &route.map {
                            Map::Switch { off, on } => vec![off, on],
                            Map::Table(values) => values.iter().collect(),
                            _ => {
                                let list = options.join(", ");
                                return Err(format!("{at}: '{name}' is a choice ({list}): drive it with a switch or a selector, not a knob"));
                            }
                        };
                        for v in values {
                            let ok = match v {
                                Value::Text(t) => options.contains(t),
                                Value::Number(n) => n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && (*n as usize) < options.len(),
                            };
                            if !ok {
                                let shown = match v {
                                    Value::Text(t) => format!("\"{t}\""),
                                    Value::Number(n) => n.to_string(),
                                };
                                return Err(format!("{at}: {shown} is not an option of '{name}' (options: {})", options.join(", ")));
                            }
                        }
                    }
                }
                if route.mode == RouteMode::Add && (inner.is_composite() || !matches!(target.kind, ControlKind::Knob { .. })) {
                    return Err(format!("{at}: add mode needs a numeric block parameter, and '{name}' is not one"));
                }
            }
        }
        Ok(())
    }
}

/// Validates a pedal file as a whole before it is saved or used: colour, control and route
/// structure, and every route against the inner nodes (other pedals, `others`, may be nested).
pub fn validate_pedal(id: &str, pedal: &PedalFile, others: &[(String, PedalFile)]) -> Result<(), String> {
    if let Some(color) = &pedal.color {
        let hex = color.strip_prefix('#').unwrap_or("");
        if !(hex.len() == 3 || hex.len() == 6) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("color '{color}' must look like #d9573b"));
        }
    }
    let mut all: Vec<(String, PedalFile)> = others.iter().filter(|(o, _)| o != id).cloned().collect();
    all.push((id.to_string(), pedal.clone()));
    let registry = Registry::new(&all);
    registry.check_effect(registry.get(&format!("{PEDAL_PREFIX}{id}")).expect("just registered"))
}

// ---------- interning of strings for the descriptors ----------

static STRINGS: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Default::default);
static LISTS: LazyLock<Mutex<HashMap<Vec<String>, &'static [&'static str]>>> = LazyLock::new(Default::default);

/// A `'static` copy of `s`, leaked once per distinct string.
fn intern(s: &str) -> &'static str {
    let mut set = STRINGS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&found) = set.get(s) {
        return found;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

fn intern_list(list: &[String]) -> &'static [&'static str] {
    if let Some(&found) = LISTS.lock().unwrap_or_else(|e| e.into_inner()).get(list) {
        return found;
    }
    let items: Vec<&'static str> = list.iter().map(|s| intern(s)).collect();
    let leaked: &'static [&'static str] = Box::leak(items.into_boxed_slice());
    LISTS.lock().unwrap_or_else(|e| e.into_inner()).insert(list.to_vec(), leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pedal(text: &str) -> PedalFile {
        toml::from_str(text).unwrap()
    }

    const P: &str = r#"
name = "P"
category = "amp"
description = "d"
[[node]]
id = "c"
type = "cab"
[[knob]]
label = "Brightness"
target = "c.high_hz"
min = 3500
max = 7000
default = 0.4
"#;

    #[test]
    fn map_semantics() {
        assert_eq!(Map::Identity.apply(-3.5), -3.5);
        let lin = Map::Linear { min: 3500.0, max: 7000.0 };
        assert_eq!(lin.apply(0.0), 3500.0);
        assert_eq!(lin.apply(0.5), 5250.0);
        assert_eq!(lin.apply(2.0), 7000.0);
        assert_eq!(lin.apply(-1.0), 3500.0);
        // min > max inverts
        assert_eq!(Map::Linear { min: 1.0, max: 0.0 }.apply(1.0), 0.0);
    }

    #[test]
    fn log_inverse_switch_and_table_maps() {
        let log = Map::Log { min: 100.0, max: 10000.0 };
        assert!((log.apply(0.0) - 100.0).abs() < 1e-3 && (log.apply(1.0) - 10000.0).abs() < 0.1);
        assert!((log.apply(0.5) - 1000.0).abs() < 0.1, "the middle of a log map is the geometric mean");
        assert_eq!(log.apply(3.0), log.apply(1.0));
        let inv = Map::Inverse { min: 2.0, max: 10.0 };
        assert_eq!((inv.apply(0.0), inv.apply(1.0), inv.apply(0.25)), (10.0, 2.0, 8.0));
        let sw = Map::Switch { off: Value::Number(0.0), on: Value::Text("hard".into()) };
        assert_eq!(sw.value(0.0), Value::Number(0.0));
        assert_eq!(sw.value(1.0), Value::Text("hard".into()));
        assert!(sw.apply(1.0).is_nan());
        let table = Map::Table(vec![Value::Number(5.0), Value::Number(7.0), Value::Number(9.0)]);
        assert_eq!((table.apply(0.0), table.apply(1.0), table.apply(2.0), table.apply(99.0), table.apply(-4.0)), (5.0, 7.0, 9.0, 9.0, 5.0));
    }

    const V2: &str = r##"
version = 2
name = "V2"
color = "#d9573b"
[[node]]
id = "d"
type = "drive"
[[control]]
id = "grit"
label = "Grit"
kind = "knob"
default = 0.55
[[control.route]]
target = "d.gain_db"
min = 6
max = 40
[[control]]
id = "voice"
label = "Voice"
kind = "selector"
options = ["Smooth", "Crunch", "Fuzz"]
default = 1
[[control.route]]
target = "d.shape"
values = ["soft", "asym", "fuzz"]
[[control]]
id = "boost"
label = "Boost"
kind = "switch"
default = true
[[control.route]]
target = "d.level_db"
off = 0
on = 6
mode = "add"
"##;

    #[test]
    fn controls_project_to_param_descriptors_and_colour() {
        let e = EffectDesc::from_pedal("v", &pedal(V2));
        let item = e.catalog_item();
        assert_eq!(item.color.as_deref(), Some("#d9573b"));
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["color"], "#d9573b");
        let p = &json["params"];
        assert_eq!((p[0]["id"].as_str(), p[0]["kind"].as_str(), p[0]["min"].as_f64(), p[0]["max"].as_f64()), (Some("grit"), Some("number"), Some(0.0), Some(1.0)));
        assert!((p[0]["default"].as_f64().unwrap() - 0.55).abs() < 1e-6);
        assert_eq!((p[1]["kind"].as_str(), p[1]["default"].as_u64()), (Some("choice"), Some(1)));
        assert_eq!(p[1]["options"], serde_json::json!(["Smooth", "Crunch", "Fuzz"]));
        assert_eq!((p[2]["id"].as_str(), p[2]["kind"].as_str(), p[2]["default"].as_u64()), (Some("boost"), Some("choice"), Some(1)));
        assert_eq!(p[2]["options"], serde_json::json!(["off", "on"]));
        assert!(Registry::new(&[]).catalog().iter().all(|c| c.color.is_none()));
        assert!(serde_json::to_value(&Registry::new(&[]).catalog()[2]).unwrap().get("color").is_none());
        assert!(Registry::new(&[]).check_effect(&e).is_ok());
        assert!(validate_pedal("v", &pedal(V2), &[]).is_ok());
    }

    #[test]
    fn route_fields_follow_the_control_kind() {
        let e = EffectDesc::from_pedal("v", &pedal(V2));
        assert_eq!(e.routes("grit"), [Route::new("d", "gain_db", Map::Linear { min: 6.0, max: 40.0 })]);
        let EffectSource::Composite(c) = &e.source else { panic!() };
        assert_eq!(c.routes[2][0].mode, RouteMode::Add);
        assert_eq!(c.routes[2][0].map, Map::Switch { off: Value::Number(0.0), on: Value::Number(6.0) });
        assert!(matches!(&c.routes[1][0].map, Map::Table(t) if t.len() == 3));
    }

    /// The problem `validate_pedal` reports after one text replacement in `V2`.
    fn problem(from: &str, to: &str) -> String {
        assert!(V2.contains(from), "{from}");
        validate_pedal("v", &pedal(&V2.replace(from, to)), &[]).expect_err(to)
    }

    #[test]
    fn validation_gives_clear_errors() {
        assert!(problem("d.gain_db", "zz.gain_db").contains("node 'zz' does not exist"));
        assert!(problem("d.gain_db", "d.nope").contains("no parameter 'nope'"));
        assert!(problem("d.gain_db", "gain_db").contains("must be 'node.parameter'"));
        assert!(problem("values = [\"soft\", \"asym\", \"fuzz\"]", "values = [\"soft\", \"asym\"]").contains("2 entries but the selector has 3"));
        assert!(problem("\"asym\", \"fuzz\"", "\"asym\", \"gritty\"").contains("\"gritty\" is not an option of 'd.shape'"));
        assert!(problem("target = \"d.shape\"", "target = \"d.gain_db\"").contains("is a number"));
        assert!(problem("target = \"d.gain_db\"\nmin = 6", "target = \"d.shape\"\nmin = 6").contains("is a choice"));
        assert!(problem("off = 0\non = 6", "off = 0\non = 6\nmin = 1").contains("switch route uses off and on"));
        assert!(problem("off = 0\n", "").contains("needs off and on"));
        assert!(problem("min = 6\nmax = 40", "map = \"log\"\nmin = 0\nmax = 40").contains("log map needs min and max greater than 0"));
        assert!(problem("min = 6\nmax = 40", "off = 1\nmin = 6\nmax = 40").contains("knob route uses map"));
        assert!(problem("target = \"d.level_db\"", "target = \"d.shape\"").contains("6 is not an option of 'd.shape'"));
        assert!(problem("kind = \"switch\"", "kind = \"knob\"").contains("knob route"));
        assert!(problem("id = \"boost\"", "id = \"grit\"").contains("used more than once"));
        assert!(problem("#d9573b", "red").contains("color"));
        // a switch may drive a choice by option name, and add mode needs a number
        let named = V2.replace("target = \"d.level_db\"\noff = 0\non = 6\nmode = \"add\"", "target = \"d.shape\"\noff = \"soft\"\non = \"hard\"");
        assert!(validate_pedal("v", &pedal(&named), &[]).is_ok());
        let added = named.replace("on = \"hard\"", "on = \"hard\"\nmode = \"add\"");
        assert!(validate_pedal("v", &pedal(&added), &[]).unwrap_err().contains("add mode needs a numeric"));
        let wrong = named.replace("\"hard\"", "\"sharp\"");
        assert!(validate_pedal("v", &pedal(&wrong), &[]).unwrap_err().contains("\"sharp\" is not an option"));
    }

    #[test]
    fn registry_lists_io_blocks_and_pedals_in_order() {
        let reg = Registry::new(&[("p".into(), pedal(P))]);
        let kinds: Vec<_> = reg.iter().map(|e| e.kind.as_str()).collect();
        let blocks: Vec<_> = openpedal_dsp::catalog().iter().map(|e| e.desc.kind).collect();
        assert_eq!(&kinds[..2], ["input", "output"]);
        assert_eq!(&kinds[2..kinds.len() - 1], blocks.as_slice());
        assert_eq!(kinds.last(), Some(&"pedal:p"));
        assert!(reg.get("drive").is_some() && reg.get("pedal:p").is_some());
        assert!(reg.get("pedal:q").is_none() && reg.get("tubo").is_none());
    }

    #[test]
    fn builtin_controls_mirror_the_block_with_identity_routes() {
        let reg = Registry::new(&[]);
        for e in reg.iter() {
            let EffectSource::Builtin(desc) = &e.source else { panic!("{} is not built in", e.kind) };
            assert_eq!(e.controls.len(), desc.params.len());
            for (c, p) in e.controls.iter().zip(desc.params) {
                // controls round-trip into the exact descriptor the interface used to get
                let a = serde_json::to_value(c.to_param_desc()).unwrap();
                assert_eq!(a, serde_json::to_value(p).unwrap(), "{}.{}", e.kind, c.id);
                assert_eq!(e.routes(&c.id), [Route::new(SELF_NODE, &c.id, Map::Identity)]);
            }
        }
        assert!(reg.get("drive").unwrap().routes("nope").is_empty());
    }

    #[test]
    fn a_pedal_knob_is_a_unit_control_with_one_linear_route() {
        let e = EffectDesc::from_pedal("p", &pedal(P));
        assert!(e.is_composite());
        assert_eq!(e.kind, "pedal:p");
        assert_eq!(e.category, Category::Amp);
        assert_eq!(
            e.controls[0].kind,
            ControlKind::Knob { min: 0.0, max: 1.0, default: 0.4, unit: String::new(), log: false, step: 0.0 }
        );
        assert_eq!(
            e.routes("Brightness"),
            [Route::new("c", "high_hz", Map::Linear { min: 3500.0, max: 7000.0 })]
        );
    }

    #[test]
    fn a_malformed_target_becomes_a_route_without_node() {
        let text = P.replace("c.high_hz", "high_hz");
        let e = EffectDesc::from_pedal("p", &pedal(&text));
        let r = &e.routes("Brightness")[0];
        assert_eq!((r.node.as_str(), r.param.as_str()), ("", "high_hz"));
    }

    #[test]
    fn catalog_projection_matches_the_interface_format() {
        let reg = Registry::new(&[("p".into(), pedal(P))]);
        let items = reg.catalog();
        let p = items.last().unwrap();
        assert!(p.composite);
        assert_eq!(p.description, "d");
        let json = serde_json::to_value(p).unwrap();
        assert_eq!(json["params"][0]["id"], "Brightness");
        assert_eq!(json["params"][0]["kind"], "number");
        assert_eq!(json["params"][0]["max"], 1.0);
        assert!(items[..items.len() - 1].iter().all(|i| !i.composite && i.description.is_empty()));
        // interning: asking twice yields the very same strings
        let again = reg.catalog();
        assert!(std::ptr::eq(p.params[0].id, again.last().unwrap().params[0].id));
    }
}
