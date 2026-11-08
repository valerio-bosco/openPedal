import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Connection as FlowConnection,
  type Edge,
} from "@xyflow/react";
import {
  PEDAL_IN,
  PEDAL_OUT,
  PEDAL_PREFIX,
  type AudioInfo,
  type CatalogItem,
  type Category,
  type EdgeSpec,
  type KnobSpec,
  type MetersMsg,
  type NodeSpec,
  type ParamValue,
  type PedalFile,
  type ServerMsg,
} from "./protocol";
import { fixturePedals } from "./mock";
import {
  CATEGORY_COLOR,
  IMPLICIT_ITEMS,
  edgeId,
  X_SCALE,
  flowToGraph,
  flowToSpecs,
  isFixed,
  slug,
  paramValue,
  specToNode,
  specsToFlow,
  structureKey,
  uniqueId,
  wouldCycle,
  type PNode,
} from "./model";
import { Connection, type Status } from "./ws";
import { EditorContext, PedalNodeMemo } from "./components/PedalNode";
import { Palette, DRAG_MIME } from "./components/Palette";
import { TopBar } from "./components/TopBar";
import { PedalDialog, type Candidate, type PedalForm } from "./components/PedalDialog";

const nodeTypes = { pedal: PedalNodeMemo };

/** Stato di "dentro il pedale": il grafo principale resta in `parent`. */
interface Editing {
  id: string;
  name: string;
  category: Category;
  description: string;
  knobs: KnobSpec[];
  parent: { nodes: PNode[]; edges: Edge[] };
}

type Dialog = { mode: "create"; ids: string[] } | { mode: "edit" } | null;

interface Toast {
  id: number;
  text: string;
}

function Editor() {
  const [catalogItems, setCatalogItems] = useState<CatalogItem[]>([]);
  const [presets, setPresets] = useState<string[]>([]);
  const [audio, setAudio] = useState<AudioInfo>();
  const [meters, setMeters] = useState<MetersMsg>();
  const [status, setStatus] = useState<Status>("connecting");
  const [graphName, setGraphName] = useState("");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [nodes, setNodes, onNodesChange] = useNodesState<PNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  const flow = useReactFlow<PNode, Edge>();
  const wrapper = useRef<HTMLDivElement>(null);
  const conn = useRef<Connection>(undefined);
  const lastKey = useRef("");
  const edgesRef = useRef(edges);
  edgesRef.current = edges;
  const editingRef = useRef(editing);
  editingRef.current = editing;
  const pedals = useRef<Record<string, PedalFile>>({});
  /** Dopo `save_pedal`: azione da eseguire quando arriva lo `state` con il pedale nel catalogo. */
  const pending = useRef<{ kind: string; run: () => void }>(undefined);

  const catalog = useMemo(
    () => new Map([...catalogItems, ...IMPLICIT_ITEMS].map((c) => [c.kind, c])),
    [catalogItems],
  );

  const toast = useCallback((text: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t.slice(-3), { id, text }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
  }, []);

  // --- messaggi dal server ---
  const onMsg = useCallback(
    (m: ServerMsg) => {
      if (m.type === "meters") return setMeters(m);
      if (m.type === "error") {
        toast(m.message);
        if (m.node) setNodes((ns) => ns.map((n) => (n.id === m.node ? { ...n, data: { ...n.data, error: m.message } } : n)));
        return;
      }
      setCatalogItems(m.catalog);
      setPresets(m.presets);
      setAudio(m.audio);
      const pend = pending.current;
      if (pend && m.catalog.some((c) => c.kind === pend.kind)) {
        pending.current = undefined;
        pend.run();
        return;
      }
      // dentro un pedale il grafo principale resta quello che ho in `parent`
      if (editingRef.current) return;
      setGraphName(m.graph.name);
      const f = specsToFlow(m.graph.node, m.graph.edge);
      const cat = new Map(m.catalog.map((c) => [c.kind, c]));
      lastKey.current = structureKey(f.nodes, f.edges, cat);
      setNodes(f.nodes);
      setEdges(f.edges);
    },
    [setNodes, setEdges, toast],
  );

  useEffect(() => {
    const c = new Connection(onMsg, setStatus);
    conn.current = c;
    c.start();
    return () => c.stop();
  }, [onMsg]);

  // in modalità demo i contenuti dei pedali d'esempio vengono dalle fixture
  useEffect(() => {
    if (status === "mock") pedals.current = { ...fixturePedals(), ...pedals.current };
  }, [status]);

  // --- struttura cambiata: nuovo grafo al motore ---
  useEffect(() => {
    if (catalogItems.length === 0 || editing) return;
    const key = structureKey(nodes, edges, catalog);
    if (key === lastKey.current) return;
    const h = window.setTimeout(() => {
      lastKey.current = key;
      setNodes((ns) => (ns.some((n) => n.data.error) ? ns.map((n) => ({ ...n, data: { ...n.data, error: undefined } })) : ns));
      conn.current?.send({ type: "set_graph", graph: flowToGraph(graphName, nodes, edges) });
    }, 150);
    return () => window.clearTimeout(h);
  }, [nodes, edges, catalog, catalogItems.length, graphName, setNodes, editing]);

  // --- parametri ---
  const setParam = useCallback(
    (nodeId: string, paramId: string, value: ParamValue) => {
      setNodes((ns) =>
        ns.map((n) => (n.id === nodeId ? { ...n, data: { ...n.data, params: { ...n.data.params, [paramId]: value } } } : n)),
      );
      const node = flow.getNode(nodeId);
      const desc = node && catalog.get(node.data.kind)?.params.find((p) => p.id === paramId);
      if (desc && desc.kind !== "file" && !editingRef.current) conn.current?.sendParam(nodeId, paramId, paramValue(desc, value));
    },
    [flow, catalog, setNodes],
  );
  const ctx = useMemo(() => ({ catalog, setParam }), [catalog, setParam]);

  // --- cavi ---
  const isValidConnection = useCallback(
    (c: FlowConnection | Edge) =>
      !wouldCycle(edgesRef.current, c.source, c.target) &&
      !edgesRef.current.some((e) => e.source === c.source && e.target === c.target),
    [],
  );
  const onConnect = useCallback(
    (c: FlowConnection) => setEdges((es) => [...es, { id: edgeId(c.source, c.target), source: c.source, target: c.target }]),
    [setEdges],
  );

  // --- aggiunta, duplicazione ---
  const addNode = useCallback(
    (kind: string, pos?: { x: number; y: number }) => {
      const taken = new Set(nodes.map((n) => n.id));
      const base = kind.replace(/^pedal:/, "p-");
      const rect = wrapper.current?.getBoundingClientRect();
      const p = pos ?? flow.screenToFlowPosition({ x: (rect?.left ?? 0) + (rect?.width ?? 600) / 2, y: (rect?.top ?? 0) + (rect?.height ?? 400) / 2 });
      const node = specToNode({ id: uniqueId(base, taken), type: kind, pos: [p.x / X_SCALE, p.y], params: {} });
      setNodes((ns) => [...ns.map((n) => ({ ...n, selected: false })), { ...node, selected: true }]);
    },
    [nodes, flow, setNodes],
  );

  const duplicate = useCallback(() => {
    const sel = nodes.filter((n) => n.selected && !isFixed(n.data.kind));
    if (sel.length === 0) return;
    const taken = new Set(nodes.map((n) => n.id));
    const map = new Map<string, string>();
    for (const n of sel) {
      const id = uniqueId(n.data.kind.replace(/^pedal:/, "p-"), taken);
      taken.add(id);
      map.set(n.id, id);
    }
    const copies: PNode[] = sel.map((n) => ({
      ...n,
      id: map.get(n.id)!,
      position: { x: n.position.x + 40, y: n.position.y + 40 },
      selected: true,
      data: { ...n.data, params: { ...n.data.params }, error: undefined },
    }));
    const newEdges = edges
      .filter((e) => map.has(e.source) && map.has(e.target))
      .map((e) => ({ id: edgeId(map.get(e.source)!, map.get(e.target)!), source: map.get(e.source)!, target: map.get(e.target)! }));
    setNodes((ns) => [...ns.map((n) => ({ ...n, selected: false })), ...copies]);
    setEdges((es) => [...es, ...newEdges]);
  }, [nodes, edges, setNodes, setEdges]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA") return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") {
        e.preventDefault();
        duplicate();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [duplicate]);

  const onDrop = (e: React.DragEvent) => {
    const kind = e.dataTransfer.getData(DRAG_MIME);
    if (!kind) return;
    e.preventDefault();
    addNode(kind, flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }));
  };

  const savePreset = () => {
    const name = window.prompt("Nome del preset", graphName || "mio-suono")?.trim();
    if (!name) return;
    setGraphName(name);
    conn.current?.send({ type: "save_preset", name, graph: flowToGraph(name, nodes, edges) });
    toast(`Preset «${name}» salvato`);
  };

  const deletePedal = (item: CatalogItem) => {
    if (!window.confirm(`Eliminare il pedale «${item.label}»?`)) return;
    conn.current?.send({ type: "delete_pedal", id: item.kind.replace(/^pedal:/, "") });
  };

  // --- pedali composti ---
  const candidatesFor = (list: PNode[]): Candidate[] =>
    list.flatMap((n) => {
      const item = catalog.get(n.data.kind);
      return (item?.params ?? []).flatMap((p) =>
        p.kind === "number"
          ? [{ nodeId: n.id, nodeLabel: item!.label, desc: p, current: paramValue(p, n.data.params[p.id]) }]
          : [],
      );
    });

  const createPedal = (ids: string[], f: PedalForm) => {
    const sel = nodes.filter((n) => ids.includes(n.id));
    const inSel = new Set(ids);
    let id = slug(f.name) || "pedale";
    for (let i = 2; catalog.has(PEDAL_PREFIX + id); i++) id = `${slug(f.name) || "pedale"}-${i}`;
    const kind = PEDAL_PREFIX + id;

    const minX = Math.min(...sel.map((n) => n.position.x));
    const minY = Math.min(...sel.map((n) => n.position.y));
    const inner: NodeSpec[] = sel.map((n) => ({
      id: n.id,
      type: n.data.kind,
      pos: [Math.round((n.position.x - minX) / X_SCALE), Math.round(n.position.y - minY)],
      params: { ...n.data.params },
    }));
    const internal = edges.filter((e) => inSel.has(e.source) && inSel.has(e.target));
    const hasInternalIn = new Set(internal.map((e) => e.target));
    const hasInternalOut = new Set(internal.map((e) => e.source));
    const extIn = edges.filter((e) => !inSel.has(e.source) && inSel.has(e.target));
    const extOut = edges.filter((e) => inSel.has(e.source) && !inSel.has(e.target));
    const entries = new Set(extIn.map((e) => e.target));
    const exits = new Set(extOut.map((e) => e.source));
    if (entries.size === 0) ids.filter((i) => !hasInternalIn.has(i)).forEach((i) => entries.add(i));
    if (exits.size === 0) ids.filter((i) => !hasInternalOut.has(i)).forEach((i) => exits.add(i));
    const pedalEdges: EdgeSpec[] = [
      ...internal.map((e) => ({ from: e.source, to: e.target })),
      ...[...entries].map((to) => ({ from: PEDAL_IN, to })),
      ...[...exits].map((from) => ({ from, to: PEDAL_OUT })),
    ];
    const pedal: PedalFile = { name: f.name, category: f.category, description: f.description, node: inner, edge: pedalEdges, knob: f.knobs };
    pedals.current[id] = pedal;

    const nodeId = uniqueId("p-" + id, new Set(nodes.map((n) => n.id)));
    const params: Record<string, ParamValue> = Object.fromEntries(f.knobs.map((k) => [k.label, k.default]));
    const pos: [number, number] = [Math.round(minX / X_SCALE), Math.round(minY)];
    const newNode = specToNode({ id: nodeId, type: kind, pos, params });
    const keep = nodes.filter((n) => !inSel.has(n.id));
    const keepEdges = edges.filter((e) => !inSel.has(e.source) && !inSel.has(e.target));
    const newEdges: Edge[] = [
      ...extIn.map((e) => ({ id: edgeId(e.source, nodeId), source: e.source, target: nodeId })),
      ...extOut.map((e) => ({ id: edgeId(nodeId, e.target), source: nodeId, target: e.target })),
    ].filter((e, i, a) => a.findIndex((x) => x.id === e.id) === i);

    pending.current = {
      kind,
      run: () => {
        const ns = [...keep.map((n) => ({ ...n, selected: false })), { ...newNode, selected: true }];
        const es = [...keepEdges, ...newEdges];
        setNodes(ns);
        setEdges(es);
        lastKey.current = structureKey(ns, es, catalog);
        conn.current?.send({ type: "set_graph", graph: flowToGraph(graphName, ns, es) });
      },
    };
    conn.current?.send({ type: "save_pedal", id, pedal });
    setDialog(null);
    toast(`Pedale «${f.name}» creato`);
  };

  const enterPedal = (node: PNode) => {
    if (!node.data.kind.startsWith(PEDAL_PREFIX) || editing) return;
    const id = node.data.kind.slice(PEDAL_PREFIX.length);
    const p = pedals.current[id];
    if (!p) return toast("Il contenuto di questo pedale non è disponibile: si può modificare solo dopo averlo creato qui.");
    const f = specsToFlow(p.node, p.edge);
    const xs = p.node.map((n) => n.pos[0]);
    const ys = p.node.map((n) => n.pos[1]);
    const midY = ys.length ? (Math.min(...ys) + Math.max(...ys)) / 2 : 0;
    const nIn = specToNode({ id: PEDAL_IN, type: PEDAL_IN, pos: [(xs.length ? Math.min(...xs) : 0) - 260, midY], params: {} });
    const nOut = specToNode({ id: PEDAL_OUT, type: PEDAL_OUT, pos: [(xs.length ? Math.max(...xs) : 0) + 300, midY], params: {} });
    setEditing({
      id,
      name: p.name,
      category: p.category as Category,
      description: p.description,
      knobs: p.knob,
      parent: { nodes, edges },
    });
    setNodes([nIn, ...f.nodes, nOut]);
    setEdges(f.edges);
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.3 }), 50);
  };

  const exitPedal = () => {
    if (!editing) return;
    setNodes(editing.parent.nodes);
    setEdges(editing.parent.edges);
    setEditing(null);
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.2 }), 50);
  };

  const savePedalEdit = (f?: PedalForm) => {
    if (!editing) return;
    const meta = f ?? editing;
    const { node, edge } = flowToSpecs(nodes, edges);
    const alive = new Set(node.map((n) => n.id));
    const knobs = meta.knobs.filter((k) => alive.has(k.target.split(".")[0]));
    const pedal: PedalFile = { name: meta.name, category: meta.category, description: meta.description, node, edge, knob: knobs };
    pedals.current[editing.id] = pedal;
    conn.current?.send({ type: "save_pedal", id: editing.id, pedal });
    // il pedale cambiato va ricompilato nel grafo principale
    const parent = editing.parent;
    conn.current?.send({ type: "set_graph", graph: flowToGraph(graphName, parent.nodes, parent.edges) });
    setDialog(null);
    exitPedal();
    toast(`Pedale «${meta.name}» salvato`);
  };

  const selected = nodes.filter((n) => n.selected && !isFixed(n.data.kind));
  const ready = catalogItems.length > 0;
  const selCount = selected.length;

  return (
    <EditorContext.Provider value={ctx}>
      <div className="app">
        <TopBar
          status={status}
          presets={presets}
          graphName={graphName}
          audio={audio}
          meters={meters}
          disabledPresets={!ready || !!editing}
          onLoadPreset={(name) => conn.current?.send({ type: "load_preset", name })}
          onSave={savePreset}
          extra={
            editing ? (
              <>
                <span className="crumb">Dentro il pedale «{editing.name}»</span>
                <button onClick={() => setDialog({ mode: "edit" })}>Dettagli e manopole…</button>
                <button className="primary" onClick={() => savePedalEdit()}>
                  Salva pedale
                </button>
                <button onClick={exitPedal}>Esci senza salvare</button>
              </>
            ) : (
              <>
                <button onClick={duplicate} disabled={selCount === 0} title="Ctrl+D">
                  Duplica
                </button>
                <button
                  className={selCount > 0 ? "primary" : undefined}
                  disabled={selCount === 0}
                  title="Racchiudi i nodi selezionati in un nuovo pedale"
                  onClick={() => setDialog({ mode: "create", ids: selected.map((n) => n.id) })}
                >
                  Crea pedale
                </button>
              </>
            )
          }
        />
        <div className="main">
          <Palette catalog={catalogItems} onAdd={addNode} onDeletePedal={deletePedal} />
          <div className="canvas" ref={wrapper} onDrop={onDrop} onDragOver={(e) => e.preventDefault()}>
            {!ready && <div className="loading">Connessione a openPedal…</div>}
            <ReactFlow
              nodes={nodes}
              edges={edges}
              nodeTypes={nodeTypes}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              onConnectEnd={(_, st) => {
                if (st.toNode && !st.isValid) toast("Quel cavo non è valido: creerebbe un ciclo o esiste già.");
              }}
              isValidConnection={isValidConnection}
              onNodeDoubleClick={(_, n) => enterPedal(n)}
              colorMode="dark"
              fitView
              minZoom={0.2}
              deleteKeyCode={["Delete", "Backspace"]}
              multiSelectionKeyCode="Shift"
              selectionKeyCode={null}
              selectionOnDrag
              panOnDrag={[1, 2]}
              defaultEdgeOptions={{ type: "default", animated: false }}
              proOptions={{ hideAttribution: true }}
            >
              <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} />
              <Controls showInteractive={false} />
              <MiniMap
                pannable
                zoomable
                nodeColor={(n) => CATEGORY_COLOR[catalog.get((n as PNode).data.kind)?.category ?? "utility"]}
                maskColor="rgba(10,10,14,0.7)"
              />
            </ReactFlow>
          </div>
        </div>
        {dialog?.mode === "create" && (
          <PedalDialog
            title="Crea pedale"
            submitLabel="Crea"
            initial={{ name: "", category: "utility", description: "" }}
            candidates={candidatesFor(nodes.filter((n) => dialog.ids.includes(n.id)))}
            knobs={[]}
            onSubmit={(f) => createPedal(dialog.ids, f)}
            onCancel={() => setDialog(null)}
          />
        )}
        {dialog?.mode === "edit" && editing && (
          <PedalDialog
            title={`Pedale «${editing.name}»`}
            submitLabel="Salva pedale"
            initial={editing}
            candidates={candidatesFor(nodes)}
            knobs={editing.knobs}
            onSubmit={(f) => savePedalEdit(f)}
            onCancel={() => setDialog(null)}
          />
        )}
        <div className="toasts">
          {toasts.map((t) => (
            <div key={t.id} className="toast">
              {t.text}
            </div>
          ))}
        </div>
      </div>
    </EditorContext.Provider>
  );
}

export default function App() {
  return (
    <ReactFlowProvider>
      <Editor />
    </ReactFlowProvider>
  );
}
