//! Graph types (v2 presets) and composite pedals: the format of the TOML files
//! and of the JSON messages to the interface. Contract: see docs/PLAN.md §5.3 and §12.

use serde::{Deserialize, Serialize};

use openpedal_dsp::params::{clean_f64, BlockDesc, Category, ParamDesc, ParamValues, Value};

/// Type of the input node (the guitar): one per graph, with no inputs.
pub const INPUT: &str = "input";
/// Type of the output node ("Ampli" in the UI): one per graph; the sound is what arrives here.
pub const OUTPUT: &str = "output";
/// Type prefix of a node that uses a composite pedal: "pedal:<id>".
pub const PEDAL_PREFIX: &str = "pedal:";
/// Implicit nodes inside a composite pedal.
pub const PEDAL_IN: &str = "pedal_in";
pub const PEDAL_OUT: &str = "pedal_out";

pub const INPUT_DESC: BlockDesc = BlockDesc { kind: INPUT, label: "Guitar", category: Category::Io, params: &[] };

pub const OUTPUT_DESC: BlockDesc = BlockDesc {
    kind: OUTPUT,
    label: "Amp (output)",
    category: Category::Io,
    params: &[ParamDesc::number("volume_db", "Volume", -60.0, 12.0, 0.0, "dB")],
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    /// Unique identifier in the graph (letters, digits, `_`, `-`).
    pub id: String,
    /// Type: a catalog block, `input`, `output` or `pedal:<id>`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Position in the editor; ignored by the engine.
    #[serde(default)]
    pub pos: [f32; 2],
    #[serde(default)]
    pub params: ParamValues,
    /// A bypassed node passes its (summed) input through unchanged. Not allowed on
    /// `input`/`output`. Omitted from files and JSON when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub bypass: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
}

/// A v2 preset: a graph of connected nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphFile {
    pub name: String,
    #[serde(default = "version_2")]
    pub version: u32,
    #[serde(default, rename = "node")]
    pub nodes: Vec<NodeSpec>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<EdgeSpec>,
}

fn version_2() -> u32 {
    2
}

/// Legacy (v1) knob of a composite pedal: a 0..1 value mapped linearly onto an internal
/// parameter. Only read from old files (`[[knob]]`): it becomes a v2 knob control with one route.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobSpec {
    pub label: String,
    /// "<internal node id>.<parameter id>", e.g. "c.high_hz".
    pub target: String,
    pub min: f32,
    pub max: f32,
    #[serde(default = "half")]
    pub default: f32,
}

fn half() -> f32 {
    0.5
}

/// What a pedal control looks like and how it is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ControlType {
    // A number from 0 to 1 (`default` = position).
    Knob,
    // Off / on (`default` = true or false).
    Switch,
    // One of `options` (`default` = option index).
    Selector,
}

/// Initial value of a control: a boolean for a switch, a number for a knob or a selector.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ControlDefault {
    Bool(bool),
    Number(f32),
}

impl Serialize for ControlDefault {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ControlDefault::Bool(b) => s.serialize_bool(*b),
            ControlDefault::Number(n) => s.serialize_f64(clean_f64(*n)),
        }
    }
}

fn clean_opt<S: serde::Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(n) => s.serialize_f64(clean_f64(*n)),
        None => s.serialize_none(),
    }
}

/// Curve of a knob route (0..1 -> `min..max`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MapKind {
    // `min + (max - min) * n`
    #[default]
    Linear,
    // `min * (max / min) ^ n` (needs `min` and `max` greater than 0)
    Log,
    // Like linear, run backwards: `n = 0` gives `max` and `n = 1` gives `min`.
    Inverse,
}

fn is_linear(m: &MapKind) -> bool {
    *m == MapKind::Linear
}

/// How a route combines with the other routes writing the same parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    // The route sets the parameter (if several do, the last control in the file wins).
    #[default]
    Set,
    // The route adds an offset on top of the value set by the other routes (or, if none sets
    // it, the node's own value). Numeric parameters only.
    Add,
}

fn is_set(m: &RouteMode) -> bool {
    *m == RouteMode::Set
}

/// One link from a control to an inner parameter. Which fields apply depends on the control:
/// a knob uses `map` + `min` + `max`, a switch `off` + `on`, a selector `values`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    /// "<inner node id>.<parameter id>", e.g. "drive.gain_db".
    pub target: String,
    /// Knob only; omitted from files when linear.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub map: MapKind,
    /// Knob: value of the parameter when the knob is at 0 and at 1.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "clean_opt")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "clean_opt")]
    pub max: Option<f32>,
    /// Selector: the parameter value for each option (numbers, or option names of a choice parameter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Value>>,
    /// Switch: the parameter value when off and when on (a number, or an option name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Value>,
    #[serde(default, skip_serializing_if = "is_set")]
    pub mode: RouteMode,
}

/// A control the user moves: it drives one or more inner parameters through its routes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlSpec {
    /// Stable identifier (used by `params` in graphs and by `set_param`).
    pub id: String,
    /// Name shown in the interface (the id when empty).
    #[serde(default)]
    pub label: String,
    pub kind: ControlType,
    /// Knob: position 0..1 (default 0.5). Switch: true/false (default false). Selector: option index (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ControlDefault>,
    /// Selector only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, rename = "route")]
    pub routes: Vec<RouteSpec>,
}

/// File format version written by this build.
pub const PEDAL_VERSION: u32 = 2;

/// A composite pedal (file `pedals/<id>.toml`), format v2. v1 files (with `[[knob]]`) are read
/// as v2 (each knob becomes a knob control with one linear route) and always written as v2.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PedalFile {
    pub name: String,
    #[serde(default = "pedal_version")]
    pub version: u32,
    #[serde(default = "utility")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    /// Enamel colour of the pedal, `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, rename = "node")]
    pub nodes: Vec<NodeSpec>,
    #[serde(default, rename = "edge")]
    pub edges: Vec<EdgeSpec>,
    #[serde(default, rename = "control")]
    pub controls: Vec<ControlSpec>,
}

fn pedal_version() -> u32 {
    PEDAL_VERSION
}

fn utility() -> String {
    "utility".into()
}

/// What is accepted on disk or on the wire: v2, plus the v1 `[[knob]]` list.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPedal {
    name: String,
    #[serde(default = "pedal_version")]
    version: u32,
    #[serde(default = "utility")]
    category: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default, rename = "node")]
    nodes: Vec<NodeSpec>,
    #[serde(default, rename = "edge")]
    edges: Vec<EdgeSpec>,
    #[serde(default, rename = "control")]
    controls: Vec<ControlSpec>,
    #[serde(default, rename = "knob")]
    knobs: Vec<KnobSpec>,
}

impl<'de> Deserialize<'de> for PedalFile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = RawPedal::deserialize(d)?;
        if raw.version > PEDAL_VERSION {
            return Err(D::Error::custom(format!(
                "pedal file version {} is newer than this build understands ({PEDAL_VERSION})",
                raw.version
            )));
        }
        let mut controls = raw.controls;
        if !raw.knobs.is_empty() && !controls.is_empty() {
            return Err(D::Error::custom("a pedal cannot mix [[knob]] (v1) and [[control]] (v2)"));
        }
        controls.extend(raw.knobs.into_iter().map(|k| ControlSpec {
            id: k.label.clone(),
            label: k.label,
            kind: ControlType::Knob,
            default: Some(ControlDefault::Number(k.default)),
            options: Vec::new(),
            routes: vec![RouteSpec {
                target: k.target,
                map: MapKind::Linear,
                min: Some(k.min),
                max: Some(k.max),
                values: None,
                off: None,
                on: None,
                mode: RouteMode::Set,
            }],
        }));
        Ok(PedalFile {
            name: raw.name,
            version: PEDAL_VERSION,
            category: raw.category,
            description: raw.description,
            color: raw.color,
            nodes: raw.nodes,
            edges: raw.edges,
            controls,
        })
    }
}

/// A catalog entry as the interface sees it (base blocks, input/output and pedals).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct CatalogItem {
    pub kind: String,
    pub label: String,
    pub category: Category,
    pub params: Vec<ParamDesc>,
    /// true = composite pedal (`pedal:<id>`), editable and deletable from the interface.
    pub composite: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Enamel colour of a custom pedal (`#rrggbb`), if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Full catalog: special nodes, base blocks and composite pedals (a projection of the effect registry).
pub fn catalog_items(pedals: &[(String, PedalFile)]) -> Vec<CatalogItem> {
    crate::registry::Registry::new(pedals).catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITHOUT: &str = "name = \"t\"\nversion = 2\n\n[[node]]\nid = \"a\"\ntype = \"gain\"\npos = [1.0, 2.0]\n";

    #[test]
    fn bypass_defaults_to_false_and_is_omitted_when_false() {
        let g: GraphFile = toml::from_str(WITHOUT).unwrap();
        assert!(!g.nodes[0].bypass);
        let text = toml::to_string(&g).unwrap();
        assert!(!text.contains("bypass"), "{text}");
        let json = serde_json::to_value(&g).unwrap();
        assert!(json["node"][0].get("bypass").is_none());
        assert_eq!(toml::from_str::<GraphFile>(&text).unwrap(), g);
    }

    #[test]
    fn bypass_round_trips_through_toml_and_json() {
        let text = format!("{WITHOUT}bypass = true\n");
        let g: GraphFile = toml::from_str(&text).unwrap();
        assert!(g.nodes[0].bypass);
        let back = toml::to_string(&g).unwrap();
        assert!(back.contains("bypass = true"), "{back}");
        assert_eq!(toml::from_str::<GraphFile>(&back).unwrap(), g);
        let json = serde_json::to_value(&g).unwrap();
        assert_eq!(json["node"][0]["bypass"], true);
        assert_eq!(serde_json::from_value::<GraphFile>(json).unwrap(), g);
    }

    const V1: &str = r#"
name = "Old"
category = "amp"
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
    fn v1_knobs_become_v2_controls_and_are_written_as_v2() {
        let p: PedalFile = toml::from_str(V1).unwrap();
        assert_eq!(p.version, 2);
        let c = &p.controls[0];
        assert_eq!((c.id.as_str(), c.label.as_str(), c.kind), ("Brightness", "Brightness", ControlType::Knob));
        assert_eq!(c.default, Some(ControlDefault::Number(0.4)));
        let r = &c.routes[0];
        assert_eq!((r.target.as_str(), r.map, r.min, r.max, r.mode), ("c.high_hz", MapKind::Linear, Some(3500.0), Some(7000.0), RouteMode::Set));
        let text = toml::to_string_pretty(&p).unwrap();
        assert!(text.contains("version = 2") && text.contains("[[control]]") && text.contains("[[control.route]]"), "{text}");
        assert!(!text.contains("[[knob]]") && !text.contains("map ="), "{text}");
        assert!(text.contains("default = 0.4\n"), "f32 values print as written: {text}");
        assert_eq!(toml::from_str::<PedalFile>(&text).unwrap(), p);
    }

    #[test]
    fn mixing_knobs_and_controls_or_a_newer_version_is_an_error() {
        let mixed = format!("{V1}[[control]]\nid = \"x\"\nkind = \"knob\"\n");
        assert!(toml::from_str::<PedalFile>(&mixed).unwrap_err().to_string().contains("mix"));
        let newer = "name = \"n\"\nversion = 3\n";
        assert!(toml::from_str::<PedalFile>(newer).unwrap_err().to_string().contains("newer"));
    }

    const V2: &str = r##"
version = 2
name = "V2"
color = "#d9573b"
[[control]]
id = "voice"
label = "Voice"
kind = "selector"
options = ["Smooth", "Fuzz"]
default = 1
[[control.route]]
target = "d.shape"
values = ["soft", "fuzz"]
[[control]]
id = "boost"
kind = "switch"
default = true
[[control.route]]
target = "d.level_db"
off = 0
on = 6
mode = "add"
[[control]]
id = "grit"
kind = "knob"
[[control.route]]
target = "d.gain_db"
map = "log"
min = 6
max = 40
"##;

    #[test]
    fn v2_controls_round_trip_through_toml_and_json() {
        let p: PedalFile = toml::from_str(V2).unwrap();
        assert_eq!(p.color.as_deref(), Some("#d9573b"));
        assert_eq!(p.controls[1].default, Some(ControlDefault::Bool(true)));
        assert_eq!(p.controls[0].routes[0].values, Some(vec![Value::Text("soft".into()), Value::Text("fuzz".into())]));
        assert_eq!(p.controls[1].routes[0].mode, RouteMode::Add);
        assert_eq!(p.controls[2].routes[0].map, MapKind::Log);
        let text = toml::to_string_pretty(&p).unwrap();
        assert_eq!(toml::from_str::<PedalFile>(&text).unwrap(), p);
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"control\"") && json.contains("\"route\""), "{json}");
        assert_eq!(serde_json::from_str::<PedalFile>(&json).unwrap(), p);
    }
}
