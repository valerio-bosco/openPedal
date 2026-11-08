// Protocol types between the interface and openPedal (contract: docs/PLAN.md §5.4).
// They mirror src/server/protocol.rs and src/graph/model.rs. Real examples are in
// in ./fixtures/*.json, generated with `openpedal dev-fixtures ui/src/fixtures`.

export const INPUT = "input";
export const OUTPUT = "output";
/** A node of type `pedal:<id>` uses the composite pedal pedals/<id>.toml. */
export const PEDAL_PREFIX = "pedal:";
/** Implicit nodes inside a composite pedal (they do not appear in `node`). */
export const PEDAL_IN = "pedal_in";
export const PEDAL_OUT = "pedal_out";

export type Category = "io" | "dynamics" | "drive" | "filter" | "modulation" | "time" | "amp" | "utility";

export type ParamDesc = { id: string; label: string } & (
  | {
      kind: "number";
      min: number;
      max: number;
      default: number;
      unit: string;
      /** The knob moves on a logarithmic scale. */
      log: boolean;
      /** > 0: stepped values (1 = integers). */
      step: number;
    }
  | { kind: "choice"; options: string[]; default: number }
  /** File path: changing it requires `set_graph`, not `set_param`. */
  | { kind: "file"; extensions: string[] }
);

export interface CatalogItem {
  /** The `type` of the nodes: "drive", "input", "pedal:cab-4x12"... */
  kind: string;
  label: string;
  category: Category;
  params: ParamDesc[];
  /** Composite pedal: editable and deletable. Its parameters are 0..1 knobs. */
  composite: boolean;
  description?: string;
}

/**
 * In the graph, values are numbers, or strings for choices (the option name,
 * e.g. "asym") and for files. A missing parameter takes its default.
 */
export type ParamValue = number | string;

export interface NodeSpec {
  id: string;
  type: string;
  pos: [number, number];
  params: Record<string, ParamValue>;
}

export interface EdgeSpec {
  from: string;
  to: string;
}

/** Preset v2. The keys are singular (`node`, `edge`), as in the TOML files. */
export interface GraphFile {
  name: string;
  version: number;
  node: NodeSpec[];
  edge: EdgeSpec[];
}

export interface KnobSpec {
  label: string;
  /** "<id nodo interno>.<id parametro>" */
  target: string;
  /** Internal parameter values with the knob at 0 and at 1. */
  min: number;
  max: number;
  /** Initial position, 0..1. */
  default: number;
}

export interface PedalFile {
  name: string;
  category: string;
  description: string;
  node: NodeSpec[];
  /** They also use the implicit PEDAL_IN and PEDAL_OUT nodes. */
  edge: EdgeSpec[];
  knob: KnobSpec[];
}

export interface AudioInfo {
  sample_rate: number;
  buffer: number;
  /** false = no audio device (editor-only mode). */
  running: boolean;
}

export interface StateMsg {
  type: "state";
  catalog: CatalogItem[];
  graph: GraphFile;
  presets: string[];
  /** Content of the composite pedals (id -> pedal), to open and edit them. */
  pedals: Record<string, PedalFile>;
  audio: AudioInfo;
}

export interface MetersMsg {
  type: "meters";
  in_db: number;
  out_db: number;
  /** Audio thread load, 0..1. */
  dsp: number;
  xruns: number;
}

export interface ErrorMsg {
  type: "error";
  message: string;
  node?: string;
}

export type ServerMsg = StateMsg | MetersMsg | ErrorMsg;

export type ClientMsg =
  | { type: "hello" }
  /** For choices, `value` is the index of the option. */
  | { type: "set_param"; node: string; param: string; value: number }
  | { type: "set_graph"; graph: GraphFile }
  | { type: "load_preset"; name: string }
  | { type: "save_preset"; name: string; graph: GraphFile }
  /** Deletes presets/<name>.toml; the graph currently playing does not change. */
  | { type: "delete_preset"; name: string }
  | { type: "save_pedal"; id: string; pedal: PedalFile }
  | { type: "delete_pedal"; id: string };
