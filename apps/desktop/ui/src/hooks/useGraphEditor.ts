// The graph being edited: React Flow nodes and cables, selection, and every edit operation on them.
import { useCallback, useMemo, useRef, type MutableRefObject } from "react";
import { useEdgesState, useNodesState, useReactFlow, type Connection as FlowConnection, type Edge } from "@xyflow/react";
import { INPUT, OUTPUT, type CatalogItem, type ParamValue } from "../generated/protocol";
import { X_SCALE, edgeId, isFixed, paramValue, specToNode, uniqueId, wouldCycle, type PNode } from "../model";
import { hasOverlap, tidyPositions } from "../lib/layout";

interface Deps {
  catalog: Map<string, CatalogItem>;
  /** True while editing inside a pedal: edits are not sent to the engine. */
  editingRef: MutableRefObject<unknown>;
  send: (msg: { type: "set_bypass"; node: string; bypass: boolean }) => void;
  sendParam: (node: string, param: string, value: number) => void;
  toast: (text: string) => void;
  setGraphName: (name: string) => void;
}

export function useGraphEditor({ catalog, editingRef, send, sendParam, toast, setGraphName }: Deps) {
  const [nodes, setNodes, onNodesChange] = useNodesState<PNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const flow = useReactFlow<PNode, Edge>();
  const wrapper = useRef<HTMLDivElement>(null);
  const edgesRef = useRef(edges);
  edgesRef.current = edges;

  // --- tidy up: columns in signal order, using the pedals' real measurements ---
  const tidy = useCallback(() => {
    const pos = tidyPositions(flow.getNodes(), edgesRef.current);
    setNodes((cur) => cur.map((n) => (pos.has(n.id) ? { ...n, position: pos.get(n.id)! } : n)));
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.15 }), 30);
  }, [flow, setNodes]);

  /** After loading a graph: if pedals overlap, tidy up, then call `onTidied` with the result. */
  const tidyIfOverlapping = useCallback(
    (onTidied: (nodes: PNode[], edges: Edge[]) => void) => {
      window.setTimeout(() => {
        if (!hasOverlap(flow.getNodes())) return;
        tidy();
        window.setTimeout(() => onTidied(flow.getNodes(), edgesRef.current), 60);
      }, 120);
    },
    [flow, tidy],
  );

  /** Empty graph: only Guitar connected to Amp (you hear the clean sound). Undo with Ctrl+Z. */
  const newGraph = useCallback(() => {
    const nIn = specToNode({ id: "in", type: INPUT, pos: [0, 0], params: {} });
    const nOut = specToNode({ id: "out", type: OUTPUT, pos: [300, 0], params: {} });
    setNodes([nIn, nOut]);
    setEdges([{ id: edgeId("in", "out"), source: "in", target: "out" }]);
    setGraphName("new");
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.6, maxZoom: 1 }), 50);
  }, [flow, setNodes, setEdges, setGraphName]);

  // --- deletion ---
  const removeNodes = useCallback(
    (ids: string[]) => {
      const gone = new Set(ids.filter((id) => !isFixed(flow.getNode(id)?.data.kind ?? "")));
      if (gone.size === 0) return;
      setNodes((ns) => ns.filter((n) => !gone.has(n.id)));
      setEdges((es) => es.filter((e) => !gone.has(e.source) && !gone.has(e.target)));
    },
    [flow, setNodes, setEdges],
  );
  const removeEdges = useCallback((ids: string[]) => setEdges((es) => es.filter((e) => !ids.includes(e.id))), [setEdges]);
  const disconnect = useCallback(
    (id: string) => setEdges((es) => es.filter((e) => e.source !== id && e.target !== id)),
    [setEdges],
  );
  const deleteSelection = useCallback(() => {
    removeEdges(edges.filter((e) => e.selected).map((e) => e.id));
    removeNodes(nodes.filter((n) => n.selected).map((n) => n.id));
  }, [nodes, edges, removeNodes, removeEdges]);

  // --- cables: dragging an end moves it, dropping it in empty space removes it ---
  const reconnected = useRef(true);
  const onReconnectStart = useCallback(() => {
    reconnected.current = false;
  }, []);
  const onReconnect = useCallback(
    (old: Edge, c: FlowConnection) => {
      reconnected.current = true;
      const others = edgesRef.current.filter((e) => e.id !== old.id);
      if (wouldCycle(others, c.source, c.target) || others.some((e) => e.source === c.source && e.target === c.target)) {
        toast("That cable is not valid: it would create a cycle or already exists.");
        return;
      }
      setEdges([...others, { id: edgeId(c.source, c.target), source: c.source, target: c.target }]);
    },
    [setEdges, toast],
  );
  const onReconnectEnd = useCallback(
    (_: unknown, edge: Edge) => {
      if (!reconnected.current) removeEdges([edge.id]);
      reconnected.current = true;
    },
    [removeEdges],
  );
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

  // --- parameters ---
  const setParam = useCallback(
    (nodeId: string, paramId: string, value: ParamValue) => {
      setNodes((ns) =>
        ns.map((n) => (n.id === nodeId ? { ...n, data: { ...n.data, params: { ...n.data.params, [paramId]: value } } } : n)),
      );
      const node = flow.getNode(nodeId);
      const desc = node && catalog.get(node.data.kind)?.params.find((p) => p.id === paramId);
      if (desc && desc.kind !== "file" && !editingRef.current) sendParam(nodeId, paramId, paramValue(desc, value));
    },
    [flow, catalog, setNodes, sendParam, editingRef],
  );
  const setBypass = useCallback(
    (nodeId: string, bypass: boolean) => {
      setNodes((ns) =>
        ns.map((n) => {
          if (n.id !== nodeId) return n;
          const { bypass: _old, ...data } = n.data;
          return { ...n, data: bypass ? { ...data, bypass: true } : data };
        }),
      );
      if (!editingRef.current) send({ type: "set_bypass", node: nodeId, bypass });
    },
    [setNodes, send, editingRef],
  );
  const removeNode = useCallback((id: string) => removeNodes([id]), [removeNodes]);
  /** What the pedal components need from the editor. */
  const pedalContext = useMemo(() => ({ catalog, setParam, setBypass, removeNode }), [catalog, setParam, setBypass, removeNode]);

  // --- adding, duplicating ---
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

  /** Duplicates the given pedals (or, with no arguments, the selected ones) along with the cables between them. */
  const duplicate = useCallback(
    (only?: string[]) => {
      const sel = nodes.filter((n) => (only ? only.includes(n.id) : n.selected) && !isFixed(n.data.kind));
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
    },
    [nodes, edges, setNodes, setEdges],
  );

  const selected = nodes.filter((n) => n.selected && !isFixed(n.data.kind));
  const selectedEdgeCount = edges.filter((e) => e.selected).length;

  return {
    nodes,
    edges,
    setNodes,
    setEdges,
    onNodesChange,
    onEdgesChange,
    flow,
    wrapper,
    pedalContext,
    selected,
    selectedEdgeCount,
    tidy,
    tidyIfOverlapping,
    newGraph,
    removeNodes,
    removeEdges,
    disconnect,
    deleteSelection,
    addNode,
    duplicate,
    isValidConnection,
    onConnect,
    onReconnectStart,
    onReconnect,
    onReconnectEnd,
  };
}

export type GraphEditor = ReturnType<typeof useGraphEditor>;
