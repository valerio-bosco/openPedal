// Composite pedals: opening the designer (new from a selection, or an existing pedal), editing the
// inner graph in the node editor and coming back, saving, deleting.
import { useState, type Dispatch, type SetStateAction } from "react";
import type { Edge } from "@xyflow/react";
import { PEDAL_PREFIX, type CatalogItem, type PedalFile } from "../generated/protocol";
import { newPedalId } from "../lib/designer";
import { draftFromSelection, keptParams, pedalInnerGraph, replaceSelection, withInnerGraph } from "../lib/pedal";
import { flowToGraph, type PNode } from "../model";
import type { GraphEditor } from "./useGraphEditor";
import type { HistoryControl } from "./useHistory";
import type { useServer } from "./useServer";

/** A pedal open in the designer. `id` is null for a pedal that is not saved yet (made from `ids`). */
export interface Designer {
  id: string | null;
  /** Nodes of the main graph that become the new pedal. */
  ids: string[];
  draft: PedalFile;
}

/** Editing state for "inside the pedal": the main graph stays in `parent`, the designer's state in `designer`. */
export interface Editing {
  designer: Designer;
  name: string;
  parent: { nodes: PNode[]; edges: Edge[] };
}

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
  const [designer, setDesigner] = useState<Designer | null>(null);

  /** The designer for a new pedal made from the selected nodes. */
  const openCreate = (ids: string[]) => {
    if (editing || ids.length === 0) return;
    setDesigner({ id: null, ids, draft: draftFromSelection(nodes, edges, ids) });
  };

  /** The designer for a saved pedal, from a node of that pedal or from its catalog entry. */
  const openEdit = (kind: string) => {
    if (editing || !kind.startsWith(PEDAL_PREFIX)) return;
    const id = kind.slice(PEDAL_PREFIX.length);
    const p = pedals.current[id];
    if (!p) return toast("The contents of this pedal are not available (pedals/ file missing or invalid).");
    setDesigner({ id, ids: [], draft: structuredClone(p) });
  };

  const closeDesigner = () => setDesigner(null);
  const changeDraft = (draft: PedalFile) => setDesigner((d) => (d ? { ...d, draft } : d));

  /** "Edit inner graph": the node editor shows the pedal's blocks; `finishInner` brings the result back. */
  const editInner = (draft: PedalFile) => {
    if (!designer) return;
    const inner = pedalInnerGraph(draft);
    setEditing({ designer: { ...designer, draft }, name: draft.name, parent: { nodes, edges } });
    setDesigner(null);
    history.reset(inner);
    setNodes(inner.nodes);
    setEdges(inner.edges);
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.3 }), 50);
  };

  const finishInner = (keep: boolean) => {
    if (!editing) return;
    const d = editing.designer;
    const draft = keep ? withInnerGraph(d.draft, nodes, edges) : d.draft;
    setNodes(editing.parent.nodes);
    setEdges(editing.parent.edges);
    setEditing(null);
    setDesigner({ ...d, draft });
    window.setTimeout(() => flow.fitView({ duration: 250, padding: 0.2 }), 50);
  };

  const savePedal = () => {
    if (!designer) return;
    const { draft } = designer;
    if (designer.id === null) {
      const id = newPedalId(draft.name, catalog);
      const kind = PEDAL_PREFIX + id;
      const built = replaceSelection(nodes, edges, designer.ids, id, kind);
      pedals.current[id] = draft;
      // the main graph is replaced once the engine has the new pedal in its catalog; the designer stays
      // open until then, so a server error can be fixed in place
      whenInCatalog(kind, () => {
        setNodes(built.nodes);
        setEdges(built.edges);
        markSynced(built.nodes, built.edges);
        send({ type: "set_graph", graph: flowToGraph(graphName, built.nodes, built.edges) });
        setDesigner(null);
        toast(`Pedal «${draft.name}» created`);
      });
      send({ type: "save_pedal", id, pedal: draft });
      return;
    }
    const id = designer.id;
    const before = pedals.current[id];
    pedals.current[id] = draft;
    send({ type: "save_pedal", id, pedal: draft });
    // the changed pedal must be recompiled into the main graph; values of removed controls go
    const kind = PEDAL_PREFIX + id;
    const next = nodes.map((n) => (n.data.kind === kind && before ? { ...n, data: { ...n.data, params: keptParams(n.data.params, before, draft) } } : n));
    setNodes(next);
    markSynced(next, edges);
    send({ type: "set_graph", graph: flowToGraph(graphName, next, edges) });
    setDesigner(null);
    toast(`Pedal «${draft.name}» saved`);
  };

  const deletePedal = (item: CatalogItem) => {
    if (!window.confirm(`Delete pedal «${item.label}»?`)) return;
    send({ type: "delete_pedal", id: item.kind.replace(/^pedal:/, "") });
  };

  return { designer, openCreate, openEdit, closeDesigner, changeDraft, editInner, finishInner, savePedal, deletePedal };
}
