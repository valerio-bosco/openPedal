// Contract test: every JSON fixture (the wire format reference, written by the Rust types)
// must satisfy the generated type guards. Run with `npm run test:protocol` (Node >= 24 runs
// TypeScript directly).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { PROTOCOL_VERSION, isCatalogItem, isClientMsg, isServerMsg } from "../src/generated/protocol.ts";

const fixture = (name: string): unknown => JSON.parse(readFileSync(new URL(`../src/fixtures/${name}.json`, import.meta.url), "utf8"));

const clientMessages = fixture("client-messages") as unknown[];
assert.ok(Array.isArray(clientMessages) && clientMessages.length > 0);
for (const m of clientMessages) assert.ok(isClientMsg(m), `client message rejected: ${JSON.stringify(m).slice(0, 120)}`);

const catalog = fixture("catalog") as unknown[];
assert.ok(Array.isArray(catalog) && catalog.length > 0);
for (const c of catalog) assert.ok(isCatalogItem(c), `catalog item rejected: ${JSON.stringify(c).slice(0, 120)}`);

for (const name of ["state", "meters", "error", "param", "response", "added"]) {
  const m = fixture(name);
  assert.ok(isServerMsg(m), `${name}.json rejected`);
  assert.equal((m as { type: string }).type, name);
}
for (const name of ["tonelab_caps", "tonelab_caps_unavailable", "tonelab_jobs", "tonelab_progress", "tonelab_progress_fit", "tonelab_job", "tonelab_fit_result"]) {
  const m = fixture(name);
  assert.ok(isServerMsg(m), `${name}.json rejected`);
  assert.ok((m as { type: string }).type.startsWith("tonelab_") && name.startsWith((m as { type: string }).type), `${name}.json has the wrong type`);
}
assert.ok(!isServerMsg({ type: "tonelab_progress", job: "a", stage: "bogus", message: "", fraction: null }));
assert.ok(!isClientMsg({ type: "tonelab_pick", job: "a", start: 0, end: 1, channel: "centre", label: "" }));
assert.equal((fixture("state") as { protocol: number }).protocol, PROTOCOL_VERSION);

// The guards must also reject malformed messages.
assert.ok(!isServerMsg({ type: "meters", in_db: 0 }));
assert.ok(!isServerMsg({ type: "nope" }));
assert.ok(!isClientMsg({ type: "set_param", node: "a", param: "b", value: "1" }));
assert.ok(!isClientMsg({ type: "set_graph", graph: { name: "x" } }));

console.log(`protocol fixtures OK (${clientMessages.length} client messages, ${catalog.length} catalog items)`);
