
export const KIND_COLOR: Record<string, string> = {
  centre: "#4cc38a",
  double: "#ffb23f",
  wide: "#5aa9ff",
  none: "#3a3e4a",
};

export const KIND_TEXT: Record<string, string> = {
  centre: "Centre: one guitar in the middle (easiest to copy)",
  double: "Double: the same part on both sides",
  wide: "Wide: spread across the stereo field",
  none: "No guitar",
};

export function clock(s: number): string {
  const m = Math.floor(s / 60);
  const r = Math.floor(s % 60);
  return `${m}:${String(r).padStart(2, "0")}`;
}

export const db = (x: number) => `${x.toFixed(1)} dB`;

/** Ruler tick spacing (seconds) giving roughly 8 to 14 ticks. */
export function tickStep(duration: number): number {
  for (const s of [1, 2, 5, 10, 15, 30, 60, 120, 300]) if (duration / s <= 14) return s;
  return 600;
}
