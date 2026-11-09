// UI test of the pedal designer over the Chrome DevTools Protocol: create a pedal from two selected nodes
// with a knob control and two routes (one log), save it, and open Grit Box (5 controls, response graph).
const PORT = process.argv[2];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const page = (await (await fetch("http://127.0.0.1:9333/json")).json()).find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pend = new Map();
ws.onmessage = (e) => { const m = JSON.parse(e.data); pend.get(m.id)?.(m); };
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (x) => (await cdp("Runtime.evaluate", { expression: x, returnByValue: true })).result.result.value;
let fail = 0;
const check = (c, m, x = "") => { console.log((c ? "ok   " : "FAIL ") + m, x); if (!c) fail++; };
const click = (sel) => ev(`(()=>{const e=document.querySelector(${JSON.stringify(sel)});if(!e)return false;e.click();return true})()`);
const clickText = (sel, text) => ev(`(()=>{const e=[...document.querySelectorAll(${JSON.stringify(sel)})].find(b=>b.textContent.trim()===${JSON.stringify(text)});if(!e)return false;e.click();return true})()`);
const waitFor = async (expr, ms = 4000) => { for (let t = 0; t < ms; t += 100) { if (await ev(expr)) return true; await sleep(100); } return false; };
const serverState = () => new Promise((res) => {
  const s = new WebSocket(`ws://127.0.0.1:${PORT}/ws`, { headers: { Origin: `http://127.0.0.1:${PORT}` } });
  s.onopen = () => s.send('{"type":"hello"}');
  s.onmessage = (e) => { const m = JSON.parse(e.data); if (m.type === "state") { s.close(); res(m); } };
});
const mouse = async (type, x, y, modifiers = 0) => cdp("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: 1, clickCount: 1, modifiers });
const clickAt = async (x, y, modifiers = 0) => { await mouse("mousePressed", x, y, modifiers); await mouse("mouseReleased", x, y, modifiers); };
const pathD = () => ev(`document.querySelector('.dz-graph path[stroke]')?.getAttribute('d') ?? ''`);

await sleep(1500);
// a known graph (the "clean" preset), loaded through another client: the UI follows
await new Promise((res) => {
  const s = new WebSocket(`ws://127.0.0.1:${PORT}/ws`, { headers: { Origin: `http://127.0.0.1:${PORT}` } });
  s.onopen = () => { s.send(JSON.stringify({ type: "load_preset", name: "clean" })); setTimeout(() => { s.close(); res(); }, 400); };
});
await sleep(1200);
const st = await serverState();
const catalog = new Map(st.catalog.map((c) => [c.kind, c]));
const nodes = new Map(st.graph.node.map((n) => [n.id, n]));
const nums = (n) => (catalog.get(n.type)?.params ?? []).filter((p) => p.kind === "number");
// two cabled blocks, together with a numeric parameter that can use a log curve and another one
const pair = st.graph.edge
  .map((e) => [nodes.get(e.from), nodes.get(e.to)])
  .find(([a, b]) => a && b && ![a, b].some((n) => n.type === "input" || n.type === "output") && [...nums(a), ...nums(b)].filter((p) => p.min > 0).length >= 1 && nums(a).length + nums(b).length >= 2);
check(!!pair, "the loaded graph has two cabled blocks with numeric parameters");
const [a, b] = pair;
const all = [a, b].flatMap((n) => nums(n).map((p) => ({ target: `${n.id}.${p.id}`, p })));
const logTarget = all.find((t) => t.p.min > 0).target;
const otherTarget = all.find((t) => t.target !== logTarget).target;
const before = await ev("document.querySelectorAll('.react-flow__node').length");

// 1. select the two blocks (click, shift+click) and open the designer
const center = (nodeId) => ev(`(()=>{const r=document.querySelector('.react-flow__node[data-id="${nodeId}"] .pedal-head').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()`);
const [ax, ay] = await center(a.id);
await clickAt(ax, ay);
const [bx, by] = await center(b.id);
await cdp("Input.dispatchKeyEvent", { type: "keyDown", key: "Shift", code: "ShiftLeft", modifiers: 8, windowsVirtualKeyCode: 16 });
await clickAt(bx, by, 8);
await cdp("Input.dispatchKeyEvent", { type: "keyUp", key: "Shift", code: "ShiftLeft", windowsVirtualKeyCode: 16 });
await sleep(200);
check((await ev("document.querySelectorAll('.react-flow__node.selected').length")) === 2, "two blocks selected");
check(await clickText("button", "Create pedal"), "Create pedal button");
check(await waitFor("!!document.querySelector('.designer')"), "the designer opens");
check((await ev("document.querySelectorAll('.dz-inner .n[data-node]').length")) === 2, "the inner graph shows the two selected blocks");

// 2. name, a knob control and two routes (one log)
await ev("(()=>{const i=document.querySelector('#dz-name');i.focus();i.select()})()");
await cdp("Input.insertText", { text: "E2E Box" });
check((await ev("document.querySelector('.dz-pedal h4').textContent")) === "E2E Box", "the preview shows the name");
await click("#dz-add-control");
check(await waitFor("!!document.querySelector('.dz-ved')"), "a control editor appears");
for (const t of [otherTarget, logTarget]) {
  await click("#dz-add-route");
  check(await waitFor(`!!document.querySelector('.dz-menu [data-target="${t}"]')`), `the picker lists ${t}`);
  await click(`.dz-menu [data-target="${t}"]`);
  await sleep(150);
}
check((await ev("document.querySelectorAll('.dz-route').length")) === 2, "two routes");
await ev(`document.querySelector('.dz-route[data-target="${logTarget}"] button[aria-label="Log"]').click()`);
await sleep(150);
check((await ev(`document.querySelector('.dz-route[data-target="${logTarget}"] button[aria-label="Log"]').getAttribute('aria-pressed')`)) === "true", "log curve selected on one route");
check(await waitFor("!!document.querySelector('.dz-graph path[stroke]')", 5000), "a response graph arrives for the draft");
const d1 = await pathD();
// moving the preview knob changes the response
const knob = await ev(`(()=>{const r=document.querySelector('.dz-pk i').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()`);
await mouse("mousePressed", knob[0], knob[1]);
await mouse("mouseMoved", knob[0], knob[1] - 80);
await mouse("mouseReleased", knob[0], knob[1] - 80);
await sleep(600);
check((await pathD()) !== d1, "dragging the preview knob changes the response");
check(!(await ev("document.querySelector('#dz-save').disabled")), "Save is enabled for a valid pedal");

// 3. client-side validation shows errors inline
await click("#dz-add-control");
await ev(`(()=>{const i=document.querySelector('.dz-ved input[aria-label="Control name"]');i.focus();i.select()})()`);
await cdp("Input.insertText", { text: "Knob" });
check(await waitFor("!!document.querySelector('.dz-ved .dz-err')"), "a duplicate control name is reported inline");
check(await ev("document.querySelector('#dz-save').disabled"), "Save is disabled while there is an error");
await click(".dz-ved .danger-hover");
await sleep(200);
check((await ev("document.querySelectorAll('.dz-crow').length")) === 1, "the extra control is removed");

// 4. save: the catalog has the pedal, state.pedals holds the routes, the node replaces the selection
await click("#dz-save");
check(await waitFor("!document.querySelector('.designer')"), "the designer closes after saving");
const after = await serverState();
check(after.catalog.some((c) => c.kind === "pedal:e2e-box"), "the catalog has pedal:e2e-box");
const saved = after.pedals["e2e-box"];
check(!!saved && saved.version === 2 && saved.control.length === 1 && saved.control[0].kind === "knob", "state.pedals holds the v2 pedal with its knob", JSON.stringify(saved?.control));
const routes = saved?.control[0].route ?? [];
check(routes.length === 2 && routes.some((r) => r.target === logTarget && r.map === "log") && routes.some((r) => r.target === otherTarget), "both routes are saved, one log", JSON.stringify(routes));
await sleep(500);
check((await ev("document.querySelectorAll('.react-flow__node').length")) === before - 2 + 1, "the main graph has the pedal in place of the two blocks");

// 5. Grit Box: 5 controls and a response graph
check(await click('button[aria-label="Edit Grit Box"]'), "Edit button of Grit Box in the palette");
check(await waitFor("!!document.querySelector('.designer')"), "the designer opens for Grit Box");
check((await ev("document.querySelectorAll('.dz-crow').length")) === 5, "Grit Box shows its 5 controls");
check((await ev("document.querySelectorAll('.dz-pedal [data-control]').length")) === 5, "the preview has 5 controls on its face");
check(await waitFor("!!document.querySelector('.dz-graph path[stroke]')", 5000), "a response graph arrives for Grit Box");
const g1 = await pathD();
await click('.dz-ps[data-control="boost"]');
await sleep(600);
check((await pathD()) !== g1, "the Boost switch changes the response");
check((await ev("document.querySelectorAll('.dz-route').length")) >= 1, "the selected control lists its routes");
if (process.env.DESIGNER_SHOT) await cdp("Page.captureScreenshot", { format: "png" }).then((r) => import("node:fs").then((fs) => fs.writeFileSync(process.env.DESIGNER_SHOT, Buffer.from(r.result.data, "base64"))));
await click("#dz-cancel");
check(await waitFor("!document.querySelector('.designer')"), "Cancel closes the designer");

console.log(fail ? `${fail} checks failed` : "all checks passed");
process.exit(fail ? 1 : 0);
