// Frequency response: log frequency axis 20 Hz - 20 kHz, dB grid, filled area in the pedal colour.
import type { Response } from "../../hooks/useResponse";

const W = 560;
const H = 240;
const L = 38;
const R = 8;
const T = 10;
const B = 22;
const FREQS = [50, 100, 200, 500, 1000, 2000, 5000, 10000];

const x = (f: number) => L + (Math.log10(Math.max(20, f) / 20) / 3) * (W - L - R);

export function ResponseGraph({ response, color, message }: { response: Response | null; color: string; message?: string }) {
  if (message || !response || response.freq.length === 0) {
    return <div className="dz-graph-empty">{message ?? "Waiting for the response…"}</div>;
  }
  const maxDb = Math.max(...response.db);
  const minDb = Math.min(...response.db);
  const top = Math.max(12, Math.ceil((maxDb + 1) / 12) * 12);
  const bottom = Math.min(top - 48, Math.floor((minDb - 1) / 12) * 12);
  const y = (d: number) => T + ((top - Math.max(bottom, Math.min(top, d))) / (top - bottom)) * (H - T - B);
  const grid: number[] = [];
  for (let d = top; d >= bottom; d -= 12) grid.push(d);
  const line = response.freq.map((f, i) => `${i ? "L" : "M"}${x(f).toFixed(1)} ${y(response.db[i]).toFixed(1)}`).join("");
  const area = `${line}L${x(response.freq[response.freq.length - 1]).toFixed(1)} ${H - B}L${x(response.freq[0]).toFixed(1)} ${H - B}Z`;
  return (
    <svg className="dz-graph" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Frequency response">
      <defs>
        <linearGradient id="dz-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={color} stopOpacity=".4" />
          <stop offset="1" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      {FREQS.map((f) => (
        <g key={f}>
          <line x1={x(f)} x2={x(f)} y1={T} y2={H - B} className="grid" />
          <text x={x(f)} y={H - 7} textAnchor="middle">
            {f >= 1000 ? `${f / 1000}k` : f}
          </text>
        </g>
      ))}
      {grid.map((d) => (
        <g key={d}>
          <line x1={L} x2={W - R} y1={y(d)} y2={y(d)} className={d === 0 ? "grid zero" : "grid"} />
          <text x={L - 6} y={y(d) + 3} textAnchor="end">
            {d > 0 ? `+${d}` : d}
          </text>
        </g>
      ))}
      <path d={area} fill="url(#dz-fill)" />
      <path d={line} fill="none" stroke={color} strokeWidth="2.2" strokeLinejoin="round" />
    </svg>
  );
}
