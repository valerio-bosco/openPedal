//! A small intermediate representation of the JSON Schema that `schemars` produces for the
//! wire types, restricted to the shapes the protocol uses. Anything else panics on purpose:
//! a new serde shape on the wire must be taught to the generator, not silently mis-generated.

use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub enum Ty {
    Str,
    Num,
    Int,
    Bool,
    Ref(String),
    Array(Box<Ty>),
    /// Fixed-length array of `n` equal items (`[f32; 2]`).
    Tuple(Box<Ty>, usize),
    /// String-keyed map.
    Map(Box<Ty>),
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    /// Always present on the wire.
    pub required: bool,
    /// `null` is a legal value (JSON `Option`).
    pub nullable: bool,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Body {
    Unit,
    Fields(Vec<Field>),
    /// Newtype variant of an internally tagged enum: the struct is flattened next to the tag.
    Newtype(String),
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub tag_value: String,
    pub body: Body,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Tagged {
    pub tag: String,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Struct(Vec<Field>),
    /// A struct whose last member is a flattened internally tagged enum (`ParamDesc`).
    Flatten(Vec<Field>, Tagged),
    Tagged(Tagged),
    StringEnum(Vec<String>),
    /// `#[serde(untagged)]`: one of several types.
    Untagged(Vec<Ty>),
    Alias(Ty),
}

#[derive(Debug, Clone)]
pub struct Def {
    pub name: String,
    pub doc: Option<String>,
    pub kind: Kind,
}

pub fn parse(defs: &Map<String, Value>) -> Vec<Def> {
    let mut names: Vec<&String> = defs.keys().collect();
    names.sort();
    names.into_iter().map(|n| parse_def(n, &defs[n])).collect()
}

fn doc(v: &Value) -> Option<String> {
    v.get("description").and_then(Value::as_str).map(str::to_owned)
}

fn parse_def(name: &str, v: &Value) -> Def {
    let kind = if let Some(values) = v.get("enum") {
        Kind::StringEnum(values.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect())
    } else if let Some(any) = v.get("anyOf") {
        Kind::Untagged(any.as_array().unwrap().iter().map(|t| ty(t).0).collect())
    } else if let Some(one) = v.get("oneOf") {
        let tagged = tagged(name, one);
        if v.get("properties").is_some() {
            Kind::Flatten(fields(v), tagged)
        } else {
            Kind::Tagged(tagged)
        }
    } else if v.get("properties").is_some() {
        Kind::Struct(fields(v))
    } else if v.get("additionalProperties").is_some() {
        Kind::Alias(ty(v).0)
    } else {
        panic!("{name}: unsupported schema shape: {v}")
    };
    Def { name: name.to_owned(), doc: doc(v), kind }
}

fn tagged(owner: &str, one: &Value) -> Tagged {
    let variants = one.as_array().unwrap();
    // The tag is the property that is a `const` in every variant.
    let first = variants[0]["properties"].as_object().unwrap();
    let tag = first
        .iter()
        .find(|(_, p)| p.get("const").is_some())
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| panic!("{owner}: no tag property"));
    let variants = variants
        .iter()
        .map(|var| {
            let tag_value = var["properties"][&tag]["const"].as_str().unwrap_or_else(|| panic!("{owner}: tag must be a string")).to_owned();
            let body = if let Some(r) = var.get("$ref") {
                Body::Newtype(ref_name(r))
            } else {
                let mut fs = fields(var);
                fs.retain(|f| f.name != tag);
                if fs.is_empty() {
                    Body::Unit
                } else {
                    Body::Fields(fs)
                }
            };
            Variant { tag_value, body, doc: doc(var) }
        })
        .collect();
    Tagged { tag, variants }
}

fn ref_name(r: &Value) -> String {
    r.as_str().unwrap().rsplit('/').next().unwrap().to_owned()
}

/// Declaration order of the `required` list first, then the optional fields alphabetically
/// (the order of `properties` itself depends on serde_json's feature flags).
fn fields(v: &Value) -> Vec<Field> {
    let props = v["properties"].as_object().unwrap();
    let required: Vec<&str> =
        v.get("required").and_then(Value::as_array).map(|a| a.iter().map(|s| s.as_str().unwrap()).collect()).unwrap_or_default();
    let mut optional: Vec<&String> = props.keys().filter(|k| !required.contains(&k.as_str())).collect();
    optional.sort();
    required
        .iter()
        .map(|k| (*k).to_owned())
        .chain(optional.into_iter().cloned())
        .map(|name| {
            let p = &props[&name];
            let (ty, nullable) = ty(p);
            Field { required: required.contains(&name.as_str()), name, ty, nullable, doc: doc(p) }
        })
        .collect()
}

/// Returns the type and whether it is nullable.
fn ty(v: &Value) -> (Ty, bool) {
    if let Some(r) = v.get("$ref") {
        return (Ty::Ref(ref_name(r)), false);
    }
    let (kind, nullable) = match &v["type"] {
        Value::String(s) => (s.as_str(), false),
        Value::Array(a) if a.len() == 2 && a[1] == "null" => (a[0].as_str().unwrap(), true),
        other => panic!("unsupported type {other} in {v}"),
    };
    let t = match kind {
        "string" => Ty::Str,
        "number" => Ty::Num,
        "integer" => Ty::Int,
        "boolean" => Ty::Bool,
        "array" => {
            let item = Box::new(ty(&v["items"]).0);
            match (v.get("minItems").and_then(Value::as_u64), v.get("maxItems").and_then(Value::as_u64)) {
                (Some(a), Some(b)) if a == b => Ty::Tuple(item, a as usize),
                _ => Ty::Array(item),
            }
        }
        "object" => Ty::Map(Box::new(ty(&v["additionalProperties"]).0)),
        other => panic!("unsupported type {other}"),
    };
    (t, nullable)
}
