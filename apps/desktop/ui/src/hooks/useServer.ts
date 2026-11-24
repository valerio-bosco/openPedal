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
import { Connection, isTonelabMsg, type Status, type TonelabClientMsg, type TonelabEvent } from "../ws";

/** Reactions that depend on the editor, installed by the hooks that own that state. */
export interface ServerHandlers {
  /** A `state` for the main graph arrived (not one that completes a pending pedal save). */
  graph: (m: StateMsg, keep?: ReadonlySet<string>) => void;
  /** The engine reported an error for a node. */
  nodeError: (node: string, message: string) => void;
  /** Another client moved a knob (`param`). */
  param: (node: string, param: string, value: number) => void;
  /** The frequency response asked for with `analyze_pedal` arrived. */
  response: (freq: number[], db: number[]) => void;
}

export function useServer(toast: (text: string) => void) {
  const [catalogItems, setCatalogItems] = useState<CatalogItem[]>([]);
  const [presets, setPresets] = useState<string[]>([]);
  const [audio, setAudio] = useState<AudioInfo>();
  const [meters, setMeters] = useState<MetersMsg>();
  const [status, setStatus] = useState<Status>("connecting");
  const conn = useRef<Connection>(undefined);
  /** Tone Lab panel listeners: they get every `tonelab_*` message. */
  const tonelabSubs = useRef(new Set<(m: TonelabEvent) => void>());
  const [history, setHistory] = useState({ canUndo: false, canRedo: false, dirty: false });
  const handlers = useRef<ServerHandlers>({ graph: () => {}, nodeError: () => {}, param: () => {}, response: () => {} });
  /** Number of user interactions (knob drag, node drag) that must not be disturbed by an incoming graph. */
  const holds = useRef(0);
  /** The last graph received while an interaction was in progress: applied when it ends. */
  const deferred = useRef<StateMsg>(undefined);
  /** Parameters (`node\0param`) the user changed during the interaction: the deferred graph is older than them. */
  const touched = useRef(new Set<string>());
  /** The graph of the last `set_graph` we sent: the server does not reply to it, so we ask for the flags. */
  const sentGraph = useRef<string>(undefined);
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
      if (m.type === "param") return handlers.current.param(m.node, m.param, m.value);
      if (m.type === "added") return; // only the iPad's `add_node` asks for it
      if (m.type === "response") return handlers.current.response(m.freq, m.db);
      if (m.type === "error") {
        toast(m.message);
        tonelabSubs.current.forEach((f) => f(m));
        if (m.node) handlers.current.nodeError(m.node, m.message);
        return;
      }
      if (isTonelabMsg(m)) return tonelabSubs.current.forEach((f) => f(m));
      if (m.protocol > PROTOCOL_VERSION) {
        toast(`This server speaks protocol ${m.protocol}, newer than this interface (${PROTOCOL_VERSION}). Update the interface.`);
        return;
      }
      setCatalogItems(m.catalog);
      setPresets(m.presets);
      setAudio(m.audio);
      setHistory({ canUndo: m.can_undo, canRedo: m.can_redo, dirty: m.dirty });
      pedals.current = { ...pedals.current, ...m.pedals };
      const pend = pending.current;
      if (pend && m.catalog.some((c) => c.kind === pend.kind)) {
        pending.current = undefined;
        pend.run();
        return;
      }
      // the reply to our own `hello` after `set_graph` carries the history flags only
      if (sentGraph.current !== undefined && sentGraph.current === JSON.stringify([m.graph.node, m.graph.edge])) {
        sentGraph.current = undefined;
        return;
      }
      if (holds.current > 0) deferred.current = m;
      else handlers.current.graph(m);
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

  const send = useCallback((msg: ClientMsg) => {
    conn.current?.send(msg);
    if (msg.type === "set_graph") {
      // `set_graph` is not answered (the sender has the graph): ask for the new `can_undo` / `dirty`
      sentGraph.current = JSON.stringify([msg.graph.node, msg.graph.edge]);
      conn.current?.send({ type: "hello" });
    }
  }, []);
  const sendParam = useCallback((node: string, param: string, value: number) => {
    if (holds.current > 0) touched.current.add(`${node}\u0000${param}`);
    conn.current?.sendParam(node, param, value);
  }, []);
  /** A user interaction starts: incoming graphs wait until the matching `release`. */
  const hold = useCallback(() => {
    holds.current++;
  }, []);
  const release = useCallback(() => {
    holds.current = Math.max(0, holds.current - 1);
    const m = deferred.current;
    if (holds.current === 0 && m) {
      deferred.current = undefined;
      const keep = touched.current;
      touched.current = new Set();
      handlers.current.graph(m, keep);
    }
    if (holds.current === 0) touched.current = new Set();
  }, []);
  /** Runs `run` when the catalog first contains `kind` (a pedal just saved). */
  const whenInCatalog = useCallback((kind: string, run: () => void) => {
    pending.current = { kind, run };
  }, []);

  /** Tone Lab transport: `send` returns false when there is no server connection. */
  const tonelab = useMemo(
    () => ({
      send: (m: TonelabClientMsg) => conn.current?.sendTonelab(m) ?? false,
      subscribe: (f: (m: TonelabEvent) => void) => {
        tonelabSubs.current.add(f);
        return () => void tonelabSubs.current.delete(f);
      },
    }),
    [],
  );

  return { status, tonelab, ...history, hold, release, catalogItems, catalog, presets, audio, meters, pedals, handlers, send, sendParam, whenInCatalog };
}
