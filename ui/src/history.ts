// Cronologia per annulla / ripeti: istantanee del grafo (nodi, posizioni, parametri, cavi).
// Una modifica continua (giro di manopola, trascinamento) diventa un solo passo perché
// l'istantanea si registra solo quando il grafo resta fermo per un attimo (vedi App.tsx).
import type { Edge } from "@xyflow/react";
import type { PNode } from "./model";

export interface Snapshot {
  nodes: PNode[];
  edges: Edge[];
  /** Nome del grafo (es. "nuovo" dopo il pulsante Nuovo): torna con l'annulla. */
  name?: string;
}

const LIMIT = 100;

/** Ciò che conta per la cronologia: non la selezione, non gli errori mostrati. */
function contentKey(s: Snapshot): string {
  return JSON.stringify([
    s.nodes.map((n) => [n.id, n.data.kind, Math.round(n.position.x), Math.round(n.position.y), n.data.params]),
    s.edges.map((e) => `${e.source}>${e.target}`).sort(),
  ]);
}

function clean(s: Snapshot): Snapshot {
  return {
    nodes: s.nodes.map((n) => ({ ...n, selected: false, dragging: false, data: { ...n.data, error: undefined } })),
    edges: s.edges.map((e) => ({ ...e, selected: false })),
    name: s.name,
  };
}

export class History {
  private past: Snapshot[] = [];
  private future: Snapshot[] = [];
  private current?: Snapshot;
  private key = "";

  /** Nuovo punto di partenza (preset caricato, ingresso in un pedale): niente da annullare. */
  reset(s: Snapshot) {
    this.past = [];
    this.future = [];
    // senza nome (es. dopo il riordino automatico) resta quello che c'era
    this.current = clean({ ...s, name: s.name ?? this.current?.name });
    this.key = contentKey(s);
  }

  /** true se `s` è lo stato già registrato. */
  same(s: Snapshot): boolean {
    return contentKey(s) === this.key;
  }

  /** Registra `s` se è cambiato rispetto all'ultima istantanea; true se ha registrato. */
  record(s: Snapshot): boolean {
    const k = contentKey(s);
    if (k === this.key) return false;
    if (this.current) {
      this.past.push(this.current);
      if (this.past.length > LIMIT) this.past.shift();
    }
    this.future = [];
    this.current = clean(s);
    this.key = k;
    return true;
  }

  undo(): Snapshot | undefined {
    const prev = this.past.pop();
    if (!prev || !this.current) return undefined;
    this.future.push(this.current);
    this.current = prev;
    this.key = contentKey(prev);
    return prev;
  }

  redo(): Snapshot | undefined {
    const next = this.future.pop();
    if (!next || !this.current) return undefined;
    this.past.push(this.current);
    this.current = next;
    this.key = contentKey(next);
    return next;
  }

  get canUndo() {
    return this.past.length > 0;
  }

  get canRedo() {
    return this.future.length > 0;
  }
}
