// Pure construction of composite pedals: wrapping a selection, opening one for editing, saving the edit.
import type { Edge } from "@xyflow/react";
import {
  PEDAL_IN,
  PEDAL_OUT,
  PEDAL_PREFIX,
  type CatalogItem,
  type EdgeSpec,
  type NodeSpec,
  type ParamValue,
  type PedalFile,
} from "../generated/protocol";
import { X_SCALE, edgeId, flowToSpecs, paramValue, slug, specToNode, specsToFlow, uniqueId, type PNode } from "../model";
import type { Candidate, PedalForm } from "../components/PedalDialog";

/** Numeric parameters of the given pedals that can be exposed as knobs. */
export function candidatesFor(list: PNode[], catalog: Map<string, CatalogItem>): Candidate[] {
  return list.flatMap((n) => {
    const item = catalog.get(n.data.kind);
    return (item?.params ?? []).flatMap((p) =>
      p.kind === "number"
        ? [{ nodeId: n.id, nodeLabel: item!.label, desc: p, current: paramValue(p, n.data.params[p.id]) }]
        : [],
    );
  });
}

/** The new pedal made from the selected nodes, and the main graph with the selection replaced by one node. */
export function buildPedal(
  nodes: PNode[],
  edges: Edge[],
  ids: string[],
  f: PedalForm,
  catalog: Map<string, CatalogItem>,
): { id: string; kind: string; pedal: PedalFile; nodes: PNode[]; edges: Edge[] } {
  const sel = nodes.filter((n) => ids.includes(n.id));
  const inSel = new Set(ids);
  let id = slug(f.name) || "pedal";
  for (let i = 2; catalog.has(PEDAL_PREFIX + id); i++) id = `${slug(f.name) || "pedal"}-${i}`;
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

  return {
    id,
    kind,
    pedal,
    nodes: [...keep.map((n) => ({ ...n, selected: false })), { ...newNode, selected: true }],
    edges: [...keepEdges, ...newEdges],
  };
}

/** The graph shown when entering a pedal: its nodes between the implicit input and output. */
export function pedalInnerGraph(p: PedalFile): { nodes: PNode[]; edges: Edge[] } {
  const f = specsToFlow(p.node, p.edge);
  const xs = p.node.map((n) => n.pos[0]);
  const ys = p.node.map((n) => n.pos[1]);
  const midY = ys.length ? (Math.min(...ys) + Math.max(...ys)) / 2 : 0;
  const nIn = specToNode({ id: PEDAL_IN, type: PEDAL_IN, pos: [(xs.length ? Math.min(...xs) : 0) - 260, midY], params: {} });
  const nOut = specToNode({ id: PEDAL_OUT, type: PEDAL_OUT, pos: [(xs.length ? Math.max(...xs) : 0) + 300, midY], params: {} });
  return { nodes: [nIn, ...f.nodes, nOut], edges: f.edges };
}

/** The pedal file for the graph being edited; knobs pointing at removed nodes are dropped. */
export function editedPedal(meta: PedalForm, nodes: PNode[], edges: Edge[]): PedalFile {
  const { node, edge } = flowToSpecs(nodes, edges);
  const alive = new Set(node.map((n) => n.id));
  const knobs = meta.knobs.filter((k) => alive.has(k.target.split(".")[0]));
  return { name: meta.name, category: meta.category, description: meta.description, node, edge, knob: knobs };
}
