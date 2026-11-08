import fs from "node:fs";
const [PORT, DIR] = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// 1. a preset to delete, saved through the server
await new Promise((res) => { const s = new WebSocket(`ws://127.0.0.1:${PORT}/ws`, { headers: { Origin: `http://127.0.0.1:${PORT}` } });
  s.onopen = () => s.send(JSON.stringify({ type: "save_preset", name: "da-cancellare", graph: { name: "da-cancellare", version: 2, node: [{ id: "in", type: "input", pos: [0, 0], params: {} }, { id: "out", type: "output", pos: [300, 0], params: {} }], edge: [{ from: "in", to: "out" }] } }));
  s.onmessage = (e) => { if (JSON.parse(e.data).type === "state") { s.close(); res(); } }; });
let fail = 0; const check = (c, m, x = "") => { console.log((c ? "ok   " : "FAIL ") + m, x); if (!c) fail++; };
check(fs.existsSync(`${DIR}/content/presets/da-cancellare.toml`), "test preset saved");
const page = (await (await fetch("http://127.0.0.1:9333/json")).json()).find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl); await new Promise((r) => (ws.onopen = r));
let id = 0; const pend = new Map(); ws.onmessage = (e) => { const m = JSON.parse(e.data); pend.get(m.id)?.(m); };
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (x) => (await cdp("Runtime.evaluate", { expression: x, returnByValue: true })).result.result.value;
await cdp("Page.reload"); await sleep(2500);
const trash = `[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')==='Delete preset')`;
check(await ev(`${trash}.disabled`) === false, "the trash button is enabled on the loaded preset (clean)");
// load the test preset from the menu
await ev(`(()=>{const s=document.querySelector('.preset select');s.value='da-cancellare';s.dispatchEvent(new Event('change',{bubbles:true}));})()`);
await sleep(800);
check(await ev(`document.querySelector('.preset select').value`) === "da-cancellare", "test preset loaded");
// "Cancel" in the confirmation: nothing is deleted
await ev(`window.confirm=()=>false; ${trash}.click()`); await sleep(500);
check(fs.existsSync(`${DIR}/content/presets/da-cancellare.toml`), "answering No keeps the file");
await ev(`window.confirm=()=>true; ${trash}.click()`); await sleep(800);
check(!fs.existsSync(`${DIR}/content/presets/da-cancellare.toml`), "answering Yes deletes the file");
const opts = await ev(`[...document.querySelectorAll('.preset select option')].map(o=>o.value).filter(Boolean)`);
check(!opts.includes("da-cancellare") && opts.includes("clean"), "the menu no longer lists it", JSON.stringify(opts));
check(await ev(`${trash}.disabled`) === true, "the trash button is disabled (the graph is no longer a saved preset)");
check(await ev(`document.querySelectorAll('.react-flow__node').length`) === 2, "the current sound stays in the editor");
console.log(fail ? "checks failed" : "all checks passed"); process.exit(fail ? 1 : 0);
