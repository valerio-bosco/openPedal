// Keeps the engine and the editor in step: loads the graph the server sends (start-up, preset,
// undo / redo, edits by another client) and sends `set_graph` when the structure changes.
//
// History is the core's (see PLAN.md §5.4): this hook never records steps. The debounce below
// yields exactly one `set_graph` per structural user action. Node positions are not part of
// `structureKey`, so moving a node sends nothing and is never an undo step; the positions travel
// along with the next structural edit or with `save_preset`.
import { useCallback, useEffect, useRef, type MutableRefObject } from "react";
import type { Edge } from "@xyflow/react";
import type { StateMsg } from "../generated/protocol";
import { flowToGraph, specsToFlow, structureKey, type PNode } from "../model";
import type { GraphEditor } from "./useGraphEditor";
import type { useServer } from "./useServer";

interface Deps {
  server: ReturnType<typeof useServer>;
  graph: GraphEditor;
  graphName: string;
  setGraphName: (name: string) => void;
  /** Non-null while editing inside a pedal: the main graph is then held elsewhere and nothing is sent. */
  editing: unknown;
  editingRef: MutableRefObject<unknown>;
}

export function useEngineSync({ server, graph, graphName, setGraphName, editing, editingRef }: Deps) {
  const { catalog, catalogItems, send, handlers } = server;
  const { nodes, edges, setNodes, setEdges, tidyIfOverlapping } = graph;
  /** Structure last sent to (or received from) the engine. */
  const lastKey = useRef("");

  /** Records that the engine already has this graph. */
  const markSynced = useCallback((ns: PNode[], es: Edge[]) => {
    lastKey.current = structureKey(ns, es, catalog);
  }, [catalog]);

  // --- the server sent a graph (start-up, preset loaded, undo / redo, another client's edit) ---
  handlers.current.graph = (m: StateMsg, keep) => {
    // inside a pedal the main graph stays the one held in `parent`
    if (editingRef.current) return;
    setGraphName(m.graph.name);
    const f = specsToFlow(m.graph.node, m.graph.edge);
    const cat = new Map(m.catalog.map((c) => [c.kind, c]));
    lastKey.current = structureKey(f.nodes, f.edges, cat);
    // Positions belong to the client: while the graph has a history or unsaved changes, a node that
    // is already on screen keeps its place (so undoing "add pedal" does not also undo the layout
    // moves). A freshly loaded preset (no history, clean) takes the server's positions.
    const keepPlaces = m.can_undo || m.can_redo || m.dirty;
    setNodes((cur) => {
      const old = new Map(cur.map((n) => [n.id, n]));
      return f.nodes.map((n) => {
        const prev = old.get(n.id);
        if (!prev) return n;
        // parameters the user is still turning are newer than this state
        const params = { ...n.data.params };
        for (const [k, v] of Object.entries(prev.data.params)) if (keep?.has(`${n.id}\u0000${k}`)) params[k] = v;
        return {
          ...n,
          data: { ...n.data, params },
          selected: prev.selected,
          measured: prev.measured,
          ...(keepPlaces ? { position: prev.position } : {}),
        };
      });
    });
    setEdges((cur) => {
      const sel = new Set(cur.filter((e) => e.selected).map((e) => e.id));
      return f.edges.map((e) => (sel.has(e.id) ? { ...e, selected: true } : e));
    });
    // automatic tidying is not an edit (and positions alone are never history steps)
    if (!keepPlaces) tidyIfOverlapping(() => {});
  };
  handlers.current.nodeError = (node, message) =>
    setNodes((ns) => ns.map((n) => (n.id === node ? { ...n, data: { ...n.data, error: message } } : n)));
  // another client turned a knob: show it live (a choice arrives as its option index)
  handlers.current.param = (node, param, value) => {
    if (editingRef.current) return;
    setNodes((ns) =>
      ns.map((n) => {
        if (n.id !== node) return n;
        const desc = catalog.get(n.data.kind)?.params.find((p) => p.id === param);
        if (!desc || desc.kind === "file") return n;
        const v = desc.kind === "choice" ? (desc.options[value] ?? n.data.params[param]) : value;
        return { ...n, data: { ...n.data, params: { ...n.data.params, [param]: v } } };
      }),
    );
  };

  // --- structure changed: new graph to the engine (one `set_graph`, 150 ms after the last change) ---
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

  return { markSynced };
}
