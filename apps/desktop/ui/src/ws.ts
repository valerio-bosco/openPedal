// Connection to the engine: WebSocket `/ws` with reconnection. If the server does not respond
// at startup, it falls back to the fake server (mock) and keeps retrying in the background.
import { MockServer } from "./mock";
import type { ClientMsg, ServerMsg } from "./generated/protocol";

export type Status = "connecting" | "online" | "mock" | "offline";

/** Wait before falling back to the mock if the server does not respond. */
const FALLBACK_MS = 1200;
/** Minimum interval between two `set_param` messages for the same knob (about 60 per second). */
const PARAM_INTERVAL_MS = 16;

interface Pending {
  last: number;
  value?: number;
  timer?: number;
}

export class Connection {
  private ws?: WebSocket;
  private mock?: MockServer;
  private everOpen = false;
  private stopped = false;
  private retry = 1000;
  private timers: number[] = [];
  private params = new Map<string, Pending>();

  constructor(
    private onMsg: (m: ServerMsg) => void,
    private onStatus: (s: Status) => void,
  ) {}

  start() {
    this.stopped = false;
    this.onStatus("connecting");
    this.open();
    this.timers.push(
      window.setTimeout(() => {
        if (!this.everOpen && !this.stopped) this.enterMock();
      }, FALLBACK_MS),
    );
  }

  stop() {
    this.stopped = true;
    this.timers.forEach((t) => window.clearTimeout(t));
    this.timers = [];
    this.params.forEach((p) => window.clearTimeout(p.timer));
    this.params.clear();
    this.mock?.stop();
    this.mock = undefined;
    const ws = this.ws;
    this.ws = undefined;
    if (ws) {
      ws.onclose = ws.onerror = ws.onmessage = ws.onopen = null;
      ws.close();
    }
  }

  private open() {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    let ws: WebSocket;
    try {
      ws = new WebSocket(`${proto}://${location.host}/ws`);
    } catch {
      return this.scheduleRetry();
    }
    this.ws = ws;
    ws.onopen = () => {
      this.everOpen = true;
      this.retry = 1000;
      if (this.mock) {
        this.mock.stop();
        this.mock = undefined;
      }
      this.onStatus("online");
      ws.send(JSON.stringify({ type: "hello" }));
    };
    ws.onmessage = (ev) => {
      try {
        this.onMsg(JSON.parse(String(ev.data)) as ServerMsg);
      } catch {
        /* invalid message: ignored */
      }
    };
    ws.onclose = () => {
      this.ws = undefined;
      if (this.stopped) return;
      if (this.everOpen) this.onStatus("offline");
      this.scheduleRetry();
    };
    ws.onerror = () => ws.close();
  }

  private scheduleRetry() {
    if (this.stopped) return;
    this.timers.push(window.setTimeout(() => this.open(), this.retry));
    this.retry = Math.min(this.retry * 1.6, 5000);
  }

  private enterMock() {
    if (this.mock) return;
    this.mock = new MockServer(this.onMsg);
    this.mock.start();
    this.onStatus("mock");
    this.mock.handle({ type: "hello" });
  }

  send(msg: ClientMsg) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(msg));
    else this.mock?.handle(msg);
  }

  /** `set_param` limited to about 60 messages per second per knob; the last value always gets through. */
  sendParam(node: string, param: string, value: number) {
    const key = `${node}\u0000${param}`;
    const p = this.params.get(key) ?? { last: 0 };
    this.params.set(key, p);
    const emit = (v: number) => {
      p.last = performance.now();
      p.value = undefined;
      p.timer = undefined;
      this.send({ type: "set_param", node, param, value: v });
    };
    const wait = PARAM_INTERVAL_MS - (performance.now() - p.last);
    if (wait <= 0 && p.timer === undefined) return emit(value);
    p.value = value;
    if (p.timer === undefined) p.timer = window.setTimeout(() => emit(p.value ?? value), Math.max(wait, 0));
  }
}
