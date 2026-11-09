// Pure construction of composite pedals: the inner graph of a selection, the main graph with the
// selection replaced by the new pedal, opening a pedal's inner graph for editing, and taking it back.
import type { Edge } from "@xyflow/react";
import { PEDAL_IN, PEDAL_OUT, type EdgeSpec, type NodeSpec, type ParamValue, type PedalFile } from "../generated/protocol";
import { X_SCALE, edgeId, flowToSpecs, specToNode, specsToFlow, uniqueId, type PNode } from "../model";
import { COLORS } from "./designer";

/** A new pedal draft (format v2, no controls yet) whose inner graph is the selected nodes. */
export function draftFromSelection(nodes: PNode[], edges: Edge[], ids: string[]): PedalFile {
  const sel = nodes.filter((n) => ids.includes(n.id));
  const inSel = new Set(ids);
  const minX = Math.min(...sel.map((n) => n.position.x));
  const minY = Math.min(...sel.map((n) => n.position.y));
  const inner: NodeSpec[] = sel.map((n) => ({
    id: n.id,
    type: n.data.kind,
    pos: [Math.round((n.position.x - minX) / X_SCALE), Math.round(n.position.y - minY)],
    params: { ...n.data.params },
    ...(n.data.bypass ? { bypass: true } : {}),
  }));
  const internal = edges.filter((e) => inSel.has(e.source) && inSel.has(e.target));
  const hasInternalIn = new Set(internal.map((e) => e.target));
  const hasInternalOut = new Set(internal.map((e) => e.source));
  const entries = new Set(edges.filter((e) => !inSel.has(e.source) && inSel.has(e.target)).map((e) => e.target));
  const exits = new Set(edges.filter((e) => inSel.has(e.source) && !inSel.has(e.target)).map((e) => e.source));
  if (entries.size === 0) ids.filter((i) => !hasInternalIn.has(i)).forEach((i) => entries.add(i));
  if (exits.size === 0) ids.filter((i) => !hasInternalOut.has(i)).forEach((i) => exits.add(i));
  const edge: EdgeSpec[] = [
    ...internal.map((e) => ({ from: e.source, to: e.target })),
    ...[...entries].map((to) => ({ from: PEDAL_IN, to })),
    ...[...exits].map((from) => ({ from, to: PEDAL_OUT })),
  ];
  return { name: "New pedal", version: 2, category: "utility", description: "", color: COLORS[0], node: inner, edge, control: [] };
}

/** The main graph with the selection replaced by one node of type `kind`. */
export function replaceSelection(nodes: PNode[], edges: Edge[], ids: string[], id: string, kind: string): { nodes: PNode[]; edges: Edge[] } {
  const inSel = new Set(ids);
  const sel = nodes.filter((n) => inSel.has(n.id));
  const minX = Math.min(...sel.map((n) => n.position.x));
  const minY = Math.min(...sel.map((n) => n.position.y));
  const extIn = edges.filter((e) => !inSel.has(e.source) && inSel.has(e.target));
  const extOut = edges.filter((e) => inSel.has(e.source) && !inSel.has(e.target));
  const nodeId = uniqueId("p-" + id, new Set(nodes.map((n) => n.id)));
  const pos: [number, number] = [Math.round(minX / X_SCALE), Math.round(minY)];
  const newNode = specToNode({ id: nodeId, type: kind, pos, params: {} });
  const newEdges: Edge[] = [
    ...extIn.map((e) => ({ id: edgeId(e.source, nodeId), source: e.source, target: nodeId })),
    ...extOut.map((e) => ({ id: edgeId(nodeId, e.target), source: nodeId, target: e.target })),
  ].filter((e, i, a) => a.findIndex((x) => x.id === e.id) === i);
  return {
    nodes: [...nodes.filter((n) => !inSel.has(n.id)).map((n) => ({ ...n, selected: false })), { ...newNode, selected: true }],
    edges: [...edges.filter((e) => !inSel.has(e.source) && !inSel.has(e.target)), ...newEdges],
  };
}

/** The graph shown when editing a pedal's inner graph: its nodes between the implicit input and output. */
export function pedalInnerGraph(p: PedalFile): { nodes: PNode[]; edges: Edge[] } {
  const f = specsToFlow(p.node, p.edge);
  const xs = p.node.map((n) => n.pos[0]);
  const ys = p.node.map((n) => n.pos[1]);
  const midY = ys.length ? (Math.min(...ys) + Math.max(...ys)) / 2 : 0;
  const nIn = specToNode({ id: PEDAL_IN, type: PEDAL_IN, pos: [(xs.length ? Math.min(...xs) : 0) - 260, midY], params: {} });
  const nOut = specToNode({ id: PEDAL_OUT, type: PEDAL_OUT, pos: [(xs.length ? Math.max(...xs) : 0) + 300, midY], params: {} });
  return { nodes: [nIn, ...f.nodes, nOut], edges: f.edges };
}

/** The draft with the inner graph that was edited in the node editor; name, colour and controls are kept. */
export function withInnerGraph(draft: PedalFile, nodes: PNode[], edges: Edge[]): PedalFile {
  const { node, edge } = flowToSpecs(nodes, edges);
  return { ...draft, node, edge };
}

/** Parameters of a node of an edited pedal that still match a control (a removed or retyped control is dropped). */
export function keptParams(params: Record<string, ParamValue>, before: PedalFile, after: PedalFile): Record<string, ParamValue> {
  const out: Record<string, ParamValue> = {};
  for (const [k, v] of Object.entries(params)) {
    const was = before.control.find((c) => c.id === k);
    const now = after.control.find((c) => c.id === k);
    if (now && was?.kind === now.kind) out[k] = v;
  }
  return out;
}
