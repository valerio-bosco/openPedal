// Connection to the engine: server state (catalog, presets, audio, meters, pedal contents),
// status, error reporting and the mock fallback (see ws.ts).
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  PROTOCOL_VERSION,
  type AudioInfo,
  type CatalogItem,
  type ClientMsg,
  type MetersMsg,
  type PedalFile,
  type ServerMsg,
  type StateMsg,
} from "../generated/protocol";
import { fixturePedals } from "../mock";
import { IMPLICIT_ITEMS } from "../model";
import { Connection, type Status } from "../ws";

/** Reactions that depend on the editor, installed by the hooks that own that state. */
export interface ServerHandlers {
  /** A `state` for the main graph arrived (not one that completes a pending pedal save). */
  graph: (m: StateMsg) => void;
  /** The engine reported an error for a node. */
  nodeError: (node: string, message: string) => void;
}

export function useServer(toast: (text: string) => void) {
  const [catalogItems, setCatalogItems] = useState<CatalogItem[]>([]);
  const [presets, setPresets] = useState<string[]>([]);
  const [audio, setAudio] = useState<AudioInfo>();
  const [meters, setMeters] = useState<MetersMsg>();
  const [status, setStatus] = useState<Status>("connecting");
  const conn = useRef<Connection>(undefined);
  const handlers = useRef<ServerHandlers>({ graph: () => {}, nodeError: () => {} });
  /** Content of the composite pedals (id -> pedal). */
  const pedals = useRef<Record<string, PedalFile>>({});
  /** After `save_pedal`: action to run when the `state` with the pedal in the catalog arrives. */
  const pending = useRef<{ kind: string; run: () => void }>(undefined);

  const catalog = useMemo(
    () => new Map([...catalogItems, ...IMPLICIT_ITEMS].map((c) => [c.kind, c])),
    [catalogItems],
  );

  const onMsg = useCallback(
    (m: ServerMsg) => {
      if (m.type === "meters") return setMeters(m);
      if (m.type === "error") {
        toast(m.message);
        if (m.node) handlers.current.nodeError(m.node, m.message);
        return;
      }
      if (m.protocol > PROTOCOL_VERSION) {
        toast(`This server speaks protocol ${m.protocol}, newer than this interface (${PROTOCOL_VERSION}). Update the interface.`);
        return;
      }
      setCatalogItems(m.catalog);
      setPresets(m.presets);
      setAudio(m.audio);
      pedals.current = { ...pedals.current, ...m.pedals };
      const pend = pending.current;
      if (pend && m.catalog.some((c) => c.kind === pend.kind)) {
        pending.current = undefined;
        pend.run();
        return;
      }
      handlers.current.graph(m);
    },
    [toast],
  );

  useEffect(() => {
    const c = new Connection(onMsg, setStatus);
    conn.current = c;
    c.start();
    return () => c.stop();
  }, [onMsg]);

  // in demo mode the example pedal contents come from the fixtures
  useEffect(() => {
    if (status === "mock") pedals.current = { ...fixturePedals(), ...pedals.current };
  }, [status]);

  const send = useCallback((msg: ClientMsg) => conn.current?.send(msg), []);
  const sendParam = useCallback((node: string, param: string, value: number) => conn.current?.sendParam(node, param, value), []);
  /** Runs `run` when the catalog first contains `kind` (a pedal just saved). */
  const whenInCatalog = useCallback((kind: string, run: () => void) => {
    pending.current = { kind, run };
  }, []);

  return { status, catalogItems, catalog, presets, audio, meters, pedals, handlers, send, sendParam, whenInCatalog };
}
