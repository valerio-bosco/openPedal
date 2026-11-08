// Connessione al motore. C1a: solo modalità mock (il WebSocket reale arriva con C1b).
import { MockServer } from "./mock";
import type { ClientMsg, ServerMsg } from "./protocol";

export type Status = "connecting" | "online" | "mock" | "offline";

export class Connection {
  private mock: MockServer;

  constructor(
    onMsg: (m: ServerMsg) => void,
    private onStatus: (s: Status) => void,
  ) {
    this.mock = new MockServer(onMsg);
  }

  start() {
    this.mock.start();
    this.onStatus("mock");
    this.mock.handle({ type: "hello" });
  }

  stop() {
    this.mock.stop();
  }

  send(msg: ClientMsg) {
    this.mock.handle(msg);
  }

  sendParam(node: string, param: string, value: number) {
    this.send({ type: "set_param", node, param, value });
  }
}
