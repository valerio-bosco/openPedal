// Local history for undo / redo inside a composite pedal (see hooks/useHistory.ts): snapshots of the
// pedal's inner graph. A continuous edit (turning a knob, dragging) becomes a single step because
// the snapshot is recorded only when the graph stays still for a moment.
import type { Edge } from "@xyflow/react";
import type { PNode } from "./model";

export interface Snapshot {
  nodes: PNode[];
  edges: Edge[];
}

const LIMIT = 100;

/** What matters for history: not the selection, not the errors shown. */
function contentKey(s: Snapshot): string {
  return JSON.stringify([
    s.nodes.map((n) => [n.id, n.data.kind, Math.round(n.position.x), Math.round(n.position.y), n.data.params]),
    s.edges.map((e) => `${e.source}>${e.target}`).sort(),
  ]);
}

function clean(s: Snapshot): Snapshot {
  return {
    nodes: s.nodes.map((n) => ({ ...n, selected: false, dragging: false, data: { ...n.data, error: undefined } })),
    edges: s.edges.map((e) => ({ ...e, selected: false }))
  };
}

export class History {
  private past: Snapshot[] = [];
  private future: Snapshot[] = [];
  private current?: Snapshot;
  private key = "";

  /** New starting point (preset loaded, entering a pedal): nothing to undo. */
  reset(s: Snapshot) {
    this.past = [];
    this.future = [];
    this.current = clean(s);
    this.key = contentKey(s);
  }

  /** Records `s` if it changed since the last snapshot; true if it recorded. */
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
