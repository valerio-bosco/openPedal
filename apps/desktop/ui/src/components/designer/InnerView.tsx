// Read-only mini view of the pedal's inner graph. Blocks driven by the selected control are highlighted.
import type { CatalogItem, PedalFile } from "../../generated/protocol";
import { PEDAL_IN, PEDAL_OUT } from "../../generated/protocol";
import { CATEGORY_COLOR } from "../../model";

const BASE_W = 300;
const BASE_H = 280;
const BW = 86;
const BH = 38;
const IO = 30;

const clip = (s: string, n: number) => (s.length > n ? s.slice(0, n - 1) + "…" : s);

export function InnerView({ draft, catalog, highlight }: { draft: PedalFile; catalog: Map<string, CatalogItem>; highlight: Set<string> }) {
  const xs = draft.node.map((n) => n.pos[0]);
  const ys = draft.node.map((n) => n.pos[1]);
  const minX = xs.length ? Math.min(...xs) : 0;
  const maxX = xs.length ? Math.max(...xs) : 0;
  const minY = ys.length ? Math.min(...ys) : 0;
  const maxY = ys.length ? Math.max(...ys) : 0;
  // blocks keep their relative places, spread enough not to overlap; the view grows if it must
  const gap = (v: number[]) => {
    const u = [...new Set(v)].sort((p, q) => p - q);
    return u.length > 1 ? Math.min(...u.slice(1).map((x, i) => x - u[i])) : Infinity;
  };
  const sx = maxX > minX ? Math.max((BASE_W - 2 * (IO + 18) - BW) / (maxX - minX), (BW + 16) / gap(xs)) : 0;
  const sy = maxY > minY ? Math.max((BASE_H - BH - 24) / (maxY - minY), (BH + 12) / gap(ys)) : 0;
  const W = Math.max(BASE_W, 2 * (IO + 18) + BW + (maxX - minX) * sx);
  const H = Math.max(BASE_H, BH + 24 + (maxY - minY) * sy);
  const innerW = W - 2 * (IO + 18) - BW;
  const innerH = H - BH - 24;
  const pos = new Map<string, { x: number; y: number }>();
  for (const n of draft.node) {
    pos.set(n.id, {
      x: IO + 18 + (maxX > minX ? (n.pos[0] - minX) * sx : innerW / 2),
      y: 12 + (maxY > minY ? (n.pos[1] - minY) * sy : innerH / 2),
    });
  }
  const midY = H / 2 - BH / 2;
  pos.set(PEDAL_IN, { x: 6, y: midY });
  pos.set(PEDAL_OUT, { x: W - IO - 6, y: midY });
  const width = (id: string) => (id === PEDAL_IN || id === PEDAL_OUT ? IO : BW);
  return (
    <svg className="dz-inner" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Inner graph">
      {draft.edge.map((e, i) => {
        const a = pos.get(e.from);
        const b = pos.get(e.to);
        if (!a || !b) return null;
        const x1 = a.x + width(e.from);
        const y1 = a.y + BH / 2;
        const x2 = b.x;
        const y2 = b.y + BH / 2;
        const dx = Math.max(20, Math.abs(x2 - x1) / 2);
        return <path key={i} d={`M${x1} ${y1} C${x1 + dx} ${y1} ${x2 - dx} ${y2} ${x2} ${y2}`} className="wire" />;
      })}
      {[PEDAL_IN, PEDAL_OUT].map((id) => (
        <g key={id} className="n io">
          <rect x={pos.get(id)!.x} y={pos.get(id)!.y} width={IO} height={BH} rx={8} />
          <text x={pos.get(id)!.x + IO / 2} y={pos.get(id)!.y + BH / 2 + 4} textAnchor="middle">
            {id === PEDAL_IN ? "In" : "Out"}
          </text>
        </g>
      ))}
      {draft.node.map((n) => {
        const p = pos.get(n.id)!;
        const item = catalog.get(n.type);
        const on = highlight.has(n.id);
        return (
          <g key={n.id} className={`n${on ? " sel" : ""}`} data-node={n.id}>
            <rect x={p.x} y={p.y} width={BW} height={BH} rx={8} style={on ? undefined : { stroke: CATEGORY_COLOR[item?.category ?? "utility"] }} />
            <text x={p.x + 8} y={p.y + 16}>
              {clip(item?.label ?? n.type, 11)}
            </text>
            <text x={p.x + 8} y={p.y + 30} className="k">
              {clip(n.id, 13)}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
