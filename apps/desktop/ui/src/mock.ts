// In-memory fake server for `npm run dev` without openPedal: speaks the §5.4 protocol
// using the fixtures. It also serves as automatic fallback when `/ws` does not respond.
import stateFixture from "./fixtures/state.json";
import clientFixture from "./fixtures/client-messages.json";
import { PROTOCOL_VERSION, type CatalogItem, type ClientMsg, type GraphFile, type PedalFile, type ServerMsg } from "./generated/protocol";

type StateMsg = Extract<ServerMsg, { type: "state" }>;
const base = stateFixture as unknown as StateMsg;

/** Example pedals (content), taken from the client messages fixture. */
export function fixturePedals(): Record<string, PedalFile> {
  const out: Record<string, PedalFile> = {};
  for (const m of clientFixture as unknown as ClientMsg[]) {
    if (m.type === "save_pedal") out[m.id] = m.pedal;
  }
  return out;
}

/** Catalog entry for a composite pedal: one 0..1 knob per `knob`. */
export function pedalCatalogItem(id: string, p: PedalFile): CatalogItem {
  return {
    kind: `pedal:${id}`,
    label: p.name,
    category: (p.category || "utility") as CatalogItem["category"],
    composite: true,
    description: p.description || undefined,
    params: p.knob.map((k) => ({
      id: k.label,
      label: k.label,
      kind: "number" as const,
      min: 0,
      max: 1,
      default: k.default,
      unit: "",
      log: false,
      step: 0,
    })),
  };
}

export class MockServer {
  private graph: GraphFile = structuredClone(base.graph);
  private presets = [...base.presets];
  private pedals = fixturePedals();
  private catalog: CatalogItem[] = structuredClone(base.catalog);
  private timer: number | undefined;
  private t0 = performance.now();

  constructor(private emit: (m: ServerMsg) => void) {}

  start() {
    this.timer = window.setInterval(() => {
      const t = (performance.now() - this.t0) / 1000;
      const noise = () => (Math.random() - 0.5) * 3;
      this.emit({
        type: "meters",
        in_db: -26 + 10 * Math.sin(t * 3.1) * Math.sin(t * 0.7) + noise(),
        out_db: -20 + 8 * Math.sin(t * 3.1 + 0.4) * Math.sin(t * 0.7) + noise(),
        dsp: 0.04 + 0.01 * Math.sin(t),
        xruns: 0,
      });
    }, 50);
  }

  stop() {
    window.clearInterval(this.timer);
  }

  private state(): StateMsg {
    return {
      type: "state",
      protocol: PROTOCOL_VERSION,
      catalog: this.catalog,
      graph: this.graph,
      presets: this.presets,
      pedals: this.pedals,
      audio: { ...base.audio, running: false },
    };
  }

  /** Asynchronous reply, like a real server. */
  handle(msg: ClientMsg) {
    window.setTimeout(() => this.handleNow(msg), 0);
  }

  private handleNow(msg: ClientMsg) {
    switch (msg.type) {
      case "hello":
        this.emit(this.state());
        break;
      case "set_graph":
        this.graph = msg.graph;
        break;
      case "load_preset": {
        const known = msg.name === "crunch-graph" || msg.name === base.graph.name;
        this.graph = known
          ? structuredClone(base.graph)
          : {
              name: msg.name,
              version: 2,
              node: [
                { id: "in", type: "input", pos: [0, 120], params: {} },
                { id: "out", type: "output", pos: [400, 120], params: {} },
              ],
              edge: [{ from: "in", to: "out" }],
            };
        this.emit(this.state());
        break;
      }
      case "save_preset":
        if (!this.presets.includes(msg.name)) this.presets = [...this.presets, msg.name];
        this.graph = { ...msg.graph, name: msg.name };
        this.emit(this.state());
        break;
      case "save_pedal":
        this.pedals[msg.id] = msg.pedal;
        this.catalog = [
          ...this.catalog.filter((c) => c.kind !== `pedal:${msg.id}`),
          pedalCatalogItem(msg.id, msg.pedal),
        ];
        this.emit(this.state());
        break;
      case "delete_preset":
        this.presets = this.presets.filter((p) => p !== msg.name);
        this.emit(this.state());
        break;
      case "delete_pedal":
        delete this.pedals[msg.id];
        this.catalog = this.catalog.filter((c) => c.kind !== `pedal:${msg.id}`);
        this.emit(this.state());
        break;
      case "set_param":
        break;
      case "set_bypass":
        this.graph = {
          ...this.graph,
          node: this.graph.node.map((n) => {
            if (n.id !== msg.node) return n;
            const { bypass: _old, ...rest } = n;
            return msg.bypass ? { ...rest, bypass: true } : rest;
          }),
        };
        break;
    }
  }
}
