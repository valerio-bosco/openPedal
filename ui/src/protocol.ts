// Tipi del protocollo tra interfaccia e openPedal (contratto: docs/PIANO.md §5.4).
// Rispecchiano src/server/protocol.rs e src/graph/model.rs. Gli esempi reali stanno
// in ./fixtures/*.json, generati con `openpedal dev-fixtures ui/src/fixtures`.

export const INPUT = "input";
export const OUTPUT = "output";
/** Un nodo di tipo `pedal:<id>` usa il pedale composto pedals/<id>.toml. */
export const PEDAL_PREFIX = "pedal:";
/** Nodi impliciti dentro un pedale composto (non compaiono in `node`). */
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
      /** La manopola si muove in scala logaritmica. */
      log: boolean;
      /** > 0: valori a scatti (1 = interi). */
      step: number;
    }
  | { kind: "choice"; options: string[]; default: number }
  /** Percorso di un file: cambiarlo richiede `set_graph`, non `set_param`. */
  | { kind: "file"; extensions: string[] }
);

export interface CatalogItem {
  /** Il `type` dei nodi: "drive", "input", "pedal:cassa-4x12"... */
  kind: string;
  label: string;
  category: Category;
  params: ParamDesc[];
  /** Pedale composto: modificabile e cancellabile. I suoi parametri sono manopole 0..1. */
  composite: boolean;
  description?: string;
}

/**
 * Nel grafo i valori sono numeri, oppure stringhe per le scelte (nome dell'opzione,
 * es. "asym") e per i file. Un parametro assente vale il suo default.
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

/** Preset v2. Le chiavi sono al singolare (`node`, `edge`) come nei file TOML. */
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
  /** Valori del parametro interno con la manopola a 0 e a 1. */
  min: number;
  max: number;
  /** Posizione iniziale, 0..1. */
  default: number;
}

export interface PedalFile {
  name: string;
  category: string;
  description: string;
  node: NodeSpec[];
  /** Usano anche i nodi impliciti PEDAL_IN e PEDAL_OUT. */
  edge: EdgeSpec[];
  knob: KnobSpec[];
}

export interface AudioInfo {
  sample_rate: number;
  buffer: number;
  /** false = nessuna scheda audio (modalità solo editor). */
  running: boolean;
}

export interface StateMsg {
  type: "state";
  catalog: CatalogItem[];
  graph: GraphFile;
  presets: string[];
  /** Contenuto dei pedali composti (id -> pedale), per aprirli e modificarli. */
  pedals: Record<string, PedalFile>;
  audio: AudioInfo;
}

export interface MetersMsg {
  type: "meters";
  in_db: number;
  out_db: number;
  /** Carico del thread audio, 0..1. */
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
  /** Per le scelte `value` è l'indice dell'opzione. */
  | { type: "set_param"; node: string; param: string; value: number }
  | { type: "set_graph"; graph: GraphFile }
  | { type: "load_preset"; name: string }
  | { type: "save_preset"; name: string; graph: GraphFile }
  | { type: "save_pedal"; id: string; pedal: PedalFile }
  | { type: "delete_pedal"; id: string };
