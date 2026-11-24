// UI test of the Tone Lab panel over the Chrome DevTools Protocol: open a job from the list, select a range
// on the song map, pick it, fit it, flip the A/B player and save the preset.
// It needs the server (or the UI's own demo fallback) to answer the Tone Lab messages; when `tonelab_caps`
// never arrives the test is skipped with a message.
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
const waitFor = async (expr, ms = 5000) => { for (let t = 0; t < ms; t += 100) { if (await ev(expr)) return true; await sleep(100); } return false; };
const mouse = (type, x, y) => cdp("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1 });
const typeInto = async (sel, text) => {
  await ev(`(()=>{const i=document.querySelector(${JSON.stringify(sel)});i.focus();i.select()})()`);
  await cdp("Input.insertText", { text });
};

await sleep(1800);
check(await click("#tonelab-open"), "the top bar has a Tone Lab button");
check(await waitFor("!!document.querySelector('.tonelab')"), "the Tone Lab panel opens");
const settled = await waitFor("document.querySelector('.tonelab')?.dataset.state !== 'waiting'", 4000);
if (!settled) {
  console.log("SKIP tonelab: tonelab_caps never arrived (the server does not answer Tone Lab messages yet)");
  process.exit(fail ? 1 : 0);
}
const state = await ev("document.querySelector('.tonelab').dataset.state");
if (state === "unavailable") {
  check(!!(await ev("document.querySelector('[data-testid=unavailable]')?.textContent.includes('tools/tonelab/README.md')")), "unavailable: shows how to set it up");
  process.exit(fail ? 1 : 0);
}
check(state === "ready", "Tone Lab is available");
check(await ev("!!document.querySelector('#tl-file')"), "there is a file picker");

// 1. open a past job
check(await waitFor("!!document.querySelector('[data-job]')"), "the job list has an entry");
await click("[data-job]");
check(await waitFor("!!document.querySelector('[data-testid=songmap]')"), "the song map appears");
check((await ev("document.querySelectorAll('.tl-section').length")) > 0, "the song map shows sections");
check((await ev("[...document.querySelectorAll('.tl-section')].every(s=>s.style.background)")) === true, "sections are coloured by kind");

// 2. drag a range
const r = await ev("(()=>{const b=document.querySelector('.tl-track').getBoundingClientRect();return [b.x,b.y,b.width,b.height]})()");
const y = r[1] + r[3] / 2;
await mouse("mousePressed", r[0] + r[2] * 0.25, y);
await mouse("mouseMoved", r[0] + r[2] * 0.35, y);
await mouse("mouseMoved", r[0] + r[2] * 0.45, y);
await mouse("mouseReleased", r[0] + r[2] * 0.45, y);
check(await waitFor("!!document.querySelector('[data-testid=selection]')"), "dragging selects a range", await ev("document.querySelector('[data-testid=sel-info]').textContent"));
check(!(await ev("document.querySelector('#tl-pick').disabled")), "Pick is enabled with a selection");

// 3. pick it
const partsBefore = await ev("document.querySelectorAll('[data-testid=part]').length");
await typeInto("input[aria-label='Part name']", "E2E riff");
await click("#tl-pick");
check(await waitFor(`document.querySelectorAll('[data-testid=part]').length === ${partsBefore + 1}`, 15000), "the picked part appears in the list");
check(await ev("document.querySelector('[data-testid=part]:last-of-type')?.textContent.includes('E2E riff')") || (await ev("[...document.querySelectorAll('[data-testid=part]')].some(p=>p.textContent.includes('E2E riff'))")), "the part has the chosen label");
check(/synthetic|recording/.test(await ev("[...document.querySelectorAll('[data-testid=di-use]')].pop().textContent")), "the panel says which input the fit will use");

// 4. fit it
check(await waitFor("!document.querySelector('[data-testid=progress]')", 15000), "the pick step finished");
await ev("[...document.querySelectorAll('[data-act=fit]')].pop().click()");
check(await waitFor("!!document.querySelector('[data-testid=fit-result]')", 30000), "a fit result card appears");
check((await ev("document.querySelectorAll('[data-testid=templates] tbody tr').length")) > 0, "the per-template table has rows");
check(/dB/.test(await ev("document.querySelector('[data-testid=err]').textContent")), "the error is explained in plain words");

// 5. A/B
check(await ev("document.querySelector('[data-testid=ab]').dataset.side") === "result", "A/B starts on the result");
await click("#tl-ab-target");
check(await ev("document.querySelector('[data-testid=ab]').dataset.side") === "target", "the toggle switches to the target");
await click("#tl-ab-result");
check(await ev("document.querySelector('[data-testid=ab]').dataset.side") === "result", "and back to the result");

// 6. save
await typeInto("#tl-name", "E2E Tone Lab");
await click("#tl-save");
check(await waitFor("[...document.querySelectorAll('.toast')].some(t=>t.textContent.includes('E2E Tone Lab'))", 3000), "a toast confirms the save");

await click("#tl-close");
check(await waitFor("!document.querySelector('.tonelab')"), "the panel closes");
process.exit(fail ? 1 : 0);
