//! Contract tests: the JSON fixtures in apps/desktop/ui/src/fixtures are the wire-format
//! reference. They must round-trip through the Rust types unchanged, and satisfy the JSON
//! Schema the TypeScript and Swift types are generated from.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use openpedal_graph::model::{GraphFile, PedalFile};
use openpedal_protocol::{codegen, AudioInfo, ClientMsg, MetersMsg, ServerMsg, PROTOCOL_VERSION};

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/ui/src/fixtures").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

/// What goes over the wire: serialized to text (f32 prints shortest, `0.15`) and parsed back;
/// `to_value` would widen f32 to f64 (`0.15000000596...`).
fn wire<T: Serialize>(t: &T) -> Value {
    serde_json::from_str(&serde_json::to_string(t).unwrap()).unwrap()
}

/// deserialize -> serialize -> compare as JSON values.
fn round_trip<T: DeserializeOwned + Serialize>(what: &str, v: &Value) {
    let t: T = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(&wire(&t), v, "{what} changed in the round trip");
}

fn validator() -> jsonschema::Validator {
    let root = json!({ "$defs": Value::Object(codegen::schema()), "anyOf": [
        { "$ref": "#/$defs/ClientMsg" },
        { "$ref": "#/$defs/ServerMsg" },
        { "$ref": "#/$defs/CatalogItem" },
    ]});
    jsonschema::validator_for(&root).unwrap()
}

fn assert_valid(v: &jsonschema::Validator, what: &str, instance: &Value) {
    let errors: Vec<String> = v.iter_errors(instance).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{what} does not match the schema: {errors:?}");
}

#[test]
fn client_messages_round_trip() {
    let msgs = fixture("client-messages");
    for m in msgs.as_array().unwrap() {
        round_trip::<ClientMsg>(&format!("client message {}", m["type"]), m);
    }
}

#[test]
fn server_messages_round_trip() {
    // meters
    let meters = fixture("meters");
    let m: MetersMsg = serde_json::from_value(meters.clone()).unwrap();
    assert_eq!(wire(&ServerMsg::Meters(m)), meters);

    // error
    let error = fixture("error");
    let msg = ServerMsg::Error { message: error["message"].as_str().unwrap().into(), node: error["node"].as_str().map(Into::into) };
    assert_eq!(wire(&msg), error);

    // state: the catalog is serialize-only (static strings), so it is checked against the schema
    // below; every other part goes through the Rust types.
    let state = fixture("state");
    assert_eq!(state["protocol"], PROTOCOL_VERSION);
    round_trip::<GraphFile>("state.graph", &state["graph"]);
    round_trip::<AudioInfo>("state.audio", &state["audio"]);
    round_trip::<BTreeMap<String, PedalFile>>("state.pedals", &state["pedals"]);
    round_trip::<Vec<String>>("state.presets", &state["presets"]);
}

#[test]
fn fixtures_match_the_schema() {
    let v = validator();
    for m in fixture("client-messages").as_array().unwrap() {
        assert_valid(&v, "client message", m);
    }
    for c in fixture("catalog").as_array().unwrap() {
        assert_valid(&v, "catalog item", c);
    }
    for name in ["state", "meters", "error"] {
        assert_valid(&v, name, &fixture(name));
    }
    // and the schema is not vacuous
    assert!(!v.is_valid(&json!({ "type": "set_param", "node": "a", "param": "b", "value": "x" })));
    assert!(!v.is_valid(&json!({ "type": "meters", "in_db": 0.0 })));
}

#[test]
fn generated_files_are_current() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, text) in [
        ("apps/desktop/ui/src/generated/protocol.ts", codegen::typescript()),
        ("apps/ios/Sources/OpenPedalApp/Generated/Protocol.swift", codegen::swift()),
    ] {
        let on_disk = std::fs::read_to_string(root.join(path)).unwrap_or_default();
        assert!(on_disk == text, "{path} is stale: run `cargo run -p openpedal-protocol --bin gen-protocol`");
    }
}
