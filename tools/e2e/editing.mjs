// UI test over the Chrome DevTools Protocol: delete, undo, redo, cables, context menu.
const PORT = process.argv[2];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const targets = await (await fetch("http://127.0.0.1:9333/json")).json();
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (e) => { const m = JSON.parse(e.data); if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => (await cdp("Runtime.evaluate", { expression: expr, returnByValue: true })).result.result.value;
const key = async (k, code, mods = 0) => {
  await cdp("Input.dispatchKeyEvent", { type: "keyDown", key: k, code, modifiers: mods, windowsVirtualKeyCode: k.toUpperCase().charCodeAt(0) });
  await cdp("Input.dispatchKeyEvent", { type: "keyUp", key: k, code, modifiers: mods });
};
const CTRL = 2, SHIFT = 8;
const count = () => ev("[document.querySelectorAll('.react-flow__node').length, document.querySelectorAll('.react-flow__edge').length]");
// graph as seen by the server (MockEngine)
const serverGraph = () => new Promise((resolve) => {
  const s = new WebSocket(`ws://127.0.0.1:${PORT}/ws`, { headers: { Origin: `http://127.0.0.1:${PORT}` } });
  s.onopen = () => s.send(JSON.stringify({ type: "hello" }));
  s.onmessage = (e) => { const m = JSON.parse(e.data); if (m.type === "state") { s.close(); resolve([m.graph.node.length, m.graph.edge.length]); } };
});
let fail = 0;
const check = (cond, msg, extra = "") => { console.log((cond ? "ok   " : "FAIL ") + msg, extra); if (!cond) fail++; };

await sleep(2500);
const start = await count();
check(start[0] >= 4, "graph loaded", JSON.stringify(start));
const names = await ev("[...document.querySelectorAll('.react-flow__node')].map(n=>n.dataset.id)");
console.log("     nodes:", names.join(", "));

// 1. × on the pedal
const victim = names.find((n) => n !== "in" && n !== "out");
await ev(`document.querySelector('.react-flow__node[data-id="${victim}"] .pedal-remove').click()`);
await sleep(700);
let c = await count();
check(c[0] === start[0] - 1, `× deletes pedal '${victim}'`, JSON.stringify(c));
check(JSON.stringify(await serverGraph()) === JSON.stringify(c), "the engine received the graph without the pedal", JSON.stringify(await serverGraph()));

// 2. undo / redo from the keyboard
await ev("document.body.focus()");
await key("z", "KeyZ", CTRL); await sleep(700);
c = await count();
check(c[0] === start[0] && c[1] === start[1], "Ctrl+Z restores pedal and cables", JSON.stringify(c));
check(JSON.stringify(await serverGraph()) === JSON.stringify(start), "the engine received the restored graph");
await key("z", "KeyZ", CTRL | SHIFT); await sleep(700);
check((await count())[0] === start[0] - 1, "Ctrl+Shift+Z redoes the deletion");
await key("y", "KeyY", CTRL); await sleep(300);
await key("z", "KeyZ", CTRL); await sleep(700);
check((await count())[0] === start[0], "Ctrl+Z again");

// 3. cable: double-click deletes it, Undo in the top bar restores it
const before = await count();
await ev(`(()=>{const e=document.querySelector('.react-flow__edge');e.dispatchEvent(new MouseEvent('dblclick',{bubbles:true}));})()`);
await sleep(700);
check((await count())[1] === before[1] - 1, "double-click deletes a cable");
await ev(`[...document.querySelectorAll('button')].find(b=>b.title.startsWith('Undo')).click()`);
await sleep(700);
check((await count())[1] === before[1], "the Undo button restores the cable");

// 4. right-click menu on the pedal
await ev(`(()=>{const n=document.querySelector('.react-flow__node[data-id="${victim}"]');const r=n.getBoundingClientRect();n.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:r.x+20,clientY:r.y+20}));})()`);
await sleep(300);
const items = await ev("[...document.querySelectorAll('.ctx-menu button')].map(b=>b.querySelector('span').textContent+(b.disabled?' (disattivo)':''))");
check(items && items.includes("Delete pedal"), "pedal context menu", JSON.stringify(items));
await ev(`[...document.querySelectorAll('.ctx-menu button')].find(b=>b.textContent.includes('Duplicate')).click()`);
await sleep(700);
check((await count())[0] === start[0] + 1, "Duplicate from the menu");
await ev(`(()=>{const n=document.querySelector('.react-flow__node[data-id="${victim}"]');n.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:50,clientY:50}));})()`);
await sleep(300);
await ev(`[...document.querySelectorAll('.ctx-menu button')].find(b=>b.textContent.includes('Disconnect')).click()`);
await sleep(700);
const edgesOfVictim = await ev(`[...document.querySelectorAll('.react-flow__edge')].filter(e=>(e.getAttribute('data-testid')||'').includes('${victim}')).length`);
check(edgesOfVictim === 0, "Disconnect all cables");

// 5. Guitar and Amp cannot be deleted
await ev(`(()=>{const n=document.querySelector('.react-flow__node[data-id="in"]');n.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:50,clientY:50}));})()`);
await sleep(300);
const fixed = await ev("[...document.querySelectorAll('.ctx-menu button')].find(b=>b.textContent.includes('Delete'))?.disabled");
check(fixed === true, "Delete is disabled for the Guitar node");
await key("Escape", "Escape");

// 6. undo everything back to the start
for (let i = 0; i < 10; i++) { await key("z", "KeyZ", CTRL); await sleep(150); }
await sleep(600);
c = await count();
check(c[0] === start[0] && c[1] === start[1], "undoing everything returns to the initial graph", JSON.stringify(c));
const undoDisabled = await ev("[...document.querySelectorAll('button')].find(b=>b.title.startsWith('Undo')).disabled");
check(undoDisabled === true, "Undo is disabled at the start of the history");
await cdp("Page.captureScreenshot", { format: "png" }).then((r) => import("node:fs").then((fs) => fs.writeFileSync(process.env.SCREENSHOT || "/tmp/openpedal-ui.png", Buffer.from(r.result.data, "base64"))));
console.log(fail ? `${fail} checks failed` : "all checks passed");
process.exit(fail ? 1 : 0);
