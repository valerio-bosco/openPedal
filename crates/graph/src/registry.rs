//! Effects: the single description of every node type.
//!
//! A node's `type` names an **effect**. An effect has user-facing **controls** (knobs, choices, files)
//! and a **source** that says what it is made of:
//!
//! - `Builtin`: one DSP block (or the `input`/`output` junctions). Its controls are the block's
//!   parameters and each one drives the same-named parameter of that block (identity routes).
//! - `Composite`: a sub-graph of other effects. Every control has one or more **routes**, each
//!   writing a mapped control value into a parameter of an inner node. Today's composite-pedal knob
//!   is one route with a `Linear` map (0..1 -> min..max).
//!
//! Later sources (user-defined "agnostic" pedals, formula blocks) plug in as new `EffectSource`
//! variants (or as `Composite`s with richer routes) and new `Map` variants (log, inverse, table...);
//! the compiler and `NodeMap::resolve` only talk to this module, never to file formats.
//!
//! The `Registry` is built from `dsp::catalog()` plus the loaded pedal files. `catalog_items`
//! (what the interface receives) is a projection of it.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use openpedal_dsp::params::{BlockDesc, Category, ParamDesc, ParamKind};

use crate::model::{CatalogItem, EdgeSpec, NodeSpec, PedalFile, INPUT_DESC, OUTPUT_DESC, PEDAL_PREFIX};

/// Name of the implicit inner node of a built-in effect (its own block), as seen by its routes.
pub const SELF_NODE: &str = "self";

/// How a control value is turned into the value of the parameter a route targets.
///
/// Designed to grow (log, inverse, table...): add a variant and its arm in `apply`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Map {
    /// The value is passed through unchanged.
    Identity,
    /// 0..1 -> `min..max` (the input is clamped to 0..1 first). `min > max` inverts the control.
    Linear { min: f32, max: f32 },
}

impl Map {
    pub fn apply(&self, value: f32) -> f32 {
        match *self {
            Map::Identity => value,
            Map::Linear { min, max } => min + (max - min) * value.clamp(0.0, 1.0),
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
}

/// The body of a composite effect: inner nodes and cables (with the implicit `pedal_in` /
/// `pedal_out`), plus the routes of each control (`routes[i]` belongs to `controls[i]`).
#[derive(Debug, Clone)]
pub struct Composite {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
    pub routes: Vec<Vec<Route>>,
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
            controls: desc.params.iter().map(Control::from_param).collect(),
            source: EffectSource::Builtin(desc),
        }
    }

    /// A composite pedal file: each knob becomes a 0..1 control with one `Linear` route.
    /// Never fails: a malformed knob target (not `node.parameter`) becomes a route with an empty
    /// `node` (and the whole text in `param`), which the compiler reports with the node to highlight.
    pub fn from_pedal(id: &str, pedal: &PedalFile) -> EffectDesc {
        let controls = pedal
            .knobs
            .iter()
            .map(|k| Control {
                id: k.label.clone(),
                label: k.label.clone(),
                kind: ControlKind::Knob { min: 0.0, max: 1.0, default: k.default, unit: String::new(), log: false, step: 0.0 },
            })
            .collect();
        let routes = pedal
            .knobs
            .iter()
            .map(|k| {
                let (node, param) = k.target.split_once('.').unwrap_or(("", &k.target));
                vec![Route { node: node.into(), param: param.into(), map: Map::Linear { min: k.min, max: k.max } }]
            })
            .collect();
        EffectDesc {
            kind: format!("{PEDAL_PREFIX}{id}"),
            label: pedal.name.clone(),
            category: Category::parse(&pedal.category),
            description: pedal.description.clone(),
            controls,
            source: EffectSource::Composite(Composite { nodes: pedal.nodes.clone(), edges: pedal.edges.clone(), routes }),
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
            EffectSource::Builtin(_) => self
                .control(id)
                .map(|c| vec![Route { node: SELF_NODE.into(), param: c.id.clone(), map: Map::Identity }])
                .unwrap_or_default(),
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
        }
    }
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
                assert_eq!(e.routes(&c.id), [Route { node: SELF_NODE.into(), param: c.id.clone(), map: Map::Identity }]);
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
            [Route { node: "c".into(), param: "high_hz".into(), map: Map::Linear { min: 3500.0, max: 7000.0 } }]
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
