// Composite pedals: creating one from a selection, entering it to edit, saving or leaving, deleting.
import { useState, type Dispatch, type SetStateAction } from "react";
import type { Edge } from "@xyflow/react";
import { PEDAL_PREFIX, type CatalogItem, type Category, type KnobSpec } from "../generated/protocol";
import { flowToGraph, type PNode } from "../model";
import { buildPedal, editedPedal, pedalInnerGraph } from "../lib/pedal";
import type { PedalForm } from "../components/PedalDialog";
import type { GraphEditor } from "./useGraphEditor";
import type { HistoryControl, HistoryScope } from "./useHistory";
import type { useServer } from "./useServer";

/** Editing state for "inside the pedal": the main graph stays in `parent`. */
export interface Editing {
  id: string;
  name: string;
  category: Category;
  description: string;
  knobs: KnobSpec[];
  parent: { nodes: PNode[]; edges: Edge[] };
  /** History of the main graph, restored when leaving the pedal. */
  parentHistory: HistoryScope;
}

export type Dialog = { mode: "create"; ids: string[] } | { mode: "edit" } | null;

interface Deps {
  server: ReturnType<typeof useServer>;
  graph: GraphEditor;
  history: HistoryControl;
  graphName: string;
  toast: (text: string) => void;
  /** Tells the engine sync that the engine already has this graph. */
  markSynced: (nodes: PNode[], edges: Edge[]) => void;
  editing: Editing | null;
  setEditing: Dispatch<SetStateAction<Editing | null>>;
}

export function usePedals({ server, graph, history, graphName, toast, markSynced, editing, setEditing }: Deps) {
  const { pedals, catalog, send, whenInCatalog } = server;
  const { nodes, edges, setNodes, setEdges, flow } = graph;
  const [dialog, setDialog] = useState<Dialog>(null);

  const createPedal = (ids: string[], f: PedalForm) => {
    const built = buildPedal(nodes, edges, ids, f, catalog);
    pedals.current[built.id] = built.pedal;
    // the main graph is replaced once the engine has the new pedal in its catalog
    whenInCatalog(built.kind, () => {
      setNodes(built.nodes);
      setEdges(built.edges);
      markSynced(built.nodes, built.edges);
      send({ type: "set_graph", graph: flowToGraph(graphName, built.nodes, built.edges) });
    });
    send({ type: "save_pedal", id: built.id, pedal: built.pedal });
    setDialog(null);
    toast(`Pedal «${f.name}» created`);
  };

  const enterPedal = (node: PNode) => {
    if (!node.data.kind.startsWith(PEDAL_PREFIX) || editing) return;
    const id = node.data.kind.slice(PEDAL_PREFIX.length);
    const p = pedals.current[id];
    if (!p) return toast("The contents of this pedal are not available (pedals/ file missing or invalid).");
    const inner = pedalInnerGraph(p);
    setEditing({
      id,
      name: p.name,
      category: p.category as Category,
      description: p.description,
      knobs: p.knob,
      parent: { nodes, edges },
      parentHistory: history.detach(inner),
    });
    setNodes(inner.nodes);
    setEdges(inner.edges);
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.3 }), 50);
  };

  const exitPedal = () => {
    if (!editing) return;
    setNodes(editing.parent.nodes);
    setEdges(editing.parent.edges);
    history.restore(editing.parentHistory);
    setEditing(null);
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.2 }), 50);
  };

  const savePedalEdit = (f?: PedalForm) => {
    if (!editing) return;
    const meta = f ?? editing;
    const pedal = editedPedal(meta, nodes, edges);
    pedals.current[editing.id] = pedal;
    send({ type: "save_pedal", id: editing.id, pedal });
    // the changed pedal must be recompiled into the main graph
    const parent = editing.parent;
    send({ type: "set_graph", graph: flowToGraph(graphName, parent.nodes, parent.edges) });
    setDialog(null);
    exitPedal();
    toast(`Pedal «${meta.name}» saved`);
  };

  const deletePedal = (item: CatalogItem) => {
    if (!window.confirm(`Delete pedal «${item.label}»?`)) return;
    send({ type: "delete_pedal", id: item.kind.replace(/^pedal:/, "") });
  };

  return { dialog, setDialog, createPedal, enterPedal, exitPedal, savePedalEdit, deletePedal };
}
