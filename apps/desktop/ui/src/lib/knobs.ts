// The simple "knob" view of a pedal's controls, used by the current Create/Edit pedal dialog:
// a knob is a 0..1 control with one linear route. (The pedal file is format v2, protocol v3: the
// pedal designer works with the full controls and routes.)
import type { ControlSpec, PedalFile } from "../generated/protocol";

export interface KnobSpec {
  label: string;
  /** "<internal node id>.<parameter id>", e.g. "c.high_hz". */
  target: string;
  /** Internal parameter values when the knob is at 0 and at 1. */
  min: number;
  max: number;
  /** Initial knob position, 0..1. */
  default: number;
}

/** The pedal's plain knobs (knob controls with a single linear route). */
export function knobsOf(p: PedalFile): KnobSpec[] {
  const out: KnobSpec[] = [];
  for (const c of p.control) {
    const r = c.route[0];
    if (c.kind !== "knob" || c.route.length !== 1 || (r.map ?? "linear") !== "linear") continue;
    out.push({ label: c.label || c.id, target: r.target, min: r.min ?? 0, max: r.max ?? 1, default: typeof c.default === "number" ? c.default : 0.5 });
  }
  return out;
}

export function controlsOf(knobs: KnobSpec[]): ControlSpec[] {
  return knobs.map((k) => ({
    id: k.label,
    label: k.label,
    kind: "knob" as const,
    default: k.default,
    route: [{ target: k.target, min: k.min, max: k.max }],
  }));
}
