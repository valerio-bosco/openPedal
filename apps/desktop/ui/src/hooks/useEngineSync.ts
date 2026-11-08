// Keeps the engine and the editor in step: loads the graph the server sends, sends `set_graph`
// when the structure changes, and sends the lightest possible update when a snapshot is restored.
import { useCallback, useEffect, useRef, type MutableRefObject } from "react";
import type { Edge } from "@xyflow/react";
import type { Snapshot } from "../history";
import type { StateMsg } from "../generated/protocol";
import { flowToGraph, paramValue, specsToFlow, structureKey, type PNode } from "../model";
import type { GraphEditor } from "./useGraphEditor";
import type { HistoryControl } from "./useHistory";
import type { useServer } from "./useServer";

interface Deps {
  server: ReturnType<typeof useServer>;
  graph: GraphEditor;
  history: HistoryControl;
  graphName: string;
  setGraphName: (name: string) => void;
  /** Non-null while editing inside a pedal: the main graph is then held elsewhere and nothing is sent. */
  editing: unknown;
  editingRef: MutableRefObject<unknown>;
}

export function useEngineSync({ server, graph, history, graphName, setGraphName, editing, editingRef }: Deps) {
  const { catalog, catalogItems, send, sendParam, handlers } = server;
  const { nodes, edges, setNodes, setEdges, tidyIfOverlapping } = graph;
  /** Structure last sent to (or received from) the engine. */
  const lastKey = useRef("");

  /** Records that the engine already has this graph. */
  const markSynced = useCallback((ns: PNode[], es: Edge[]) => {
    lastKey.current = structureKey(ns, es, catalog);
  }, [catalog]);

  // --- the server sent a graph (start-up, preset loaded, saved, deleted) ---
  handlers.current.graph = (m: StateMsg) => {
    // inside a pedal the main graph stays the one held in `parent`
    if (editingRef.current) return;
    setGraphName(m.graph.name);
    const f = specsToFlow(m.graph.node, m.graph.edge);
    const cat = new Map(m.catalog.map((c) => [c.kind, c]));
    lastKey.current = structureKey(f.nodes, f.edges, cat);
    setNodes(f.nodes);
    setEdges(f.edges);
    // a loaded preset is a new starting point; the same graph (e.g. after a save) is not
    if (!history.same(f)) {
      history.reset({ ...f, name: m.graph.name });
      // automatic tidying is not an edit to undo
      tidyIfOverlapping((ns, es) => history.reset({ nodes: ns, edges: es }));
    }
  };
  handlers.current.nodeError = (node, message) =>
    setNodes((ns) => ns.map((n) => (n.id === node ? { ...n, data: { ...n.data, error: message } } : n)));

  // --- structure changed: new graph to the engine ---
  useEffect(() => {
    if (catalogItems.length === 0 || editing) return;
    const key = structureKey(nodes, edges, catalog);
    if (key === lastKey.current) return;
    const h = window.setTimeout(() => {
      lastKey.current = key;
      setNodes((ns) => (ns.some((n) => n.data.error) ? ns.map((n) => ({ ...n, data: { ...n.data, error: undefined } })) : ns));
      send({ type: "set_graph", graph: flowToGraph(graphName, nodes, edges) });
    }, 150);
    return () => window.clearTimeout(h);
  }, [nodes, edges, catalog, catalogItems.length, graphName, setNodes, send, editing]);

  /** Restores the graph to a snapshot and tells the engine in the lightest way possible. */
  const restore = useCallback(
    (s: Snapshot) => {
      const prev = new Map(nodes.map((n) => [n.id, n]));
      setNodes(s.nodes);
      setEdges(s.edges);
      if (editingRef.current) return; // inside a pedal everything is sent on save
      const name = s.name ?? graphName;
      setGraphName(name);
      const key = structureKey(s.nodes, s.edges, catalog);
      if (key !== lastKey.current) {
        lastKey.current = key;
        send({ type: "set_graph", graph: flowToGraph(name, s.nodes, s.edges) });
        return;
      }
      // same structure: only the changed parameters are needed, without rebuilding the sound
      for (const n of s.nodes) {
        const before = prev.get(n.id);
        const item = catalog.get(n.data.kind);
        if (!before || !item) continue;
        if (!!n.data.bypass !== !!before.data.bypass) send({ type: "set_bypass", node: n.id, bypass: !!n.data.bypass });
        for (const p of item.params) {
          if (p.kind === "file") continue;
          const now = n.data.params[p.id];
          if (now === before.data.params[p.id]) continue;
          sendParam(n.id, p.id, paramValue(p, now));
        }
      }
    },
    [nodes, catalog, graphName, setNodes, setEdges, setGraphName, send, sendParam, editingRef],
  );

  return { markSynced, restore };
}
