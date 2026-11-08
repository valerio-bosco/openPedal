// Pure layout helpers for the canvas.
import type { Edge } from "@xyflow/react";
import type { PNode } from "../model";

const GAP_X = 70;
const GAP_Y = 40;

/** Columns in signal order, using the pedals' real measurements: the new position of every node. */
export function tidyPositions(ns: PNode[], es: Edge[]): Map<string, { x: number; y: number }> {
  const depth = new Map(ns.map((n) => [n.id, 0]));
  // the graph has no cycles: propagating the depth for n passes is enough
  for (let i = 0; i < ns.length; i++) {
    for (const e of es) depth.set(e.target, Math.max(depth.get(e.target) ?? 0, (depth.get(e.source) ?? 0) + 1));
  }
  const cols: PNode[][] = [];
  for (const n of ns) (cols[depth.get(n.id) ?? 0] ??= []).push(n);
  const pos = new Map<string, { x: number; y: number }>();
  let x = 0;
  for (const col of cols) {
    if (!col) continue;
    col.sort((a, b) => a.position.y - b.position.y);
    const h = col.reduce((t, n) => t + (n.measured?.height ?? 120), 0) + GAP_Y * (col.length - 1);
    let y = -h / 2;
    for (const n of col) {
      pos.set(n.id, { x, y });
      y += (n.measured?.height ?? 120) + GAP_Y;
    }
    x += Math.max(...col.map((n) => n.measured?.width ?? 240)) + GAP_X;
  }
  return pos;
}

/** True if any two pedals overlap on screen. */
export function hasOverlap(ns: PNode[]): boolean {
  const box = (n: PNode) => ({ x: n.position.x, y: n.position.y, w: n.measured?.width ?? 0, h: n.measured?.height ?? 0 });
  return ns.some((a, i) =>
    ns.slice(i + 1).some((b) => {
      const p = box(a);
      const q = box(b);
      return p.x < q.x + q.w && q.x < p.x + p.w && p.y < q.y + q.h && q.y < p.y + p.h;
    }),
  );
}
