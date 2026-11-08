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
import type { AudioInfo, CatalogItem, MetersMsg, ParamValue, ServerMsg } from "./protocol";
import {
  CATEGORY_COLOR,
  IMPLICIT_ITEMS,
  edgeId,
  flowToGraph,
  isFixed,
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

const nodeTypes = { pedal: PedalNodeMemo };

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
  const [nodes, setNodes, onNodesChange] = useNodesState<PNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  const flow = useReactFlow<PNode, Edge>();
  const wrapper = useRef<HTMLDivElement>(null);
  const conn = useRef<Connection>(undefined);
  const lastKey = useRef("");
  const edgesRef = useRef(edges);
  edgesRef.current = edges;

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

  // --- struttura cambiata: nuovo grafo al motore ---
  useEffect(() => {
    if (catalogItems.length === 0) return;
    const key = structureKey(nodes, edges, catalog);
    if (key === lastKey.current) return;
    const h = window.setTimeout(() => {
      lastKey.current = key;
      setNodes((ns) => (ns.some((n) => n.data.error) ? ns.map((n) => ({ ...n, data: { ...n.data, error: undefined } })) : ns));
      conn.current?.send({ type: "set_graph", graph: flowToGraph(graphName, nodes, edges) });
    }, 150);
    return () => window.clearTimeout(h);
  }, [nodes, edges, catalog, catalogItems.length, graphName, setNodes]);

  // --- parametri ---
  const setParam = useCallback(
    (nodeId: string, paramId: string, value: ParamValue) => {
      setNodes((ns) =>
        ns.map((n) => (n.id === nodeId ? { ...n, data: { ...n.data, params: { ...n.data.params, [paramId]: value } } } : n)),
      );
      const node = flow.getNode(nodeId);
      const desc = node && catalog.get(node.data.kind)?.params.find((p) => p.id === paramId);
      if (desc && desc.kind !== "file") conn.current?.sendParam(nodeId, paramId, paramValue(desc, value));
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
      const node = specToNode({ id: uniqueId(base, taken), type: kind, pos: [p.x, p.y], params: {} });
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

  const ready = catalogItems.length > 0;
  const selCount = nodes.filter((n) => n.selected && !isFixed(n.data.kind)).length;

  return (
    <EditorContext.Provider value={ctx}>
      <div className="app">
        <TopBar
          status={status}
          presets={presets}
          graphName={graphName}
          audio={audio}
          meters={meters}
          disabledPresets={!ready}
          onLoadPreset={(name) => conn.current?.send({ type: "load_preset", name })}
          onSave={savePreset}
          extra={
            <button onClick={duplicate} disabled={selCount === 0} title="Ctrl+D">
              Duplica
            </button>
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
