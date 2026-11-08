// Server finto in memoria per `npm run dev` senza openPedal: parla il protocollo §5.4
// usando le fixture. Serve anche da ripiego automatico quando `/ws` non risponde.
import stateFixture from "./fixtures/state.json";
import clientFixture from "./fixtures/client-messages.json";
import type { CatalogItem, ClientMsg, GraphFile, PedalFile, ServerMsg, StateMsg } from "./protocol";

const base = stateFixture as unknown as StateMsg;

/** Pedali di esempio (contenuto), ricavati dalla fixture dei messaggi del client. */
export function fixturePedals(): Record<string, PedalFile> {
  const out: Record<string, PedalFile> = {};
  for (const m of clientFixture as unknown as ClientMsg[]) {
    if (m.type === "save_pedal") out[m.id] = m.pedal;
  }
  return out;
}

/** Voce di catalogo di un pedale composto: una manopola 0..1 per ogni `knob`. */
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
      catalog: this.catalog,
      graph: this.graph,
      presets: this.presets,
      pedals: this.pedals,
      audio: { ...base.audio, running: false },
    };
  }

  /** Risposta asincrona, come un vero server. */
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
        const known = msg.name === "crunch-grafo" || msg.name === base.graph.name;
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
    }
  }
}
