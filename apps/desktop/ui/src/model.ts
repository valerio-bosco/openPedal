// Conversions between the protocol graph and React Flow nodes/cables, plus utility functions.
import type { Edge, Node } from "@xyflow/react";
import {
  INPUT,
  OUTPUT,
  PEDAL_IN,
  PEDAL_OUT,
  type CatalogItem,
  type Category,
  type EdgeSpec,
  type GraphFile,
  type NodeSpec,
  type ParamDesc,
  type ParamValue,
} from "./generated/protocol";

export interface PedalData extends Record<string, unknown> {
  kind: string;
  params: Record<string, ParamValue>;
  bypass?: boolean;
  error?: string;
}
export type PNode = Node<PedalData, "pedal">;

export type NumberDesc = Extract<ParamDesc, { kind: "number" }>;

/** Synthetic entries for the implicit nodes inside a composite pedal. */
export const IMPLICIT_ITEMS: CatalogItem[] = [
  { kind: PEDAL_IN, label: "Pedal input", category: "io", params: [], composite: false },
  { kind: PEDAL_OUT, label: "Pedal output", category: "io", params: [], composite: false },
];

export const CATEGORY_LABEL: Record<Category, string> = {
  io: "Input / output",
  dynamics: "Dynamics",
  drive: "Distortion",
  filter: "Filters",
  modulation: "Modulation",
  time: "Time",
  amp: "Amps & cabs",
  utility: "Utility",
};
export const CATEGORY_ORDER: Category[] = ["dynamics", "drive", "amp", "filter", "modulation", "time", "utility", "io"];
export const CATEGORY_COLOR: Record<Category, string> = {
  io: "#8d93a1",
  dynamics: "#3fb6a8",
  drive: "#e5683c",
  filter: "#9b6cf0",
  modulation: "#3c9be5",
  time: "#e5b93c",
  amp: "#c9503f",
  utility: "#7a8a99",
};

export const isFixed = (kind: string) => kind === INPUT || kind === OUTPUT || kind === PEDAL_IN || kind === PEDAL_OUT;

/** Positions in files are compact (~220 px): on screen the nodes are wider, so the x axis is stretched. */
export const X_SCALE = 1.5;

export const edgeId = (from: string, to: string) => `${from}->${to}`;

export function specToNode(n: NodeSpec): PNode {
  return {
    id: n.id,
    type: "pedal",
    position: { x: n.pos[0] * X_SCALE, y: n.pos[1] },
    deletable: !isFixed(n.type),
    data: { kind: n.type, params: { ...n.params }, ...(n.bypass ? { bypass: true } : {}) },
  };
}

export function specsToFlow(nodes: NodeSpec[], edges: EdgeSpec[]) {
  return {
    nodes: nodes.map(specToNode),
    edges: edges.map((e): Edge => ({ id: edgeId(e.from, e.to), source: e.from, target: e.to })),
  };
}

export function flowToSpecs(nodes: PNode[], edges: Edge[]) {
  const node: NodeSpec[] = nodes
    .filter((n) => n.data.kind !== PEDAL_IN && n.data.kind !== PEDAL_OUT)
    .map((n) => ({
      id: n.id,
      type: n.data.kind,
      pos: [Math.round(n.position.x / X_SCALE), Math.round(n.position.y)] as [number, number],
      params: { ...n.data.params },
      ...(n.data.bypass ? { bypass: true } : {}),
    }));
  const edge: EdgeSpec[] = edges.map((e) => ({ from: e.source, to: e.target }));
  return { node, edge };
}

export function flowToGraph(name: string, nodes: PNode[], edges: Edge[]): GraphFile {
  return { name, version: 2, ...flowToSpecs(nodes, edges) };
}

/** True if adding source -> target would create a cycle (or a cable to itself). */
export function wouldCycle(edges: { source: string; target: string }[], source: string, target: string): boolean {
  if (source === target) return true;
  const next = new Map<string, string[]>();
  for (const e of edges) next.set(e.source, [...(next.get(e.source) ?? []), e.target]);
  const seen = new Set<string>();
  const stack = [target];
  while (stack.length) {
    const cur = stack.pop()!;
    if (cur === source) return true;
    if (seen.has(cur)) continue;
    seen.add(cur);
    stack.push(...(next.get(cur) ?? []));
  }
  return false;
}

/** Signature of the graph structure: changes only when a `set_graph` is needed (not for positions). */
export function structureKey(nodes: PNode[], edges: Edge[], catalog: Map<string, CatalogItem>): string {
  const ns = nodes.map((n) => {
    const item = catalog.get(n.data.kind);
    const files = (item?.params ?? [])
      .filter((p) => p.kind === "file")
      .map((p) => `${p.id}=${String(n.data.params[p.id] ?? "")}`)
      .join(",");
    return `${n.id}:${n.data.kind}:${files}`;
  });
  const es = edges.map((e) => `${e.source}>${e.target}`);
  return JSON.stringify([ns.sort(), es.sort()]);
}

// --- parameter values ---

export function paramValue(desc: ParamDesc, v: ParamValue | undefined): number {
  if (desc.kind === "number") return typeof v === "number" ? v : desc.default;
  if (desc.kind === "choice") {
    if (typeof v === "number") return v;
    const i = typeof v === "string" ? desc.options.indexOf(v) : -1;
    return i >= 0 ? i : desc.default;
  }
  return 0;
}

export function toNorm(d: NumberDesc, v: number): number {
  const c = Math.min(d.max, Math.max(d.min, v));
  if (d.log && d.min > 0) return Math.log(c / d.min) / Math.log(d.max / d.min);
  return d.max === d.min ? 0 : (c - d.min) / (d.max - d.min);
}

export function fromNorm(d: NumberDesc, t: number): number {
  const c = Math.min(1, Math.max(0, t));
  let v = d.log && d.min > 0 ? d.min * Math.pow(d.max / d.min, c) : d.min + c * (d.max - d.min);
  if (d.step > 0) v = Math.round(v / d.step) * d.step;
  return Math.min(d.max, Math.max(d.min, v));
}

export function formatNumber(d: NumberDesc, v: number): string {
  const range = d.max - d.min;
  const dec = d.step >= 1 ? 0 : range >= 500 ? 0 : range >= 50 ? 1 : range >= 5 ? 2 : 3;
  const s = v.toFixed(dec);
  return dec > 0 ? s.replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "") : s;
}

export function slug(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function uniqueId(base: string, taken: Set<string>): string {
  let i = 1;
  while (taken.has(`${base}-${i}`)) i++;
  return `${base}-${i}`;
}
