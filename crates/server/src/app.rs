//! HTTP routes, WebSocket and message handling (protocol: docs/PLAN.md §5.4).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

use openpedal_protocol::{ClientMsg, ServerMsg, StateMsg, PROTOCOL_VERSION};
use openpedal_graph::{analysis, chain};
use openpedal_graph::model::GraphFile;
use openpedal_library::Library;

use crate::tonelab::{self, Tonelab};
use crate::{assets, EngineHandle};

pub struct ServerOptions {
    pub port: u16,
    pub open_browser: bool,
    /// true = listen on all interfaces (for the phone).
    pub lan: bool,
    pub library: Library,
}

/// Undo steps kept.
const HISTORY_CAP: usize = 100;
/// Messages a slow client may fall behind before it is resynchronized with a fresh `state`.
const BROADCAST_BACKLOG: usize = 256;

/// The shared edit history (one for every connected client) and the "unsaved" reference.
struct Session {
    undo: Vec<GraphFile>,
    redo: Vec<GraphFile>,
    /// The graph as it was at the last load or save (compared without its name).
    saved: GraphFile,
    /// Snapshot taken by `edit_begin`; becomes an undo step at the first `set_param` of the gesture.
    pending_edit: Option<GraphFile>,
    /// `dirty` as last sent, to notice when a plain `set_param` flips it.
    last_dirty: bool,
}

impl Session {
    /// Records `before` as an undo step (any edit clears the redo stack).
    fn push(&mut self, before: GraphFile) {
        self.pending_edit = None;
        self.redo.clear();
        self.undo.push(before);
        if self.undo.len() > HISTORY_CAP {
            self.undo.remove(0);
        }
    }

    fn reset(&mut self, saved: GraphFile) {
        self.undo.clear();
        self.redo.clear();
        self.pending_edit = None;
        self.saved = saved;
    }
}

/// A message for every connected client except `except`.
#[derive(Clone)]
pub(crate) struct Broadcast {
    pub(crate) except: Option<u64>,
    /// Only this client (progress of the step it asked for).
    pub(crate) only: Option<u64>,
    pub(crate) msg: ServerMsg,
}

pub struct AppState {
    pub engine: Arc<dyn EngineHandle>,
    pub library: Library,
    session: Mutex<Session>,
    tx: broadcast::Sender<Broadcast>,
    pub(crate) tonelab: Arc<Tonelab>,
    next_client: AtomicU64,
}

impl AppState {
    pub fn new(engine: Arc<dyn EngineHandle>, library: Library) -> Self {
        Self::with_tonelab_tool(engine, library, std::env::var("OPENPEDAL_TONELAB").ok())
    }

    /// `tool`: what `$OPENPEDAL_TONELAB` says (a path, `mock`, `none`), or `None` to look for the tool in the repository.
    pub fn with_tonelab_tool(engine: Arc<dyn EngineHandle>, library: Library, tool: Option<String>) -> Self {
        let graph = engine.graph();
        let session = Session { undo: vec![], redo: vec![], saved: graph, pending_edit: None, last_dirty: false };
        let tx = broadcast::channel(BROADCAST_BACKLOG).0;
        let tonelab = Arc::new(Tonelab::new(tx.clone(), library.clone(), engine.audio().running, tool, "tools/tonelab/bin/tonelab".into(), tonelab::native_config()));
        Self { engine, library, session: Mutex::new(session), tx, tonelab, next_client: AtomicU64::new(1) }
    }

    fn lock(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Same nodes and cables (the name does not count: saving renames the graph).
fn same_content(a: &GraphFile, b: &GraphFile) -> bool {
    a.nodes == b.nodes && a.edges == b.edges
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/tonelab/file/{job}/{*path}", get(tonelab::serve_file))
        .route("/tonelab/upload", post(tonelab::upload).layer(tonelab::upload_limit()))
        .fallback(assets::serve).with_state(state)
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
        let state = Arc::new(AppState::new(engine, opts.library));
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
pub(crate) fn same_origin(headers: &HeaderMap) -> bool {
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
    let id = state.next_client.fetch_add(1, Ordering::Relaxed);
    // Subscribe first, so nothing another client does can slip between connecting and `hello`.
    let mut updates = state.tx.subscribe();
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
            update = updates.recv() => {
                let msg = match update {
                    Ok(b) if b.except == Some(id) || b.only.is_some_and(|o| o != id) => continue,
                    Ok(b) => b.msg,
                    // Fell too far behind: whatever was missed is replaced by the current state.
                    Err(broadcast::error::RecvError::Lagged(_)) => state_msg(&state),
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if tx.send(to_text(&msg)).await.is_err() {
                    break;
                }
            }
            incoming = rx.next() => {
                let replies = match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMsg>(&text) {
                        Ok(msg) => handle_message_from(&state, id, msg),
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
    let node = e.chain().find_map(|c| c.downcast_ref::<openpedal_graph::GraphError>()).and_then(|g| g.node.clone());
    error(format!("{e:#}"), node)
}

fn build_state(state: &AppState, session: &Session) -> ServerMsg {
    let graph = state.engine.graph();
    let dirty = !same_content(&graph, &session.saved);
    ServerMsg::State(StateMsg {
        protocol: PROTOCOL_VERSION,
        catalog: state.engine.catalog(),
        graph,
        presets: state.library.list_presets(),
        pedals: state.library.load_pedals().unwrap_or_default().into_iter().collect(),
        audio: state.engine.audio(),
        can_undo: !session.undo.is_empty(),
        can_redo: !session.redo.is_empty(),
        dirty,
    })
}

pub fn state_msg(state: &AppState) -> ServerMsg {
    build_state(state, &state.lock())
}

/// Handles a message from an anonymous client (tests, embedding): nobody is excluded from broadcasts.
#[cfg(test)]
pub fn handle_message(state: &AppState, msg: ClientMsg) -> Vec<ServerMsg> {
    handle_message_from(state, 0, msg)
}

/// Sends `msg` to every client but `from`.
fn broadcast_others(state: &AppState, from: u64, msg: ServerMsg) {
    // Err = nobody is listening, which is fine.
    let _ = state.tx.send(Broadcast { except: Some(from), only: None, msg });
}

/// The current state: goes in the sender's replies and to every other client.
fn publish(state: &AppState, session: &mut Session, from: u64) -> Vec<ServerMsg> {
    let msg = build_state(state, session);
    if let ServerMsg::State(s) = &msg {
        session.last_dirty = s.dirty;
    }
    broadcast_others(state, from, msg.clone());
    vec![msg]
}

/// `analyze_pedal` / `analyze_node`: renders the effect offline at the engine's sample rate.
fn analyze(state: &AppState, msg: ClientMsg) -> Result<ServerMsg> {
    let sample_rate = state.engine.audio().sample_rate as f32;
    let pedals = state.library.load_pedals()?;
    let response = match msg {
        ClientMsg::AnalyzePedal { pedal, values } => analysis::analyze_pedal(&pedal, &values, &pedals, sample_rate)?,
        ClientMsg::AnalyzeNode { node } => {
            let graph = state.engine.graph();
            let spec = graph.nodes.iter().find(|n| n.id == node).with_context(|| format!("node '{node}' does not exist"))?;
            analysis::analyze_node(spec, &pedals, sample_rate).map_err(|e| anyhow::Error::new(openpedal_graph::GraphError { message: format!("{e:#}"), node: Some(node.clone()) }))?
        }
        _ => unreachable!("only analysis requests come here"),
    };
    Ok(ServerMsg::Response { freq: response.freq, db: response.db })
}

/// Applies a new graph produced by an edit, as one undo step.
fn edit_graph(state: &AppState, session: &mut Session, next: GraphFile) -> Result<()> {
    let before = state.engine.graph();
    state.engine.set_graph(next)?;
    session.push(before);
    Ok(())
}

/// Moves one step through the history: `undo` pops from `session.undo`, otherwise from `redo`.
fn travel(state: &AppState, session: &mut Session, undo: bool) -> Result<()> {
    let target = if undo { session.undo.pop() } else { session.redo.pop() };
    let Some(mut target) = target else { anyhow::bail!("nothing to {}", if undo { "undo" } else { "redo" }) };
    let current = state.engine.graph();
    // The name follows the preset (saving renames the graph), not the history.
    target.name = current.name.clone();
    if let Err(e) = state.engine.set_graph(target.clone()) {
        if undo { session.undo.push(target) } else { session.redo.push(target) }
        return Err(e);
    }
    session.pending_edit = None;
    if undo { session.redo.push(current) } else { session.undo.push(current) }
    Ok(())
}

/// Handles a client message and returns the replies for that client. Changes other clients must
/// see are broadcast to them (`from` identifies the sender, which never gets its own `param` back).
pub fn handle_message_from(state: &AppState, from: u64, msg: ClientMsg) -> Vec<ServerMsg> {
    // Analysis only reads: it needs neither the history lock nor a broadcast, and renders offline
    // (a few milliseconds), so it never delays edits or the audio thread.
    if matches!(msg, ClientMsg::AnalyzePedal { .. } | ClientMsg::AnalyzeNode { .. }) {
        return vec![analyze(state, msg).unwrap_or_else(error_from)];
    }
    // Tone Lab steps run on their own threads and answer through the broadcast channel.
    if !matches!(msg, ClientMsg::TonelabSavePreset { .. })
        && matches!(
            msg,
            ClientMsg::TonelabCaps
                | ClientMsg::TonelabJobs
                | ClientMsg::TonelabStart { .. }
                | ClientMsg::TonelabOpen { .. }
                | ClientMsg::TonelabPick { .. }
                | ClientMsg::TonelabRecordDi { .. }
                | ClientMsg::TonelabFit { .. }
                | ClientMsg::TonelabCancel { .. }
        )
    {
        state.tonelab.handle(from, msg);
        return vec![];
    }
    // One edit at a time: history, engine and broadcast order stay consistent.
    let mut session = state.lock();
    let session = &mut *session;
    let result: Result<Vec<ServerMsg>> = (|| match msg {
        ClientMsg::Hello => Ok(vec![build_state(state, session)]),
        ClientMsg::SetParam { node, param, value } => {
            let before = state.engine.graph();
            state.engine.set_param(&node, &param, value)?;
            let after = state.engine.graph();
            let is_choice = after
                .nodes
                .iter()
                .find(|n| n.id == node)
                .is_some_and(|n| matches!(n.params.get(&param), Some(openpedal_dsp::params::Value::Text(_))));
            let mut pushed = false;
            if is_choice {
                session.push(before);
                pushed = true;
            } else if let Some(begin) = session.pending_edit.take() {
                session.push(begin);
                pushed = true;
            }
            if pushed || session.last_dirty != !same_content(&after, &session.saved) {
                return Ok(publish(state, session, from));
            }
            broadcast_others(state, from, ServerMsg::Param { node, param, value });
            Ok(vec![])
        }
        ClientMsg::SetBypass { node, bypass } => {
            let before = state.engine.graph();
            state.engine.set_bypass(&node, bypass)?;
            session.push(before);
            Ok(publish(state, session, from))
        }
        ClientMsg::SetGraph { graph } => {
            let before = state.engine.graph();
            state.engine.set_graph(graph.clone())?;
            if before != graph {
                session.push(before);
            }
            // The sender already has the graph it sent; the others get the new state.
            publish(state, session, from);
            Ok(vec![])
        }
        ClientMsg::AddNode { kind, after } => {
            if !state.engine.catalog().iter().any(|c| c.kind == kind) {
                anyhow::bail!("unknown type '{kind}'");
            }
            let (next, id) = chain::add_node(&state.engine.graph(), &kind, after.as_deref())?;
            edit_graph(state, session, next)?;
            let mut replies = publish(state, session, from);
            replies.push(ServerMsg::Added { node: id });
            Ok(replies)
        }
        ClientMsg::RemoveNode { node } => {
            let next = chain::remove_node(&state.engine.graph(), &node)?;
            edit_graph(state, session, next)?;
            Ok(publish(state, session, from))
        }
        ClientMsg::MoveNode { node, to } => {
            let current = state.engine.graph();
            let next = chain::move_node(&current, &node, to as usize)?;
            // Dropping a pedal where it already is changes nothing.
            if next.nodes.iter().map(|n| &n.id).ne(current.nodes.iter().map(|n| &n.id)) {
                edit_graph(state, session, next)?;
            }
            Ok(publish(state, session, from))
        }
        ClientMsg::AnalyzePedal { .. } | ClientMsg::AnalyzeNode { .. } => unreachable!("handled above"),
        ClientMsg::Undo => {
            travel(state, session, true)?;
            Ok(publish(state, session, from))
        }
        ClientMsg::Redo => {
            travel(state, session, false)?;
            Ok(publish(state, session, from))
        }
        ClientMsg::EditBegin => {
            session.pending_edit = Some(state.engine.graph());
            Ok(vec![])
        }
        ClientMsg::LoadPreset { name } => {
            // The library names the graph after the file (v1 presets have names like "Clean"
            // for the file clean.toml): the interface recognizes the preset by that name.
            let graph = state.library.load_preset(&name)?;
            state.engine.set_graph(graph.clone())?;
            session.reset(graph);
            Ok(publish(state, session, from))
        }
        ClientMsg::SavePreset { name, graph } => {
            state.library.save_preset(&name, &graph)?;
            // The playing graph now is that preset.
            state.engine.set_graph_name(&name);
            session.saved = graph;
            Ok(publish(state, session, from))
        }
        ClientMsg::SavePedal { id, pedal } => {
            state.library.save_pedal(&id, &pedal)?;
            state.engine.reload_pedals()?;
            Ok(publish(state, session, from))
        }
        ClientMsg::DeletePreset { name } => {
            state.library.delete_preset(&name)?;
            Ok(publish(state, session, from))
        }
        ClientMsg::DeletePedal { id } => {
            state.library.delete_pedal(&id)?;
            state.engine.reload_pedals()?;
            Ok(publish(state, session, from))
        }
        ClientMsg::TonelabSavePreset { job, part, name } => {
            state.tonelab.save_preset(&job, &part, &name)?;
            Ok(publish(state, session, from))
        }
        _ => unreachable!("handled before the lock"),
    })();
    result.unwrap_or_else(|e| vec![error_from(e)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpedal_dsp::params::Value;
    use openpedal_graph::model::PedalFile;
    use crate::mock::{default_graph, MockEngine};
    use std::path::PathBuf;

    /// Unique temporary folder for a test.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openpedal-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app(tag: &str) -> (Arc<AppState>, Arc<MockEngine>, PathBuf) {
        let root = temp_dir(tag);
        let library = Library::new(&root);
        std::fs::create_dir_all(library.pedals_dir()).unwrap();
        let engine = Arc::new(MockEngine::new(library.pedals_dir(), default_graph()));
        let state = Arc::new(AppState::new(engine.clone(), library));
        (state, engine, root)
    }

    fn dist(graph: &openpedal_graph::model::GraphFile, id: &str) -> Option<Value> {
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
        let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../apps/desktop/ui/src/fixtures/state.json")).unwrap();
        assert_eq!(json["type"], "state");
        assert_eq!(json["protocol"], PROTOCOL_VERSION);
        assert_eq!(fixture["protocol"], PROTOCOL_VERSION);
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
    fn set_bypass_updates_the_graph_and_rejects_input_output() {
        let (state, engine, root) = app("bypass");
        let mut g = default_graph();
        g.nodes[1].kind = "drive".into();
        g.nodes[1].id = "dist".into();
        g.nodes.push(openpedal_graph::model::NodeSpec { id: "out".into(), kind: "output".into(), pos: [440.0, 0.0], params: Default::default(), bypass: false });
        assert!(handle_message(&state, ClientMsg::SetGraph { graph: g }).is_empty());
        assert!(matches!(handle_message(&state, ClientMsg::SetBypass { node: "dist".into(), bypass: true })[0], ServerMsg::State(_)));
        assert!(engine.graph().nodes.iter().find(|n| n.id == "dist").unwrap().bypass);
        let json = serde_json::to_value(engine.graph()).unwrap();
        assert_eq!(json["node"][1]["bypass"], true);
        assert!(matches!(handle_message(&state, ClientMsg::SetBypass { node: "dist".into(), bypass: false })[0], ServerMsg::State(_)));
        assert!(!engine.graph().nodes.iter().find(|n| n.id == "dist").unwrap().bypass);
        for node in ["in", "out"] {
            let r = handle_message(&state, ClientMsg::SetBypass { node: node.into(), bypass: true });
            assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("cannot be bypassed")), "{node}: {r:?}");
        }
        let r = handle_message(&state, ClientMsg::SetBypass { node: "nope".into(), bypass: true });
        assert!(matches!(&r[0], ServerMsg::Error { .. }));
        // wire format
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"set_bypass","node":"dist","bypass":true}"#).unwrap();
        assert!(matches!(msg, ClientMsg::SetBypass { bypass: true, .. }));
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
        g.nodes.push(openpedal_graph::model::NodeSpec { id: "out".into(), kind: "output".into(), pos: [440.0, 0.0], params: Default::default(), bypass: false });
        g.edges.push(openpedal_graph::model::EdgeSpec { from: "dist".into(), to: "out".into() });
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
        assert!(state.library.list_presets().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unwritable_dirs_report_errors_instead_of_failing_silently() {
        let (state, _, root) = app("unwritable");
        // A plain file where the folder should be: every write below must fail.
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let state = AppState::new(state.engine.clone(), Library::new(&blocked));
        let r = handle_message(&state, ClientMsg::SavePreset { name: "x".into(), graph: default_graph() });
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("cannot")), "{r:?}");
        let r = handle_message(&state, ClientMsg::SavePedal { id: "x".into(), pedal: pedal() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "x".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let _ = std::fs::remove_dir_all(root);
    }

    const LOWPASS_PEDAL: &str = "name = \"LP\"\n[[node]]\nid = \"f\"\ntype = \"filter\"\nparams = { kind = \"lowpass\", freq = 1000 }\n[[edge]]\nfrom = \"pedal_in\"\nto = \"f\"\n[[edge]]\nfrom = \"f\"\nto = \"pedal_out\"\n[[control]]\nid = \"cut\"\nkind = \"knob\"\n[[control.route]]\ntarget = \"f.freq\"\nmap = \"log\"\nmin = 200\nmax = 5000\n";

    fn response(r: &[ServerMsg]) -> (&[f32], &[f32]) {
        match r {
            [ServerMsg::Response { freq, db }] => (freq, db),
            other => panic!("expected one response, got {}", serde_json::to_string(other).unwrap()),
        }
    }

    #[test]
    fn analyze_pedal_answers_only_the_asker_and_works_for_unsaved_drafts() {
        let (state, _, root) = app("analyze-pedal");
        let draft: PedalFile = toml::from_str(LOWPASS_PEDAL).unwrap();
        let mut rx = state.tx.subscribe();
        let r = handle_message(&state, ClientMsg::AnalyzePedal { pedal: draft.clone(), values: Default::default() });
        let (freq, db) = response(&r);
        assert_eq!((freq.len(), db.len()), (128, 128));
        assert!(db[0].abs() < 0.5 && db[127] < -30.0, "lowpass shape: {} .. {}", db[0], db[127]);
        assert!(rx.try_recv().is_err(), "nothing is broadcast");
        assert!(state.library.list_pedals().is_empty(), "the draft was not saved");
        // control values
        let open = handle_message(&state, ClientMsg::AnalyzePedal { pedal: draft.clone(), values: [("cut".to_string(), 1.0)].into() });
        assert!(response(&open).1[90] > db[90] + 10.0, "a higher cutoff passes more at the same frequency");
        // errors name the problem
        let r = handle_message(&state, ClientMsg::AnalyzePedal { pedal: draft, values: [("nope".to_string(), 1.0)].into() });
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("no control 'nope'")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn analyze_node_uses_the_playing_graph() {
        let (state, _, root) = app("analyze-node");
        let mut g = default_graph();
        let node = |id: &str, kind: &str, text: &str| openpedal_graph::model::NodeSpec { id: id.into(), kind: kind.into(), pos: [0.0, 0.0], params: toml::from_str(text).unwrap(), bypass: false };
        g.nodes.insert(1, node("loud", "gain", "db = 12.0"));
        assert!(handle_message(&state, ClientMsg::SetGraph { graph: g }).is_empty());
        let r = handle_message(&state, ClientMsg::AnalyzeNode { node: "loud".into() });
        assert!(response(&r).1.iter().all(|d| (d - 12.0).abs() < 0.1));
        for bad in ["nope", "in"] {
            let r = handle_message(&state, ClientMsg::AnalyzeNode { node: bad.into() });
            assert!(matches!(&r[0], ServerMsg::Error { .. }), "{bad}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_pedals_are_refused_when_saved() {
        let (state, _, root) = app("save-invalid");
        let mut bad: PedalFile = toml::from_str(LOWPASS_PEDAL).unwrap();
        bad.controls[0].routes[0].target = "f.nothing".into();
        let r = handle_message(&state, ClientMsg::SavePedal { id: "bad".into(), pedal: bad });
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("no parameter 'nothing'")), "{}", serde_json::to_string(&r).unwrap());
        assert!(state.library.list_pedals().is_empty());
        let good: PedalFile = toml::from_str(LOWPASS_PEDAL).unwrap();
        assert!(matches!(handle_message(&state, ClientMsg::SavePedal { id: "lp".into(), pedal: good })[0], ServerMsg::State(_)));
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
            state.library.load_preset("my-sound").unwrap(),
            openpedal_graph::model::GraphFile { name: "my-sound".into(), ..default_graph() }
        );
        let r = handle_message(&state, ClientMsg::LoadPreset { name: "my-sound".into() });
        assert!(matches!(r[0], ServerMsg::State(_)));
        let r = handle_message(&state, ClientMsg::LoadPreset { name: "nonexistent".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }));
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "my-sound".into() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert!(s.presets.is_empty() && !state.library.presets_dir().join("my-sound.toml").exists());
        let r = handle_message(&state, ClientMsg::DeletePreset { name: "my-sound".into() });
        assert!(matches!(r[0], ServerMsg::Error { .. }), "deleting a preset that does not exist is an error");

        let r = handle_message(&state, ClientMsg::SavePedal { id: "test".into(), pedal: pedal() });
        let ServerMsg::State(s) = &r[0] else { panic!("expected state") };
        assert!(s.catalog.iter().any(|c| c.kind == "pedal:test"));
        assert!(s.pedals.contains_key("test"));
        assert!(state.library.list_pedals() == ["test"], "the pedal is saved through the library");
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

    // --- chain edits, history, broadcast ---

    fn state_of(r: &[ServerMsg]) -> &StateMsg {
        r.iter().find_map(|m| if let ServerMsg::State(s) = m { Some(s) } else { None }).unwrap_or_else(|| panic!("no state in {r:?}"))
    }

    fn ids(engine: &MockEngine) -> Vec<String> {
        engine.graph().nodes.into_iter().map(|n| n.id).collect()
    }

    fn add(state: &AppState, kind: &str, after: Option<&str>) -> Vec<ServerMsg> {
        handle_message(state, ClientMsg::AddNode { kind: kind.into(), after: after.map(Into::into) })
    }

    fn gain(engine: &MockEngine, id: &str) -> Option<Value> {
        dist(&engine.graph(), id)
    }

    fn set_gain(state: &AppState, id: &str, v: f32) -> Vec<ServerMsg> {
        handle_message(state, ClientMsg::SetParam { node: id.into(), param: "gain_db".into(), value: v })
    }

    #[test]
    fn add_remove_move_on_a_linear_chain() {
        let (state, engine, root) = app("chain");
        let r = add(&state, "drive", None);
        assert_eq!(r.len(), 2);
        assert!(matches!(&r[1], ServerMsg::Added { node } if node == "drive"));
        assert_eq!(state_of(&r).graph.nodes.len(), 3);
        add(&state, "delay", None);
        let r = add(&state, "drive", Some("in"));
        assert!(matches!(&r[1], ServerMsg::Added { node } if node == "drive2"), "{r:?}");
        assert_eq!(ids(&engine), ["in", "drive2", "drive", "delay", "out"]);
        assert_eq!(engine.graph().edges.len(), 4);

        let r = handle_message(&state, ClientMsg::MoveNode { node: "delay".into(), to: 0 });
        assert_eq!(state_of(&r).graph.nodes[1].id, "delay");
        assert_eq!(ids(&engine), ["in", "delay", "drive2", "drive", "out"]);
        handle_message(&state, ClientMsg::MoveNode { node: "delay".into(), to: 99 });
        assert_eq!(ids(&engine), ["in", "drive2", "drive", "delay", "out"]);
        handle_message(&state, ClientMsg::RemoveNode { node: "drive2".into() });
        assert_eq!(ids(&engine), ["in", "drive", "delay", "out"]);

        for bad in [
            ClientMsg::RemoveNode { node: "in".into() },
            ClientMsg::RemoveNode { node: "nope".into() },
            ClientMsg::MoveNode { node: "out".into(), to: 0 },
        ] {
            assert!(matches!(handle_message(&state, bad)[0], ServerMsg::Error { .. }));
        }
        let r = add(&state, "no-such-effect", None);
        assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("unknown type")));
        assert!(matches!(&add(&state, "input", None)[0], ServerMsg::Error { .. }));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn chain_edits_refuse_non_linear_graphs() {
        let (state, engine, root) = app("nonlinear");
        add(&state, "drive", None);
        let mut g = engine.graph();
        g.edges.push(openpedal_graph::model::EdgeSpec { from: "in".into(), to: "out".into() });
        handle_message(&state, ClientMsg::SetGraph { graph: g.clone() });
        for msg in [
            ClientMsg::AddNode { kind: "delay".into(), after: None },
            ClientMsg::RemoveNode { node: "drive".into() },
            ClientMsg::MoveNode { node: "drive".into(), to: 0 },
        ] {
            let r = handle_message(&state, msg);
            assert!(matches!(&r[0], ServerMsg::Error { message, .. } if message.contains("parallel routing")), "{r:?}");
        }
        assert_eq!(engine.graph(), g);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn undo_redo_and_flags() {
        let (state, engine, root) = app("undo");
        let r = handle_message(&state, ClientMsg::Hello);
        let s = state_of(&r);
        assert_eq!((s.can_undo, s.can_redo, s.dirty, s.protocol), (false, false, false, PROTOCOL_VERSION));
        assert!(matches!(handle_message(&state, ClientMsg::Undo)[0], ServerMsg::Error { .. }));
        assert!(matches!(handle_message(&state, ClientMsg::Redo)[0], ServerMsg::Error { .. }));

        let r = add(&state, "drive", None);
        let s = state_of(&r);
        assert_eq!((s.can_undo, s.can_redo, s.dirty), (true, false, true));
        let r = handle_message(&state, ClientMsg::Undo);
        let s = state_of(&r);
        assert_eq!((s.can_undo, s.can_redo, s.dirty), (false, true, false), "back to the loaded graph is clean");
        assert_eq!(ids(&engine), ["in", "out"]);
        let r = handle_message(&state, ClientMsg::Redo);
        assert_eq!((state_of(&r).can_undo, state_of(&r).can_redo), (true, false));
        assert_eq!(ids(&engine), ["in", "drive", "out"]);

        // a new edit clears the redo stack
        handle_message(&state, ClientMsg::Undo);
        add(&state, "delay", None);
        assert!(!state_of(&handle_message(&state, ClientMsg::Hello)).can_redo);

        // bypass is a step
        handle_message(&state, ClientMsg::SetBypass { node: "delay".into(), bypass: true });
        handle_message(&state, ClientMsg::Undo);
        assert!(!engine.graph().nodes.iter().find(|n| n.id == "delay").unwrap().bypass);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn edit_begin_makes_a_whole_drag_one_undo_step() {
        let (state, engine, root) = app("drag");
        add(&state, "drive", None);
        set_gain(&state, "drive", 1.0);
        handle_message(&state, ClientMsg::EditBegin);
        // no parameter change yet: no step, nothing to undo beyond the previous ones
        assert_eq!(state.lock().undo.len(), 1, "add node only");
        for v in [3.0, 6.0, 9.0, 12.0] {
            set_gain(&state, "drive", v);
        }
        assert_eq!(state.lock().undo.len(), 2);
        handle_message(&state, ClientMsg::Undo);
        assert_eq!(gain(&engine, "drive"), Some(Value::Number(1.0)), "the drag is undone in one step");
        handle_message(&state, ClientMsg::Undo);
        assert_eq!(ids(&engine), ["in", "out"]);
        let r = handle_message(&state, ClientMsg::Redo);
        assert!(state_of(&r).can_redo);

        // a begin without movement leaves no empty step
        handle_message(&state, ClientMsg::EditBegin);
        handle_message(&state, ClientMsg::EditBegin);
        assert_eq!(state.lock().undo.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn choices_are_undo_steps_and_numbers_alone_are_not() {
        let (state, engine, root) = app("choice");
        add(&state, "drive", None);
        let before = state.lock().undo.len();
        set_gain(&state, "drive", 5.0);
        set_gain(&state, "drive", 6.0);
        assert_eq!(state.lock().undo.len(), before, "a number without edit_begin is not a step");
        handle_message(&state, ClientMsg::SetParam { node: "drive".into(), param: "shape".into(), value: 1.0 });
        assert_eq!(state.lock().undo.len(), before + 1);
        handle_message(&state, ClientMsg::Undo);
        assert!(engine.graph().nodes[1].params.get("shape").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn history_is_capped_and_load_preset_clears_it() {
        let (state, _, root) = app("cap");
        add(&state, "drive", None);
        for i in 0..150 {
            handle_message(&state, ClientMsg::SetBypass { node: "drive".into(), bypass: i % 2 == 0 });
        }
        assert_eq!(state.lock().undo.len(), 100);
        handle_message(&state, ClientMsg::SavePreset { name: "p".into(), graph: engine_graph(&state) });
        assert!(state_of(&handle_message(&state, ClientMsg::Hello)).can_undo, "saving keeps the history");
        let r = handle_message(&state, ClientMsg::LoadPreset { name: "p".into() });
        let s = state_of(&r);
        assert_eq!((s.can_undo, s.can_redo, s.dirty), (false, false, false));
        let _ = std::fs::remove_dir_all(root);
    }

    fn engine_graph(state: &AppState) -> openpedal_graph::model::GraphFile {
        state.engine.graph()
    }

    #[test]
    fn dirty_follows_edits_saves_and_loads() {
        let (state, _, root) = app("dirty");
        let dirty = |state: &AppState| state_of(&handle_message(state, ClientMsg::Hello)).dirty;
        assert!(!dirty(&state));
        add(&state, "drive", None);
        assert!(dirty(&state));
        let r = handle_message(&state, ClientMsg::SavePreset { name: "mine".into(), graph: engine_graph(&state) });
        let s = state_of(&r);
        assert!(!s.dirty);
        assert_eq!(s.graph.name, "mine", "the playing graph takes the preset's name");
        set_gain(&state, "drive", 4.0);
        assert!(dirty(&state), "a plain parameter change makes it dirty");
        handle_message(&state, ClientMsg::LoadPreset { name: "mine".into() });
        assert!(!dirty(&state));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn set_graph_is_one_undo_step_and_reaches_the_others() {
        let (state, engine, root) = app("setgraph");
        let mut rx = state.tx.subscribe();
        let mut g = default_graph();
        g.nodes[0].pos = [10.0, 10.0];
        assert!(handle_message_from(&state, 1, ClientMsg::SetGraph { graph: g.clone() }).is_empty());
        let b = rx.try_recv().unwrap();
        assert_eq!(b.except, Some(1));
        let ServerMsg::State(s) = b.msg else { panic!("expected state") };
        assert!(s.can_undo);
        assert_eq!(engine.graph(), g);
        // the same graph again is not a step
        handle_message_from(&state, 1, ClientMsg::SetGraph { graph: g });
        assert_eq!(state.lock().undo.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn changes_are_broadcast_and_set_param_is_not_echoed() {
        let (state, _, root) = app("broadcast");
        add(&state, "drive", None);
        let mut rx = state.tx.subscribe();

        let r = handle_message_from(&state, 1, ClientMsg::MoveNode { node: "drive".into(), to: 0 });
        assert!(matches!(r[0], ServerMsg::State(_)), "the sender gets the state in its replies");
        let b = rx.try_recv().unwrap();
        assert_eq!(b.except, Some(1), "and the others through the broadcast");
        assert!(matches!(b.msg, ServerMsg::State(_)));

        // the first change flips `dirty`/`can_undo` (state for everybody)...
        handle_message_from(&state, 1, ClientMsg::EditBegin);
        let r = handle_message_from(&state, 1, ClientMsg::SetParam { node: "drive".into(), param: "gain_db".into(), value: 3.0 });
        assert!(matches!(r[0], ServerMsg::State(_)));
        assert!(matches!(rx.try_recv().unwrap().msg, ServerMsg::State(_)));
        // ...the rest of the drag is light `param` messages, never sent back to the sender
        let r = handle_message_from(&state, 1, ClientMsg::SetParam { node: "drive".into(), param: "gain_db".into(), value: 6.0 });
        assert!(r.is_empty());
        let b = rx.try_recv().unwrap();
        assert_eq!(b.except, Some(1));
        assert!(matches!(&b.msg, ServerMsg::Param { node, param, value } if node == "drive" && param == "gain_db" && *value == 6.0));
        assert!(rx.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    async fn next_json<S>(ws: &mut S, wait_ms: u64) -> Option<serde_json::Value>
    where
        S: futures_util::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        use tokio_tungstenite::tungstenite::Message as M;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
        loop {
            let m = tokio::time::timeout_at(deadline, ws.next()).await.ok()??.ok()?;
            if let M::Text(t) = m {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["type"] != "meters" {
                    return Some(v);
                }
            }
        }
    }

    #[tokio::test]
    async fn two_websocket_clients_stay_in_sync() {
        use tokio_tungstenite::tungstenite::Message as M;
        let (state, _, root) = app("ws2");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        let url = format!("ws://127.0.0.1:{port}/ws");
        let (mut a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let (mut b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        for ws in [&mut a, &mut b] {
            ws.send(M::text(r#"{"type":"hello"}"#)).await.unwrap();
            assert_eq!(next_json(ws, 2000).await.unwrap()["type"], "state");
        }

        a.send(M::text(r#"{"type":"add_node","kind":"drive"}"#)).await.unwrap();
        let sa = next_json(&mut a, 2000).await.unwrap();
        assert_eq!((sa["type"].as_str(), sa["graph"]["node"].as_array().map(Vec::len)), (Some("state"), Some(3)));
        assert_eq!(next_json(&mut a, 2000).await.unwrap(), serde_json::json!({"type": "added", "node": "drive"}));
        let sb = next_json(&mut b, 2000).await.unwrap();
        assert_eq!(sb["type"], "state");
        assert_eq!(sb["graph"], sa["graph"]);
        assert_eq!(sb["can_undo"], true);
        assert!(next_json(&mut b, 200).await.is_none(), "only the asking client gets `added`");

        // first change: state for everybody; then `param` for B only
        a.send(M::text(r#"{"type":"edit_begin"}"#)).await.unwrap();
        a.send(M::text(r#"{"type":"set_param","node":"drive","param":"gain_db","value":3}"#)).await.unwrap();
        a.send(M::text(r#"{"type":"set_param","node":"drive","param":"gain_db","value":9}"#)).await.unwrap();
        assert_eq!(next_json(&mut a, 2000).await.unwrap()["type"], "state");
        assert!(next_json(&mut a, 300).await.is_none(), "set_param is not echoed to its sender");
        assert_eq!(next_json(&mut b, 2000).await.unwrap()["type"], "state");
        let p = next_json(&mut b, 2000).await.unwrap();
        assert_eq!(p, serde_json::json!({"type": "param", "node": "drive", "param": "gain_db", "value": 9.0}));

        // and B can undo for both
        b.send(M::text(r#"{"type":"undo"}"#)).await.unwrap();
        for ws in [&mut a, &mut b] {
            let s = next_json(ws, 2000).await.unwrap();
            assert_eq!(s["type"], "state");
            assert_eq!(s["can_redo"], true);
        }
        let _ = std::fs::remove_dir_all(root);
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

    // ---- Tone Lab ----

    /// A stand-in for `tonelab`: creates `JOBS/fake-song`, can be slow (`JOBS/slow` exists).
    fn fake_tool(root: &std::path::Path, jobs: &std::path::Path) -> PathBuf {
        let script = format!(
            r#"#!/bin/sh
J="{jobs}"
case "$1" in
  caps) echo '{{"youtube":false,"device":"cpu","jobs_dir":"{jobs}"}}' ;;
  fetch)
    echo "PROGRESS fetch - reading" >&2
    mkdir -p "$J/fake-song"
    echo '{{"id":"fake-song","source":{{"kind":"file","ref":"x","title":"Fake"}},"created":"2026-11-24T10:00:00+01:00","duration_s":5}}' > "$J/fake-song/job.json"
    echo fake-song ;;
  separate)
    echo "PROGRESS separate 0.5 half way" >&2
    [ -e "$J/slow" ] && exec sleep 30
    echo "not a progress line" >&2 ;;
  analyse)
    echo '{{"tempo_bpm":100,"key":"C major","sections":[{{"start":0,"end":4,"guitar_db":-20,"corr":0.9,"kind":"centre"}}]}}' > "$J/fake-song/analysis.json" ;;
  *) echo "tonelab: unknown command $1" >&2; exit 2 ;;
esac
"#,
            jobs = jobs.display()
        );
        let path = root.join("fake-tonelab");
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

    async fn serve(state: Arc<AppState>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        port
    }

    async fn connect(port: u16) -> Ws {
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await.unwrap().0
    }

    async fn send(ws: &mut Ws, json: &str) {
        ws.send(tokio_tungstenite::tungstenite::Message::text(json)).await.unwrap();
    }

    /// Messages of the given types, until one of `until` arrives (meters are skipped). Returns everything seen.
    async fn collect_until(ws: &mut Ws, until: &str) -> Vec<serde_json::Value> {
        let mut seen = vec![];
        for _ in 0..400 {
            let Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) = tokio::time::timeout(Duration::from_secs(10), ws.next()).await else { break };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["type"] == "meters" {
                continue;
            }
            let hit = v["type"] == until || (until == "cancelled" && v["stage"] == "cancelled");
            seen.push(v);
            if hit {
                return seen;
            }
        }
        panic!("never saw {until}: {seen:?}");
    }

    async fn http(port: u16, request: &str) -> (String, Vec<u8>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&buf[..split]).to_string(), buf[split + 4..].to_vec())
    }

    #[tokio::test]
    async fn tonelab_demo_answers_every_message() {
        let root = temp_dir("tl-demo");
        let library = Library::new(&root);
        std::fs::create_dir_all(library.pedals_dir()).unwrap();
        let engine = Arc::new(MockEngine::new(library.pedals_dir(), default_graph()));
        let state = Arc::new(AppState::with_tonelab_tool(engine, library.clone(), Some("mock".into())));
        let port = serve(state).await;
        let (mut a, mut b) = (connect(port).await, connect(port).await);

        send(&mut a, r#"{"type":"tonelab_caps"}"#).await;
        let m = collect_until(&mut a, "tonelab_caps").await.pop().unwrap();
        assert_eq!((m["available"].as_bool(), m["reason"].is_null()), (Some(true), true));
        send(&mut a, r#"{"type":"tonelab_jobs"}"#).await;
        assert_eq!(collect_until(&mut a, "tonelab_jobs").await.pop().unwrap()["jobs"][0]["id"], crate::tonelab_mock::DEMO_JOB);
        send(&mut a, &format!(r#"{{"type":"tonelab_open","job":"{}"}}"#, crate::tonelab_mock::DEMO_JOB)).await;
        let job = collect_until(&mut a, "tonelab_job").await.pop().unwrap();
        assert_eq!(job["job"]["parts"].as_array().unwrap().len(), 1);
        assert!(job["job"]["parts"][0]["fit"]["template"].is_string());

        // Pick: progress only to the asker, the job to everyone.
        send(&mut a, &format!(r#"{{"type":"tonelab_pick","job":"{}","start":30.0,"end":40.0,"channel":"left","label":"lead"}}"#, crate::tonelab_mock::DEMO_JOB)).await;
        let seen = collect_until(&mut a, "tonelab_job").await;
        assert!(seen.iter().any(|m| m["type"] == "tonelab_progress" && m["stage"] == "parts"));
        assert!(seen.iter().any(|m| m["type"] == "tonelab_progress" && m["stage"] == "synth"));
        let job = seen.last().unwrap();
        assert_eq!((job["job"]["parts"][1]["id"].as_str(), job["job"]["parts"][1]["channel"].as_str()), (Some("p2"), Some("left")));
        let other = collect_until(&mut b, "tonelab_job").await;
        assert!(other.iter().all(|m| m["type"] != "tonelab_progress"), "progress leaked to another client: {other:?}");

        send(&mut a, &format!(r#"{{"type":"tonelab_record_di","job":"{}","part":"p2","seconds":1.0}}"#, crate::tonelab_mock::DEMO_JOB)).await;
        let job = collect_until(&mut a, "tonelab_job").await.pop().unwrap();
        assert_eq!(job["job"]["parts"][1]["has_user_di"], true);
        send(&mut a, &format!(r#"{{"type":"tonelab_fit","job":"{}","part":"p2"}}"#, crate::tonelab_mock::DEMO_JOB)).await;
        let seen = collect_until(&mut a, "tonelab_fit_result").await;
        assert!(seen.iter().any(|m| m["type"] == "tonelab_progress" && m["stage"] == "fit"));
        let result = seen.last().unwrap();
        assert_eq!((result["part"].as_str(), result["fit"]["holdout_error_db"].is_number(), result["templates"].as_array().unwrap().len()), (Some("p2"), true, 6));

        send(&mut a, &format!(r#"{{"type":"tonelab_save_preset","job":"{}","part":"p1","name":"demo-fit"}}"#, crate::tonelab_mock::DEMO_JOB)).await;
        let st = collect_until(&mut a, "state").await.pop().unwrap();
        assert!(st["presets"].as_array().unwrap().iter().any(|p| p == "demo-fit"));
        assert!(library.list_presets().contains(&"demo-fit".to_string()));

        // Start, cancel while it runs.
        send(&mut a, r#"{"type":"tonelab_start","source":"https://example.com/x"}"#).await;
        send(&mut a, r#"{"type":"tonelab_cancel","job":"new"}"#).await;
        collect_until(&mut a, "cancelled").await;

        // The player gets a WAV, with ranges; escapes are refused.
        let (head, body) = http(port, &format!("GET /tonelab/file/{}/stems/guitar.wav HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", crate::tonelab_mock::DEMO_JOB)).await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(&body[..4], b"RIFF");
        let (head, body) = http(port, &format!("GET /tonelab/file/{}/mix.wav HTTP/1.1\r\nHost: x\r\nRange: bytes=0-43\r\nConnection: close\r\n\r\n", crate::tonelab_mock::DEMO_JOB)).await;
        assert!(head.starts_with("HTTP/1.1 206") && body.len() == 44, "{head}");
        let (head, _) = http(port, "GET /tonelab/file/other-job/mix.wav HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tonelab_runs_the_tool_serves_files_and_cancels() {
        let root = temp_dir("tl-real");
        let jobs = root.join("jobs");
        std::fs::create_dir_all(&jobs).unwrap();
        let tool = fake_tool(&root, &jobs);
        let library = Library::new(root.join("lib"));
        std::fs::create_dir_all(library.pedals_dir()).unwrap();
        let engine = Arc::new(MockEngine::new(library.pedals_dir(), default_graph()));
        let state = Arc::new(AppState::with_tonelab_tool(engine, library, Some(tool.to_string_lossy().into_owned())));
        let port = serve(state).await;
        let mut ws = connect(port).await;
        let mut other = connect(port).await;

        send(&mut ws, r#"{"type":"tonelab_caps"}"#).await;
        let caps = collect_until(&mut ws, "tonelab_caps").await.pop().unwrap();
        assert_eq!((caps["available"].as_bool(), caps["youtube"].as_bool(), caps["device"].as_str()), (Some(true), Some(false), Some("cpu")));

        // Upload, then start from the uploaded path.
        let body = b"RIFFfake";
        let req = format!("POST /tonelab/upload HTTP/1.1\r\nHost: x\r\nX-Filename: ../../my song.wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let mut raw = req.into_bytes();
        raw.extend_from_slice(body);
        let (head, resp) = {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(&raw).await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            let split = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            (String::from_utf8_lossy(&buf[..split]).to_string(), buf[split + 4..].to_vec())
        };
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let path = serde_json::from_slice::<serde_json::Value>(&resp).unwrap()["path"].as_str().unwrap().to_string();
        assert!(path.starts_with(jobs.join("uploads").to_str().unwrap()) && path.ends_with("-my song.wav"), "{path}");

        // A path that was not uploaded is refused.
        send(&mut ws, r#"{"type":"tonelab_start","source":"/etc/passwd"}"#).await;
        let err = collect_until(&mut ws, "error").await.pop().unwrap();
        assert!(err["message"].as_str().unwrap().contains("importing the song failed"), "{err}");

        send(&mut ws, &format!(r#"{{"type":"tonelab_start","source":{}}}"#, serde_json::to_string(&path).unwrap())).await;
        let seen = collect_until(&mut ws, "tonelab_job").await;
        let stages: Vec<&str> = seen.iter().filter(|m| m["type"] == "tonelab_progress").map(|m| m["stage"].as_str().unwrap()).collect();
        assert_eq!(stages, ["fetch", "separate"], "{seen:?}");
        assert_eq!(seen.iter().find(|m| m["stage"] == "fetch").unwrap()["job"], "new");
        assert_eq!(seen.iter().find(|m| m["stage"] == "separate").unwrap()["job"], "fake-song");
        assert_eq!(seen.iter().find(|m| m["stage"] == "separate").unwrap()["fraction"], 0.5);
        let job = seen.last().unwrap();
        assert_eq!((job["job"]["id"].as_str(), job["job"]["tempo_bpm"].as_f64(), job["job"]["sections"].as_array().unwrap().len()), (Some("fake-song"), Some(100.0), 1));
        // Everyone gets the job, nobody else gets the progress.
        let seen_other = collect_until(&mut other, "tonelab_job").await;
        assert!(seen_other.iter().all(|m| m["type"] != "tonelab_progress"), "{seen_other:?}");

        send(&mut ws, r#"{"type":"tonelab_jobs"}"#).await;
        assert_eq!(collect_until(&mut ws, "tonelab_jobs").await.pop().unwrap()["jobs"].as_array().unwrap().len(), 1);

        // Files: served inside the job only.
        std::fs::create_dir_all(jobs.join("fake-song/stems")).unwrap();
        std::fs::write(jobs.join("fake-song/stems/guitar.wav"), (0u8..100).collect::<Vec<_>>()).unwrap();
        std::fs::write(jobs.join("secret.wav"), b"secret").unwrap();
        let get = |p: &str, extra: &str| format!("GET /tonelab/file/{p} HTTP/1.1\r\nHost: x\r\n{extra}Connection: close\r\n\r\n");
        let (head, body) = http(port, &get("fake-song/stems/guitar.wav", "")).await;
        assert!(head.starts_with("HTTP/1.1 200") && body.len() == 100, "{head}");
        let (head, body) = http(port, &get("fake-song/stems/guitar.wav", "Range: bytes=10-19\r\n")).await;
        assert!(head.starts_with("HTTP/1.1 206") && head.to_lowercase().contains("content-range: bytes 10-19/100") && body == (10u8..20).collect::<Vec<_>>(), "{head}");
        for bad in ["fake-song/../secret.wav", "fake-song/%2e%2e/secret.wav", "..%2fsecret.wav/x.wav", "fake-song/job.json", "fake-song/stems/missing.wav", "fake-song//etc/passwd.wav"] {
            let (head, _) = http(port, &get(bad, "")).await;
            assert!(!head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.1 206"), "{bad}: {head}");
        }

        // A slow step can be cancelled; the job is free again afterwards.
        std::fs::write(jobs.join("slow"), b"").unwrap();
        send(&mut ws, &format!(r#"{{"type":"tonelab_start","source":{}}}"#, serde_json::to_string(&path).unwrap())).await;
        collect_until(&mut ws, "tonelab_progress").await; // fetch
        collect_until(&mut ws, "tonelab_progress").await; // separate: now sleeping
        send(&mut ws, r#"{"type":"tonelab_cancel","job":"fake-song"}"#).await;
        let t0 = std::time::Instant::now();
        let seen = collect_until(&mut ws, "cancelled").await;
        assert!(t0.elapsed() < Duration::from_secs(5), "cancel took too long");
        assert_eq!(seen.last().unwrap()["job"], "fake-song");
        send(&mut ws, r#"{"type":"tonelab_cancel","job":"fake-song"}"#).await;
        assert!(collect_until(&mut ws, "error").await.pop().unwrap()["message"].as_str().unwrap().contains("nothing is running"));

        // The live input cannot be recorded yet: a clear error.
        send(&mut ws, r#"{"type":"tonelab_record_di","job":"fake-song","part":"p1","seconds":5.0}"#).await;
        assert!(collect_until(&mut ws, "error").await.pop().unwrap()["message"].as_str().unwrap().contains("not available yet"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tonelab_without_the_tool_says_why() {
        let (state, _, root) = app("tl-off");
        for tool in [Some("none".to_string()), Some("/does/not/exist".to_string())] {
            let st = AppState::with_tonelab_tool(state.engine.clone(), Library::new(&root), tool);
            let reason = st.tonelab.caps().unwrap_err();
            assert!(!reason.is_empty());
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
