// Undo / redo of the graph. The editor talks to this interface only, so the client-side
// snapshots (history.ts) can later be replaced by server-side undo.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Edge } from "@xyflow/react";
import { History, type Snapshot } from "../history";
import type { PNode } from "../model";

/** The history of one graph, set aside while another is edited (inside a pedal). */
export type HistoryScope = History;

export interface HistoryControl {
  canUndo: boolean;
  canRedo: boolean;
  /** The state to go back to, or undefined if there is nothing to undo. */
  undo(): Snapshot | undefined;
  redo(): Snapshot | undefined;
  /** True if `s` is the state already recorded. */
  same(s: Snapshot): boolean;
  /** New starting point (preset loaded, entering a pedal): nothing to undo. */
  reset(s: Snapshot): void;
  /** Starts an empty history for another graph; returns the current one, to give back to `restore`. */
  detach(initial: Snapshot): HistoryScope;
  restore(scope: HistoryScope): void;
}

/** Snapshot-based history: records a snapshot when the graph stays still for a moment. */
export function useHistory(nodes: PNode[], edges: Edge[], graphName: string): HistoryControl {
  const history = useRef(new History());
  /** Changes when the history changes: refreshes the Undo / Redo buttons. */
  const [, setVersion] = useState(0);
  const bump = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    if (nodes.some((n) => n.dragging)) return;
    const h = window.setTimeout(() => {
      if (history.current.record({ nodes, edges, name: graphName })) bump();
    }, 350);
    return () => window.clearTimeout(h);
  }, [nodes, edges, graphName, bump]);

  const undo = useCallback(() => {
    const s = history.current.undo();
    if (s) bump();
    return s;
  }, [bump]);
  const redo = useCallback(() => {
    const s = history.current.redo();
    if (s) bump();
    return s;
  }, [bump]);
  const same = useCallback((s: Snapshot) => history.current.same(s), []);
  const reset = useCallback(
    (s: Snapshot) => {
      history.current.reset(s);
      bump();
    },
    [bump],
  );
  const detach = useCallback(
    (initial: Snapshot) => {
      const scope = history.current;
      history.current = new History();
      history.current.reset(initial);
      bump();
      return scope;
    },
    [bump],
  );
  const restore = useCallback(
    (scope: HistoryScope) => {
      history.current = scope;
      bump();
    },
    [bump],
  );

  return { canUndo: history.current.canUndo, canRedo: history.current.canRedo, undo, redo, same, reset, detach, restore };
}
