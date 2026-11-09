// Local undo / redo, used ONLY while editing inside a composite pedal.
//
// The main graph's history lives in the core (protocol v2: `undo` / `redo`, `can_undo` / `can_redo`),
// shared with every client. The core has no notion of "editing a pedal's inner graph": that graph
// is only sent when the pedal is saved (`save_pedal`), so its undo stack stays on the client.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Edge } from "@xyflow/react";
import { History, type Snapshot } from "../history";
import type { PNode } from "../model";

export interface HistoryControl {
  canUndo: boolean;
  canRedo: boolean;
  /** The state to go back to, or undefined if there is nothing to undo. */
  undo(): Snapshot | undefined;
  redo(): Snapshot | undefined;
  /** New starting point (entering a pedal): nothing to undo. */
  reset(s: Snapshot): void;
}

/** Snapshot-based history of the pedal being edited: records when the graph stays still for a moment. */
export function useHistory(nodes: PNode[], edges: Edge[], active: boolean): HistoryControl {
  const history = useRef(new History());
  /** Changes when the history changes: refreshes the Undo / Redo buttons. */
  const [, setVersion] = useState(0);
  const bump = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    if (!active || nodes.some((n) => n.dragging)) return;
    const h = window.setTimeout(() => {
      if (history.current.record({ nodes, edges })) bump();
    }, 350);
    return () => window.clearTimeout(h);
  }, [active, nodes, edges, bump]);

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
  const reset = useCallback(
    (s: Snapshot) => {
      history.current.reset(s);
      bump();
    },
    [bump],
  );

  return { canUndo: history.current.canUndo, canRedo: history.current.canRedo, undo, redo, reset };
}
