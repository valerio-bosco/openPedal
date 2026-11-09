// The pedal's face, built from its controls. Knobs drag, switches and selectors click: these move the
// preview values (what the response graph is measured with), not the saved defaults.
import { useRef } from "react";
import type { ControlSpec, PedalFile } from "../../generated/protocol";
import { controlDefault } from "../../lib/designer";

interface Props {
  draft: PedalFile;
  values: Record<string, number>;
  selected: string | undefined;
  onSelect: (id: string) => void;
  onValue: (id: string, v: number) => void;
}

function PreviewKnob({ c, v, sel, onSelect, onValue }: { c: ControlSpec; v: number; sel: boolean; onSelect: () => void; onValue: (v: number) => void }) {
  const drag = useRef<{ y: number; v: number } | null>(null);
  return (
    <div
      className={`dz-pk${sel ? " sel" : ""}`}
      data-control={c.id}
      role="slider"
      aria-label={c.label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={v}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { y: e.clientY, v };
        onSelect();
      }}
      onPointerMove={(e) => {
        if (drag.current) onValue(Math.max(0, Math.min(1, drag.current.v + (drag.current.y - e.clientY) / (e.shiftKey ? 800 : 160))));
      }}
      onPointerUp={() => (drag.current = null)}
      onPointerCancel={() => (drag.current = null)}
    >
      <i style={{ ["--a" as string]: `${-135 + 270 * v}deg` }} />
      <span>{c.label}</span>
    </div>
  );
}

export function PedalPreview({ draft, values, selected, onSelect, onValue }: Props) {
  return (
    <div className="dz-pedal" style={{ ["--c" as string]: draft.color || "#d9573b" }}>
      <h4>{draft.name || "Untitled"}</h4>
      <div className="dz-face">
        {draft.control.map((c) => {
          const v = values[c.id] ?? controlDefault(c);
          const sel = c.id === selected;
          if (c.kind === "knob") {
            return <PreviewKnob key={c.id} c={c} v={v} sel={sel} onSelect={() => onSelect(c.id)} onValue={(x) => onValue(c.id, x)} />;
          }
          const options = c.options ?? [];
          const click = () => {
            onSelect(c.id);
            onValue(c.id, c.kind === "switch" ? (v >= 0.5 ? 0 : 1) : (Math.round(v) + 1) % Math.max(1, options.length));
          };
          return (
            <div key={c.id} className={`dz-ps${sel ? " sel" : ""}${c.kind === "switch" && v >= 0.5 ? " on" : ""}`} data-control={c.id} role="button" aria-label={c.label} onClick={click}>
              {c.kind === "switch" ? (
                <div className="tog" />
              ) : (
                <div className="opts">
                  {options.map((_, i) => (
                    <b key={i} className={i === Math.round(v) ? "on" : ""} />
                  ))}
                </div>
              )}
              <span>{c.kind === "switch" ? c.label : (options[Math.round(v)] ?? c.label)}</span>
            </div>
          );
        })}
        {draft.control.length === 0 && <span className="dz-empty">No controls yet</span>}
      </div>
      <span className="dz-led" />
      <span className="dz-fs" />
    </div>
  );
}
