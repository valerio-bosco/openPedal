//! Contract tests: the JSON fixtures in apps/desktop/ui/src/fixtures are the wire-format
//! reference. They must round-trip through the Rust types unchanged, and satisfy the JSON
//! Schema the TypeScript and Swift types are generated from.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use openpedal_graph::model::{GraphFile, PedalFile};
use openpedal_protocol::{
    codegen, AudioInfo, ClientMsg, MetersMsg, ServerMsg, TonelabFit, TonelabJobInfo, TonelabJobSummary, TonelabStage, TonelabTemplateResult, PROTOCOL_VERSION,
};

const TONELAB_FIXTURES: [&str; 7] =
    ["tonelab_caps", "tonelab_caps_unavailable", "tonelab_jobs", "tonelab_progress", "tonelab_progress_fit", "tonelab_job", "tonelab_fit_result"];

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

    // param, added
    let param = fixture("param");
    let msg = ServerMsg::Param { node: param["node"].as_str().unwrap().into(), param: param["param"].as_str().unwrap().into(), value: param["value"].as_f64().unwrap() as f32 };
    assert_eq!(wire(&msg), param);
    // response
    let response = fixture("response");
    let freq: Vec<f32> = serde_json::from_value(response["freq"].clone()).unwrap();
    let db: Vec<f32> = serde_json::from_value(response["db"].clone()).unwrap();
    assert_eq!(freq.len(), db.len());
    assert_eq!(wire(&ServerMsg::Response { freq, db }), response);
    let added = fixture("added");
    assert_eq!(wire(&ServerMsg::Added { node: added["node"].as_str().unwrap().into() }), added);

    // state: the catalog is serialize-only (static strings), so it is checked against the schema
    // below; every other part goes through the Rust types.
    let state = fixture("state");
    assert_eq!(state["protocol"], PROTOCOL_VERSION);
    assert_eq!(PROTOCOL_VERSION, 4);
    for key in ["can_undo", "can_redo", "dirty"] {
        assert!(state[key].is_boolean(), "state.{key}");
    }
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
    for name in ["state", "meters", "error", "param", "response", "added"].into_iter().chain(TONELAB_FIXTURES) {
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

#[test]
fn fixtures_carry_a_v2_custom_pedal_and_the_analysis_messages() {
    let state = fixture("state");
    let grit = &state["pedals"]["grit-box"];
    assert_eq!(grit["version"], 2);
    assert!(grit["color"].as_str().unwrap().starts_with('#'));
    assert!(grit.get("knob").is_none());
    let controls = grit["control"].as_array().unwrap();
    let kinds: Vec<&str> = controls.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    for kind in ["knob", "switch", "selector"] {
        assert!(kinds.contains(&kind), "{kind}");
    }
    let modes: Vec<&str> = controls.iter().flat_map(|c| c["route"].as_array().unwrap()).filter_map(|r| r["mode"].as_str()).collect();
    assert!(modes.contains(&"add"));
    let item = state["catalog"].as_array().unwrap().iter().find(|c| c["kind"] == "pedal:grit-box").unwrap();
    assert_eq!(item["color"], grit["color"]);
    // the catalog projects the controls to ordinary parameters
    assert_eq!(item["params"][3]["kind"], "choice");
    assert_eq!(item["params"][4]["options"], json!(["off", "on"]));

    let msgs = fixture("client-messages");
    let types: Vec<&str> = msgs.as_array().unwrap().iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert!(types.contains(&"analyze_pedal") && types.contains(&"analyze_node"));
}

#[test]
fn tonelab_messages_round_trip() {
    let caps = fixture("tonelab_caps");
    let msg = ServerMsg::TonelabCaps { available: true, youtube: caps["youtube"].as_bool().unwrap(), device: caps["device"].as_str().unwrap().into(), reason: None };
    assert_eq!(wire(&msg), caps);
    let off = fixture("tonelab_caps_unavailable");
    assert_eq!(off["available"], false);
    let msg = ServerMsg::TonelabCaps { available: false, youtube: false, device: String::new(), reason: off["reason"].as_str().map(Into::into) };
    assert_eq!(wire(&msg), off);

    let jobs = fixture("tonelab_jobs");
    let list: Vec<TonelabJobSummary> = serde_json::from_value(jobs["jobs"].clone()).unwrap();
    assert_eq!(wire(&ServerMsg::TonelabJobs { jobs: list }), jobs);

    for name in ["tonelab_progress", "tonelab_progress_fit"] {
        let p = fixture(name);
        let stage: TonelabStage = serde_json::from_value(p["stage"].clone()).unwrap();
        let fraction = p["fraction"].as_f64().map(|f| f as f32);
        let msg = ServerMsg::TonelabProgress { job: p["job"].as_str().unwrap().into(), stage, message: p["message"].as_str().unwrap().into(), fraction };
        assert_eq!(wire(&msg), p, "{name}");
    }

    let job = fixture("tonelab_job");
    let info: TonelabJobInfo = serde_json::from_value(job["job"].clone()).unwrap();
    assert!(!info.sections.is_empty() && info.parts.iter().any(|p| p.fit.is_some()));
    assert_eq!(wire(&ServerMsg::TonelabJob { job: info }), job);

    let result = fixture("tonelab_fit_result");
    let msg = ServerMsg::TonelabFitResult {
        job: result["job"].as_str().unwrap().into(),
        part: result["part"].as_str().unwrap().into(),
        fit: serde_json::from_value::<TonelabFit>(result["fit"].clone()).unwrap(),
        templates: serde_json::from_value::<Vec<TonelabTemplateResult>>(result["templates"].clone()).unwrap(),
    };
    assert_eq!(wire(&msg), result);

    let types: Vec<String> = fixture("client-messages").as_array().unwrap().iter().map(|m| m["type"].as_str().unwrap().to_string()).collect();
    for t in ["tonelab_caps", "tonelab_jobs", "tonelab_start", "tonelab_open", "tonelab_pick", "tonelab_record_di", "tonelab_fit", "tonelab_save_preset", "tonelab_cancel"] {
        assert!(types.iter().any(|x| x == t), "client-messages.json lacks {t}");
    }
}
