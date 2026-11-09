// Frequency response of the pedal being designed: `analyze_pedal` goes out (debounced, latest wins)
// whenever the draft or the preview values change, and the `response` that comes back is returned.
import { useEffect, useMemo, useState } from "react";
import type { PedalFile } from "../generated/protocol";
import type { useServer } from "./useServer";

export interface Response {
  freq: number[];
  db: number[];
}

const DEBOUNCE_MS = 80;

/** `enabled` false (offline or mock, or an invalid draft) sends nothing and keeps the last response. */
export function useResponse(server: ReturnType<typeof useServer>, draft: PedalFile, values: Record<string, number>, enabled: boolean): Response | null {
  const { send, handlers } = server;
  const [response, setResponse] = useState<Response | null>(null);
  // the name, colour and description do not change the sound
  const key = useMemo(() => JSON.stringify([draft.node, draft.edge, draft.control, values]), [draft.node, draft.edge, draft.control, values]);

  useEffect(() => {
    handlers.current.response = (freq, db) => setResponse({ freq, db });
    return () => {
      handlers.current.response = () => {};
    };
  }, [handlers]);

  useEffect(() => {
    if (!enabled) return;
    // the timer is the latest-wins: a change within 80 ms replaces the request that has not been sent
    const h = window.setTimeout(() => send({ type: "analyze_pedal", pedal: { ...draft, name: draft.name || "draft" }, values }), DEBOUNCE_MS);
    return () => window.clearTimeout(h);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, send]);

  return response;
}
