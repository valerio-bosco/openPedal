//! HTTP routes, WebSocket and message handling (protocol: docs/PLAN.md §5.4).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};

use super::protocol::{ClientMsg, ServerMsg, StateMsg};
use super::{assets, files, EngineHandle};
use crate::graph::load::read_graph;

pub struct ServerOptions {
    pub port: u16,
    pub open_browser: bool,
    /// true = listen on all interfaces (for the phone).
    pub lan: bool,
    pub presets_dir: PathBuf,
    pub pedals_dir: PathBuf,
}

pub struct AppState {
    pub engine: Arc<dyn EngineHandle>,
    pub presets_dir: PathBuf,
    pub pedals_dir: PathBuf,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new().route("/ws", get(ws_handler)).fallback(assets::serve).with_state(state)
}

/// Starts the server; does not return until Ctrl+C or SIGTERM arrives.
pub fn run(engine: Arc<dyn EngineHandle>, opts: ServerOptions) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().context("cannot start tokio")?;
    rt.block_on(async move {
        let ip = if opts.lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(ip), opts.port))
            .await
            .with_context(|| format!("cannot listen on port {} (it may already be in use: try --port)", opts.port))?;
        let port = listener.local_addr()?.port();
        let local_url = format!("http://127.0.0.1:{port}");
        eprintln!("openPedal: interface at {local_url}");
        if opts.lan {
            match lan_address() {
                Some(addr) => eprintln!("  from the phone (same network): http://{addr}:{port}"),
                None => eprintln!("  listening on all interfaces, port {port}"),
            }
        }
        if opts.open_browser {
            if let Err(e) = webbrowser::open(&local_url) {
                eprintln!("  cannot open the browser ({e}): open the address by hand");
            }
        }
        let state = Arc::new(AppState { engine, presets_dir: opts.presets_dir, pedals_dir: opts.pedals_dir });
        axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await.context("server error")
    })
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// The machine's address on the local network (no packet is sent).
fn lan_address() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

/// Rejects WebSockets opened from pages on other sites (the server can write files).
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else { return true };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let origin = origin.to_str().unwrap_or_default();
    origin.split("://").nth(1) == Some(host)
}

async fn ws_handler(ws: WebSocketUpgrade, headers: HeaderMap, State(state): State<Arc<AppState>>) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    ws.on_upgrade(move |socket| client_loop(socket, state))
}

fn to_text(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap_or_default().into())
}

async fn client_loop(socket: WebSocket, state: Arc<AppState>) {
    let (mut tx, mut rx) = socket.split();
    let mut meters = tokio::time::interval(Duration::from_millis(50));
    meters.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = meters.tick() => {
                if tx.send(to_text(&ServerMsg::Meters(state.engine.meters()))).await.is_err() {
                    break;
                }
            }
            incoming = rx.next() => {
                let replies = match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMsg>(&text) {
                        Ok(msg) => handle_message(&state, msg),
                        Err(e) => vec![error(format!("invalid message: {e}"), None)],
                    },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                for reply in replies {
                    if tx.send(to_text(&reply)).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

fn error(message: String, node: Option<String>) -> ServerMsg {
    ServerMsg::Error { message, node }
}

fn error_from(e: anyhow::Error) -> ServerMsg {
    // If the error comes from the graph and concerns a node, the interface highlights it.
    let node = e.chain().find_map(|c| c.downcast_ref::<crate::graph::GraphError>()).and_then(|g| g.node.clone());
    error(format!("{e:#}"), node)
}

pub fn state_msg(state: &AppState) -> ServerMsg {
    ServerMsg::State(StateMsg {
        catalog: state.engine.catalog(),
        graph: state.engine.graph(),
        presets: files::list(&state.presets_dir),
        pedals: crate::graph::load::read_pedals(&state.pedals_dir).unwrap_or_default().into_iter().collect(),
        audio: state.engine.audio(),
    })
}

/// Handles a client message and returns the replies for that client.
pub fn handle_message(state: &AppState, msg: ClientMsg) -> Vec<ServerMsg> {
    let result: Result<Vec<ServerMsg>> = (|| match msg {
        ClientMsg::Hello => Ok(vec![state_msg(state)]),
        ClientMsg::SetParam { node, param, value } => {
            state.engine.set_param(&node, &param, value)?;
            Ok(vec![])
        }
        ClientMsg::SetGraph { graph } => {
            state.engine.set_graph(graph)?;
            Ok(vec![])
        }
        ClientMsg::LoadPreset { name } => {
            let path = files::file_path(&state.presets_dir, &name)?;
            let mut graph = read_graph(&path)?;
            // The interface recognizes the preset by its name: it must be the file's name
            // (v1 presets have names like "Clean" for the file clean.toml).
            graph.name = name;
            state.engine.set_graph(graph)?;
            Ok(vec![state_msg(state)])
        }
        ClientMsg::SavePreset { name, mut graph } => {
            graph.name = name.clone();
            files::save_graph(&state.presets_dir, &name, &graph)?;
            Ok(vec![state_msg(state)])
        }
        ClientMsg::SavePedal { id, pedal } => {
            files::save_pedal(&state.pedals_dir, &id, &pedal)?;
            state.engine.reload_pedals()?;
            Ok(vec![state_msg(state)])
        }
        ClientMsg::DeletePreset { name } => {
            files::delete(&state.presets_dir, &name)?;
            Ok(vec![state_msg(state)])
        }
        ClientMsg::DeletePedal { id } => {
            files::delete(&state.pedals_dir, &id)?;
            state.engine.reload_pedals()?;
            Ok(vec![state_msg(state)])
        }
    })();
    result.unwrap_or_else(|e| vec![error_from(e)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::params::Value;
    use crate::graph::model::PedalFile;
    use crate::server::mock::{default_graph, MockEngine};
    use std::path::Path;

    /// Unique temporary folder for a test.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openpedal-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app(tag: &str) -> (Arc<AppState>, Arc<MockEngine>, PathBuf) {
        let root = temp_dir(tag);
        let (presets, pedals) = (root.join("presets"), root.join("pedals"));
        std::fs::create_dir_all(&pedals).unwrap();
        let engine = Arc::new(MockEngine::new(&pedals, default_graph()));
        let state = Arc::new(AppState { engine: engine.clone(), presets_dir: presets, pedals_dir: pedals });
        (state, engine, root)
    }

    fn dist(graph: &crate::graph::model::GraphFile, id: &str) -> Option<Value> {
        graph.nodes.iter().find(|n| n.id == id)?.params.get("gain_db").cloned()
    }

    fn pedal() -> PedalFile {
        toml::from_str("name = \"Test\"\ncategory = \"drive\"\n").unwrap()
    }

    #[test]
    fn hello_replies_with_state_like_the_fixtures() {
        let (state, _, root) = app("hello");
        let replies = handle_message(&state, ClientMsg::Hello);
        let json = serde_json::to_value(&replies[0]).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../ui/src/fixtures/state.json")).unwrap();
        assert_eq!(json["type"], "state");
        // Same top-level keys, and the real catalog.
        let keys = |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys(&json), keys(&fixture));
        assert_eq!(keys(&json["audio"]), keys(&fixture["audio"]));
        assert_eq!(json["audio"]["running"], false);
        assert_eq!(json["catalog"][0], fixture["catalog"][0]);
        assert_eq!(json["graph"]["node"].as_array().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn set_param_reaches_the_engine() {
        let (state, engine, root) = app("param");
        let mut g = default_graph();
        g.nodes[1].kind = "drive".into();
        g.nodes[1].id = "dist".into();
        g.edges[0].to = "dist".into();
        // The mock graph needs an output: we add one.
        g.nodes.push(crate::graph::model::NodeSpec { id: "out".into(), kind: "output".into(), pos: [440.0, 0.0], params: Default::default() });
        g.edges.push(crate::graph::model::EdgeSpec { from: "dist".into(), to: "out".into() });
        assert!(handle_message(&state, ClientMsg::SetGraph { graph: g }).is_empty());

        let r = handle_message(&state, ClientMsg::SetParam { node: "dist".into(), param: "gain_db".into(), value: 18.0 });
        assert!(r.is_empty());
        assert_eq!(dist(&engine.graph(), "dist"), Some(Value::Number(18.0)));
        // A choice is sent as an index and saved as a name.
        handle_message(&state, ClientMsg::SetParam { node: "dist".into(), param: "shape".into(), value: 1.0 });
        let shape = engine.graph().nodes.iter().find(|n| n.id == "dist").unwrap().params.get("shape").cloned();
        assert!(matches!(shape, Some(Value::Text(_))));
        // Nonexistent node: error message in English.
        let r = handle_message(&state, ClientMsg::SetParam { node: "boh".into(), param: "x".into(), value: 0.0 });
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("does not exist")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dangerous_names_are_rejected() {
        let (state, _, root) = app("nomi");
        for bad in ["../x", "a/b", "", "a.b", "..", "x y", "\\x"] {
            let r = handle_message(&state, ClientMsg::SavePreset { name: bad.into(), graph: default_graph() });
            assert!(matches!(r[0], ServerMsg::Error { .. }), "{bad:?} must be rejected");
            let r = handle_message(&state, ClientMsg::SavePedal { id: bad.into(), pedal: pedal() });
            assert!(matches!(r[0], ServerMsg::Error { .. }));
            let r = handle_message(&state, ClientMsg::DeletePreset { name: bad.into() });
            assert!(matches!(r[0], ServerMsg::Error { .. }));
            let r = handle_message(&state, ClientMsg::DeletePedal { id: bad.into() });
            assert!(matches!(r[0], ServerMsg::Error { .. }));
            let r = handle_message(&state, ClientMsg::LoadPreset { name: bad.into() });
            assert!(matches!(r[0], ServerMsg::Error { .. }));
        }
        assert!(!root.join("x.toml").exists());
        assert!(files::list(&state.presets_dir).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unwritable_dirs_report_errors_instead_of_failing_silently() {
        let (state, _, root) = app("unwritable");
        // A plain file where the folder should be: every write below must fail.
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let state = AppState { presets_dir: blocked.join("presets"), pedals_dir: blocked.join("pedals"), engine: state.engine.clone() };
        let r = handle_message(&state, ClientMsg::SavePreset { name: "x".into(), graph: default_graph() });
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("cannot")), "{r:?}");
        let r = handle_message(&state, ClientMsg::SavePedal { id: "x".into(), pedal: pedal() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "x".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_load_preset_and_pedals() {
        let (state, _, root) = app("file");
        let r = handle_message(&state, ClientMsg::SavePreset { name: "my-sound".into(), graph: default_graph() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert_eq!(s.presets, vec!["my-sound".to_string()]);
        // the saved graph's name is the file's name
        assert_eq!(
            read_graph(Path::new(&state.presets_dir.join("my-sound.toml"))).unwrap(),
            crate::graph::model::GraphFile { name: "my-sound".into(), ..default_graph() }
        );
        let r = handle_message(&state, ClientMsg::LoadPreset { name: "my-sound".into() });
        assert!(matches!(r[0], ServerMsg::State(_)));
        let r = handle_message(&state, ClientMsg::LoadPreset { name: "nonexistent".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "my-sound".into() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert!(s.presets.is_empty() && !state.presets_dir.join("my-sound.toml").exists());
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "my-sound".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }), "deleting a preset that does not exist is an error");

        let r = handle_message(&state, ClientMsg::SavePedal { id: "test".into(), pedal: pedal() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert!(s.catalog.iter().any(|c| c.kind == "pedal:test"));
        let r = handle_message(&state, ClientMsg::DeletePedal { id: "test".into() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert!(!s.catalog.iter().any(|c| c.kind == "pedal:test"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn foreign_origin_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:7777".parse().unwrap());
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, "http://127.0.0.1:7777".parse().unwrap());
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, "http://evil.example".parse().unwrap());
        assert!(!same_origin(&h));
    }

    #[tokio::test]
    async fn real_websocket_hello_and_meters() {
        use tokio_tungstenite::tungstenite::Message as M;
        let (state, _, root) = app("ws");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        ws.send(M::text(r#"{"type":"hello"}"#)).await.unwrap();
        let (mut got_state, mut got_meters) = (false, false);
        for _ in 0..20 {
            let Some(Ok(M::Text(t))) = ws.next().await else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            match v["type"].as_str().unwrap() {
                "state" => got_state = true,
                "meters" => got_meters = true,
                _ => {}
            }
            if got_state && got_meters {
                break;
            }
        }
        assert!(got_state && got_meters);
        ws.send(M::text("not json")).await.unwrap();
        loop {
            let Some(Ok(M::Text(t))) = ws.next().await else { continue };
            if t.contains("\"error\"") {
                break;
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
