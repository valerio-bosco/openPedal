import { useState } from "react";
import type { Category, KnobSpec } from "../protocol";
import { CATEGORY_LABEL, CATEGORY_ORDER, formatNumber, type NumberDesc } from "../model";

/** Numeric parameter of an internal node that can be exposed as a knob. */
export interface Candidate {
  nodeId: string;
  nodeLabel: string;
  desc: NumberDesc;
  current: number;
}

export interface PedalForm {
  name: string;
  category: Category;
  description: string;
  knobs: KnobSpec[];
}

interface Row {
  c: Candidate;
  on: boolean;
  label: string;
  min: string;
  max: string;
  /** Initial position 0..1. */
  def: number;
}

interface Props {
  title: string;
  submitLabel: string;
  initial: { name: string; category: Category; description: string };
  candidates: Candidate[];
  /** Knobs that already exist (editing a pedal). */
  knobs: KnobSpec[];
  onSubmit: (f: PedalForm) => void;
  onCancel: () => void;
}

export function PedalDialog({ title, submitLabel, initial, candidates, knobs, onSubmit, onCancel }: Props) {
  const [name, setName] = useState(initial.name);
  const [category, setCategory] = useState<Category>(initial.category);
  const [description, setDescription] = useState(initial.description);
  const [error, setError] = useState("");
  const [rows, setRows] = useState<Row[]>(() =>
    candidates.map((c) => {
      const k = knobs.find((x) => x.target === `${c.nodeId}.${c.desc.id}`);
      const span = c.desc.max - c.desc.min;
      return {
        c,
        on: !!k,
        label: k?.label ?? `${c.nodeLabel} ${c.desc.label}`.trim(),
        min: String(k?.min ?? c.desc.min),
        max: String(k?.max ?? c.desc.max),
        def: k?.default ?? (span > 0 ? Math.min(1, Math.max(0, (c.current - c.desc.min) / span)) : 0),
      };
    }),
  );

  const patch = (i: number, p: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)));

  const submit = () => {
    if (!name.trim()) return setError("Give the pedal a name.");
    const on = rows.filter((r) => r.on);
    const labels = on.map((r) => r.label.trim());
    if (labels.some((l) => !l)) return setError("Every exposed knob needs a name.");
    if (new Set(labels).size !== labels.length) return setError("Knob names must be different from each other.");
    const out: KnobSpec[] = [];
    for (const r of on) {
      const min = parseFloat(r.min.replace(",", "."));
      const max = parseFloat(r.max.replace(",", "."));
      if (!Number.isFinite(min) || !Number.isFinite(max) || min === max)
        return setError(`Invalid range for «${r.label}».`);
      out.push({ label: r.label.trim(), target: `${r.c.nodeId}.${r.c.desc.id}`, min, max, default: r.def });
    }
    onSubmit({ name: name.trim(), category, description: description.trim(), knobs: out });
  };

  return (
    <div className="modal-bg" onMouseDown={(e) => e.target === e.currentTarget && onCancel()}>
      <div className="modal" onKeyDown={(e) => e.key === "Escape" && onCancel()}>
        <h2>{title}</h2>
        <div className="form-row">
          <label>
            Name
            <input type="text" autoFocus value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <label style={{ flex: "0 0 180px" }}>
            Category
            <select value={category} onChange={(e) => setCategory(e.target.value as Category)}>
              {CATEGORY_ORDER.filter((c) => c !== "io").map((c) => (
                <option key={c} value={c}>
                  {CATEGORY_LABEL[c]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="form-row">
          <label>
            Description (optional)
            <input type="text" value={description} onChange={(e) => setDescription(e.target.value)} />
          </label>
        </div>
        <p className="hint">Choose which parameters to expose as knobs of the new pedal (from 0 to 1 over the given range).</p>
        {rows.length === 0 ? (
          <p className="hint">No numeric parameters available in the chosen nodes.</p>
        ) : (
          <table className="knob-table">
            <thead>
              <tr>
                <th></th>
                <th>Parameter</th>
                <th>Knob name</th>
                <th>Minimum</th>
                <th>Maximum</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={`${r.c.nodeId}.${r.c.desc.id}`}>
                  <td>
                    <input type="checkbox" checked={r.on} onChange={(e) => patch(i, { on: e.target.checked })} />
                  </td>
                  <td>
                    {r.c.nodeLabel} · {r.c.desc.label}
                    <span className="knob-unit"> ({formatNumber(r.c.desc, r.c.current)} {r.c.desc.unit})</span>
                  </td>
                  <td>
                    <input type="text" disabled={!r.on} value={r.label} onChange={(e) => patch(i, { label: e.target.value })} />
                  </td>
                  <td>
                    <input type="text" className="num" disabled={!r.on} value={r.min} onChange={(e) => patch(i, { min: e.target.value })} />
                  </td>
                  <td>
                    <input type="text" className="num" disabled={!r.on} value={r.max} onChange={(e) => patch(i, { max: e.target.value })} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {error && <div className="err">{error}</div>}
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button className="primary" onClick={submit}>
            {submitLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
