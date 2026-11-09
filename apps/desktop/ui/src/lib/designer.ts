// Pure logic of the pedal designer: control defaults, how routes combine into inner parameter values
// (mirrors `apply_routes` in crates/graph/src/compile.rs), route construction and client-side validation
// (mirrors the main rules of `validate_pedal`; the server stays the authority).
import type {
  CatalogItem,
  ControlSpec,
  ControlType,
  MapKind,
  NodeSpec,
  ParamDesc,
  ParamValue,
  PedalFile,
  RouteSpec,
} from "../generated/protocol";
import { PEDAL_PREFIX } from "../generated/protocol";
import { formatNumber, paramValue, slug } from "../model";

export const COLORS = ["#d9573b", "#e8423a", "#9b6cf0", "#3fb6d3", "#2fbf71", "#e2c25a", "#4f86f7", "#8e98a6"];

export const CONTROL_KINDS: ControlType[] = ["knob", "switch", "selector"];
export const CURVES: { kind: MapKind; label: string }[] = [
  { kind: "linear", label: "Linear" },
  { kind: "log", label: "Log" },
  { kind: "inverse", label: "Inverse" },
];

type NumberParam = Extract<ParamDesc, { kind: "number" }>;
type ChoiceParam = Extract<ParamDesc, { kind: "choice" }>;
const isNumber = (d: ParamDesc | undefined): d is NumberParam => d?.kind === "number";
const isChoice = (d: ParamDesc | undefined): d is ChoiceParam => d?.kind === "choice";

// --- targets ---

export function splitTarget(target: string): { node: string; param: string } {
  const i = target.indexOf(".");
  return i < 0 ? { node: "", param: target } : { node: target.slice(0, i), param: target.slice(i + 1) };
}

export interface Resolved {
  node?: NodeSpec;
  item?: CatalogItem;
  desc?: ParamDesc;
}

export function resolveTarget(draft: PedalFile, catalog: Map<string, CatalogItem>, target: string): Resolved {
  const { node: nodeId, param } = splitTarget(target);
  const node = draft.node.find((n) => n.id === nodeId);
  const item = node ? catalog.get(node.type) : undefined;
  return { node, item, desc: item?.params.find((p) => p.id === param) };
}

/** The value of a parameter as a number: a choice is its option index. */
export function numeric(desc: ParamDesc, v: ParamValue | undefined): number {
  return paramValue(desc, v);
}

/** Text for a parameter value (option name for a choice, number with its unit otherwise). */
export function formatValue(desc: ParamDesc | undefined, n: number | undefined): string {
  if (n === undefined || !desc) return "-";
  if (desc.kind === "choice") return desc.options[Math.round(n)] ?? String(n);
  if (desc.kind === "number") return `${formatNumber(desc, n)}${desc.unit ? " " + desc.unit : ""}`;
  return String(n);
}

// --- controls ---

export function controlDefault(c: ControlSpec): number {
  if (c.kind === "switch") return c.default === true || c.default === 1 ? 1 : 0;
  if (c.kind === "selector") return Math.max(0, Math.min((c.options?.length ?? 1) - 1, Math.round(Number(c.default ?? 0))));
  return typeof c.default === "number" ? Math.max(0, Math.min(1, c.default)) : 0.5;
}

/** Preview values: every control at its default. */
export function initialValues(draft: PedalFile): Record<string, number> {
  return Object.fromEntries(draft.control.map((c) => [c.id, controlDefault(c)]));
}

const KIND_LABEL: Record<ControlType, string> = { knob: "Knob", switch: "Switch", selector: "Selector" };
export const kindLabel = (k: ControlType) => KIND_LABEL[k];

export function newControl(draft: PedalFile, kind: ControlType = "knob"): ControlSpec {
  const label = kindLabel(kind);
  const ids = new Set(draft.control.map((c) => c.id));
  const labels = new Set(draft.control.map((c) => c.label));
  let id = slug(label);
  let text = label;
  for (let i = 2; ids.has(id) || labels.has(text); i++) {
    id = `${slug(label)}-${i}`;
    text = `${label} ${i}`;
  }
  return { id, label: text, kind, route: [], ...(kind === "knob" ? { default: 0.5 } : {}) };
}

export const round = (x: number) => Number(x.toPrecision(4));

/** A fresh route to `target` for the given control kind. */
export function newRoute(c: ControlSpec, target: string, desc: ParamDesc, current: ParamValue | undefined): RouteSpec {
  const cur = numeric(desc, current);
  if (c.kind === "knob") {
    if (!isNumber(desc)) return { target, min: 0, max: 1 };
    const log = desc.log && desc.min > 0;
    return { target, ...(log ? { map: "log" as const } : {}), min: desc.min, max: desc.max };
  }
  if (c.kind === "switch") {
    if (isChoice(desc)) {
      const n = desc.options.length;
      return { target, off: desc.options[Math.min(n - 1, Math.max(0, cur))], on: desc.options[(Math.round(cur) + 1) % n] };
    }
    if (!isNumber(desc)) return { target, off: 0, on: 1 };
    return { target, off: round(cur), on: cur === desc.max ? desc.min : desc.max };
  }
  const n = c.options?.length ?? 2;
  const values: ParamValue[] = Array.from({ length: n }, (_, i) => {
    if (isChoice(desc)) return desc.options[i % desc.options.length];
    if (!isNumber(desc)) return 0;
    const t = n > 1 ? i / (n - 1) : 0;
    return round(desc.log && desc.min > 0 ? desc.min * Math.pow(desc.max / desc.min, t) : desc.min + (desc.max - desc.min) * t);
  });
  return { target, values };
}

/** Rebuilds a control's routes after its kind changed (the targets stay, their fields are made anew). */
export function convertControl(draft: PedalFile, catalog: Map<string, CatalogItem>, c: ControlSpec, kind: ControlType): ControlSpec {
  if (c.kind === kind) return c;
  const next: ControlSpec = { id: c.id, label: c.label, kind, route: [] };
  if (kind === "knob") next.default = 0.5;
  if (kind === "switch") next.default = false;
  if (kind === "selector") {
    next.options = c.options && c.options.length >= 2 ? c.options : ["A", "B", "C"];
    next.default = 0;
  }
  next.route = c.route.flatMap((r) => {
    const { node, desc } = resolveTarget(draft, catalog, r.target);
    if (!desc) return [];
    if (kind === "knob" && !isNumber(desc)) return []; // a choice cannot be driven by a knob
    return [newRoute(next, r.target, desc, node?.params[splitTarget(r.target).param])];
  });
  return next;
}

/** A selector with a different number of positions: every route's `values` follows. */
export function resizeSelector(c: ControlSpec, options: string[]): ControlSpec {
  const route = c.route.map((r) => {
    const values = r.values ?? [];
    const fill = values[values.length - 1] ?? 0;
    return { ...r, values: options.map((_, i) => values[i] ?? fill) };
  });
  return { ...c, options, route, default: Math.min(Number(c.default ?? 0), options.length - 1) };
}

// --- effective values ---

/** The number a route writes for a control at position `v` (knob 0..1, switch 0/1, selector index). */
export function routeNumber(c: ControlSpec, r: RouteSpec, v: number, desc: ParamDesc): number | undefined {
  const pv = (x: ParamValue | undefined): number | undefined => {
    if (x === undefined) return undefined;
    if (typeof x === "number") return x;
    if (isChoice(desc)) {
      const i = desc.options.indexOf(x);
      return i >= 0 ? i : undefined;
    }
    return undefined;
  };
  if (c.kind === "knob") {
    const n = Math.min(1, Math.max(0, v));
    const { min, max } = r;
    if (min === undefined || max === undefined) return undefined;
    if (r.map === "log") return min > 0 && max > 0 ? min * Math.pow(max / min, n) : undefined;
    if (r.map === "inverse") return max + (min - max) * n;
    return min + (max - min) * n;
  }
  if (c.kind === "switch") return pv(v >= 0.5 ? r.on : r.off);
  const i = Math.min((r.values?.length ?? 1) - 1, Math.max(0, Math.round(v)));
  return pv(r.values?.[i]);
}

/**
 * Value of every driven inner parameter ("node.param" -> number; a choice is its option index) with the
 * controls at `values`. Same rule as the engine: `set` routes replace (last wins), then `add` routes add
 * to that (or to the node's own value), and the result is clamped to the parameter range.
 */
export function effectiveValues(draft: PedalFile, values: Record<string, number>, catalog: Map<string, CatalogItem>): Map<string, number> {
  const sets = new Map<string, number>();
  const adds = new Map<string, number>();
  for (const c of draft.control) {
    const v = values[c.id] ?? controlDefault(c);
    for (const r of c.route) {
      const { desc } = resolveTarget(draft, catalog, r.target);
      if (!desc) continue;
      const n = routeNumber(c, r, v, desc);
      if (n === undefined || !Number.isFinite(n)) continue;
      if (r.mode === "add") adds.set(r.target, (adds.get(r.target) ?? 0) + n);
      else sets.set(r.target, n);
    }
  }
  const out = new Map<string, number>();
  for (const target of new Set([...sets.keys(), ...adds.keys()])) {
    const { node, desc } = resolveTarget(draft, catalog, target);
    if (!desc || !node) continue;
    let n = sets.get(target) ?? numeric(desc, node.params[splitTarget(target).param]);
    n += adds.get(target) ?? 0;
    if (isNumber(desc)) n = Math.min(desc.max, Math.max(desc.min, n));
    out.set(target, n);
  }
  return out;
}

// --- validation ---

export interface Problem {
  /** Control id the problem is about (a problem of the pedal itself has none). */
  control?: string;
  message: string;
}

function routeProblem(draft: PedalFile, catalog: Map<string, CatalogItem>, c: ControlSpec, r: RouteSpec): string | undefined {
  const { node: nodeId, param } = splitTarget(r.target);
  if (!nodeId) return `target '${r.target}' must be 'node.parameter'`;
  const { node, item, desc } = resolveTarget(draft, catalog, r.target);
  if (!node) return `node '${nodeId}' does not exist`;
  if (!item) return `node '${nodeId}' has an unknown type '${node.type}'`;
  if (!desc) return `'${nodeId}' has no parameter '${param}'`;
  const name = r.target;
  if (desc.kind === "file") return `'${name}' is a file and cannot be driven by a control`;
  const finite = (x: unknown) => typeof x === "number" && Number.isFinite(x);
  const checkValue = (v: ParamValue | undefined): string | undefined => {
    if (v === undefined || v === "") return "a value is missing";
    if (desc.kind === "number") return finite(v) ? undefined : "its values must be numbers";
    if (typeof v === "string") return desc.options.includes(v) ? undefined : `"${v}" is not one of its options`;
    return Number.isInteger(v) && v >= 0 && v < desc.options.length ? undefined : `${v} is not one of its options`;
  };
  if (c.kind === "knob") {
    if (desc.kind === "choice") return `'${name}' is a choice: drive it with a switch or a selector, not a knob`;
    if (!finite(r.min) || !finite(r.max)) return `'${name}': a knob route needs min and max`;
    if (r.map === "log" && (r.min! <= 0 || r.max! <= 0)) return `'${name}': a log curve needs min and max greater than 0`;
  } else if (c.kind === "switch") {
    const bad = checkValue(r.off) ?? checkValue(r.on);
    if (bad) return `'${name}': ${bad}`;
  } else {
    const n = c.options?.length ?? 0;
    if (!r.values || r.values.length !== n) return `'${name}': needs one value per position (${n})`;
    for (const v of r.values) {
      const bad = checkValue(v);
      if (bad) return `'${name}': ${bad}`;
    }
  }
  if (r.mode === "add" && (desc.kind !== "number" || item.composite)) return `'${name}': add mode needs a numeric block parameter`;
  return undefined;
}

export function validateDraft(draft: PedalFile, catalog: Map<string, CatalogItem>): Problem[] {
  const out: Problem[] = [];
  if (!draft.name.trim()) out.push({ message: "Give the pedal a name." });
  if (draft.color && !/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(draft.color)) out.push({ message: `Colour '${draft.color}' must look like #d9573b.` });
  if (draft.node.length === 0) out.push({ message: "The pedal needs at least one block inside." });
  const ids = new Map<string, number>();
  const labels = new Map<string, number>();
  for (const c of draft.control) {
    ids.set(c.id, (ids.get(c.id) ?? 0) + 1);
    labels.set(c.label.trim(), (labels.get(c.label.trim()) ?? 0) + 1);
  }
  for (const c of draft.control) {
    const at = (message: string) => out.push({ control: c.id, message });
    if (!c.label.trim()) at("Give the control a name.");
    else if ((labels.get(c.label.trim()) ?? 0) > 1) at(`The name '${c.label}' is used by more than one control.`);
    if ((ids.get(c.id) ?? 0) > 1) at(`The control id '${c.id}' is used more than once.`);
    if (c.kind === "selector") {
      const opts = c.options ?? [];
      if (opts.length < 2) at("A selector needs at least two positions.");
      if (opts.some((o) => !o.trim())) at("Every position needs a name.");
    }
    for (const r of c.route) {
      const p = routeProblem(draft, catalog, c, r);
      if (p) at(p);
    }
  }
  return out;
}

/** A pedal id (file name) for `name` that is not yet in the catalog. */
export function newPedalId(name: string, catalog: Map<string, CatalogItem>): string {
  const base = slug(name).slice(0, 48) || "pedal";
  let id = base;
  for (let i = 2; catalog.has(PEDAL_PREFIX + id); i++) id = `${base}-${i}`;
  return id;
}

/** Shape of a knob curve for its thumbnail: output 0..1 for input 0..1 (log is drawn with a fixed bend). */
export function curveY(kind: MapKind, n: number): number {
  if (kind === "inverse") return 1 - n;
  if (kind === "log") return (Math.pow(10, n) - 1) / 9;
  return n;
}
