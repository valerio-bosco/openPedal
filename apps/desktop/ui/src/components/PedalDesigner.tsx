// The pedal designer: inner graph on the left, live pedal face and frequency response in the middle,
// controls and their routes on the right.
import { useMemo, useState } from "react";
import type { Category, ControlSpec, PedalFile } from "../generated/protocol";
import { useResponse } from "../hooks/useResponse";
import type { useServer } from "../hooks/useServer";
import { COLORS, controlDefault, effectiveValues, initialValues, kindLabel, newControl, splitTarget, validateDraft } from "../lib/designer";
import { CATEGORY_LABEL, CATEGORY_ORDER } from "../model";
import { ControlEditor } from "./designer/ControlEditor";
import { InnerView } from "./designer/InnerView";
import { PedalPreview } from "./designer/PedalPreview";
import { ResponseGraph } from "./designer/ResponseGraph";

interface Props {
  /** True for a pedal that is not saved yet. */
  isNew: boolean;
  draft: PedalFile;
  server: ReturnType<typeof useServer>;
  onChange: (draft: PedalFile) => void;
  onEditInner: () => void;
  onSave: () => void;
  onCancel: () => void;
}

const ICON = { knob: "◐", switch: "⏻", selector: "≡" } as const;

export function PedalDesigner({ isNew, draft, server, onChange, onEditInner, onSave, onCancel }: Props) {
  const { catalog } = server;
  const [selected, setSelected] = useState<string | undefined>(draft.control[0]?.id);
  const [preview, setPreview] = useState<Record<string, number>>({});
  // preview values: what the user moved, over every control's default
  const values = useMemo(() => ({ ...initialValues(draft), ...preview }), [draft, preview]);

  const problems = useMemo(() => validateDraft(draft, catalog), [draft, catalog]);
  const pedalProblems = problems.filter((p) => p.control === undefined);
  const effective = useMemo(() => effectiveValues(draft, values, catalog), [draft, values, catalog]);
  const control = draft.control.find((c) => c.id === selected);
  const highlight = useMemo(() => new Set((control?.route ?? []).map((r) => splitTarget(r.target).node)), [control]);

  // analysis needs a draft the server can compile; a half-edited control waits
  const analysable = server.status === "online" && draft.node.length > 0 && !problems.some((p) => p.control !== undefined);
  const response = useResponse(server, draft, values, analysable);
  const color = draft.color || COLORS[0];

  const setControl = (next: ControlSpec) => {
    const old = draft.control.find((c) => c.id === next.id);
    onChange({ ...draft, control: draft.control.map((c) => (c.id === next.id ? next : c)) });
    // a new kind or starting position puts the preview there too
    if (old && (old.kind !== next.kind || controlDefault(old) !== controlDefault(next))) {
      setPreview((p) => ({ ...p, [next.id]: controlDefault(next) }));
    }
  };
  const addControl = () => {
    const c = newControl(draft);
    onChange({ ...draft, control: [...draft.control, c] });
    setSelected(c.id);
  };
  const removeControl = (id: string) => {
    const rest = draft.control.filter((c) => c.id !== id);
    onChange({ ...draft, control: rest });
    setPreview(({ [id]: _gone, ...others }) => others);
    setSelected(rest[0]?.id);
  };
  const summary = (c: ControlSpec) => c.route.map((r) => r.target).join(" · ") || "drives nothing yet";
  const problemsOf = (id: string) => problems.filter((p) => p.control === id).map((p) => p.message);

  return (
    <div className="designer" role="dialog" aria-label="Pedal designer" onKeyDown={(e) => e.key === "Escape" && !(e.target instanceof HTMLInputElement) && onCancel()}>
      <div className="dz-bar">
        <span className="dz-title">{isNew ? "New pedal" : "Pedal designer"}</span>
        <input type="text" id="dz-name" aria-label="Pedal name" value={draft.name} autoFocus={isNew} onChange={(e) => onChange({ ...draft, name: e.target.value })} />
        <select aria-label="Category" value={draft.category} onChange={(e) => onChange({ ...draft, category: e.target.value as Category })}>
          {CATEGORY_ORDER.filter((c) => c !== "io").map((c) => (
            <option key={c} value={c}>
              {CATEGORY_LABEL[c]}
            </option>
          ))}
        </select>
        <span className="lbl">Colour</span>
        {COLORS.map((c) => (
          <button key={c} className={`dz-sw${c === draft.color ? " on" : ""}`} style={{ background: c }} aria-label={`Colour ${c}`} aria-pressed={c === draft.color} onClick={() => onChange({ ...draft, color: c })} />
        ))}
        <span className="grow" />
        <button id="dz-cancel" onClick={onCancel}>
          Cancel
        </button>
        <button id="dz-save" className="primary" disabled={problems.length > 0} title={problems[0]?.message} onClick={onSave}>
          Save pedal
        </button>
      </div>
      {pedalProblems.length > 0 && (
        <div className="dz-banner">
          {pedalProblems.map((p, i) => (
            <span key={i}>{p.message}</span>
          ))}
        </div>
      )}
      <div className="dz-main">
        <div className="dz-col">
          <h3>Inside</h3>
          <div className="dz-inner-box">
            <InnerView draft={draft} catalog={catalog} highlight={highlight} />
          </div>
          <p className="hint">The blocks of this pedal. Highlighted: the blocks the selected control drives.</p>
          <label className="dz-field">
            Description (optional)
            <input type="text" value={draft.description} onChange={(e) => onChange({ ...draft, description: e.target.value })} />
          </label>
          <button id="dz-edit-inner" onClick={onEditInner}>
            Edit inner graph…
          </button>
        </div>
        <div className="dz-center">
          <div className="dz-preview">
            <PedalPreview draft={draft} values={values} selected={selected} onSelect={setSelected} onValue={(id, v) => setPreview((p) => ({ ...p, [id]: v }))} />
          </div>
          <div className="dz-graph-box">
            <div className="row">
              <h3>Frequency response</h3>
              <span className="range">20 Hz – 20 kHz</span>
            </div>
            <ResponseGraph response={response} color={color} message={server.status === "online" ? undefined : "The response graph is available when connected to openPedal."} />
          </div>
        </div>
        <div className="dz-col">
          <div className="row between">
            <h3>Controls</h3>
            <button id="dz-add-control" onClick={addControl}>
              + Control
            </button>
          </div>
          <div className="dz-controls">
            {draft.control.map((c) => (
              <div key={c.id} className={`dz-crow${c.id === selected ? " sel" : ""}${problemsOf(c.id).length ? " bad" : ""}`} data-control-row={c.id} onClick={() => setSelected(c.id)}>
                <span className="vicon" title={kindLabel(c.kind)}>
                  {ICON[c.kind]}
                </span>
                <div>
                  <div>{c.label || "(unnamed)"}</div>
                  <div className="t">{summary(c)}</div>
                </div>
                <span className="t">{c.route.length}</span>
              </div>
            ))}
            {draft.control.length === 0 && <div className="hint">Add a control (knob, switch or selector) and choose what it drives.</div>}
          </div>
          {control && (
            <ControlEditor key={control.id} draft={draft} control={control} catalog={catalog} effective={effective} problems={problemsOf(control.id)} onChange={setControl} onRemove={() => removeControl(control.id)} />
          )}
        </div>
      </div>
    </div>
  );
}
