// Editor of the selected control: name, kind, positions, default and its routes (what it drives).
import { useState } from "react";
import type { CatalogItem, ControlSpec, ControlType, MapKind, ParamDesc, ParamValue, PedalFile, RouteSpec } from "../../generated/protocol";
import {
  CONTROL_KINDS,
  CURVES,
  controlDefault,
  convertControl,
  curveY,
  formatValue,
  kindLabel,
  newRoute,
  numeric,
  resizeSelector,
  resolveTarget,
  splitTarget,
} from "../../lib/designer";
import { NumField } from "./NumField";

interface Props {
  draft: PedalFile;
  control: ControlSpec;
  catalog: Map<string, CatalogItem>;
  /** Value every driven parameter has now ("node.param" -> number, choices as option index). */
  effective: Map<string, number>;
  problems: string[];
  onChange: (c: ControlSpec) => void;
  onRemove: () => void;
}

function CurveThumb({ kind }: { kind: MapKind }) {
  const pts = Array.from({ length: 21 }, (_, i) => {
    const n = i / 20;
    return `${i ? "L" : "M"}${(4 + n * 32).toFixed(1)} ${(22 - curveY(kind, n) * 18).toFixed(1)}`;
  });
  return (
    <svg viewBox="0 0 40 26" aria-hidden="true">
      <path d={pts.join("")} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

/** A route value: a number for a numeric parameter, an option name for a choice. */
function ValueField({ desc, value, label, onChange }: { desc: ParamDesc; value: ParamValue | undefined; label: string; onChange: (v: ParamValue) => void }) {
  if (desc.kind === "choice") {
    const cur = typeof value === "string" ? value : desc.options[numeric(desc, value)];
    return (
      <select aria-label={label} value={cur ?? ""} onChange={(e) => onChange(e.target.value)}>
        {desc.options.map((o) => (
          <option key={o}>{o}</option>
        ))}
      </select>
    );
  }
  return <NumField label={label} value={typeof value === "number" ? value : 0} onChange={onChange} />;
}

interface RouteProps {
  control: ControlSpec;
  route: RouteSpec;
  desc: ParamDesc | undefined;
  item: CatalogItem | undefined;
  now: string;
  onChange: (r: RouteSpec) => void;
  onRemove: () => void;
}

function RouteRow({ control: c, route: r, desc, item, now, onChange, onRemove }: RouteProps) {
  const canAdd = desc?.kind === "number" && !item?.composite;
  const name = r.target;
  return (
    <div className="dz-route" data-target={name}>
      <div className="head">
        <b>{name}</b>
        <span className="now">
          now <span data-now={name}>{now}</span>
        </span>
        <button className="x" aria-label={`Remove route to ${name}`} title="Remove route" onClick={onRemove}>
          ×
        </button>
      </div>
      {desc && c.kind === "knob" && (
        <div className="row">
          <span className="lbl">From</span>
          <ValueField desc={desc} label={`${name} from`} value={r.min} onChange={(v) => onChange({ ...r, min: Number(v) })} />
          <span className="lbl">to</span>
          <ValueField desc={desc} label={`${name} to`} value={r.max} onChange={(v) => onChange({ ...r, max: Number(v) })} />
          <span className="grow" />
          {CURVES.map(({ kind, label }) => {
            const on = (r.map ?? "linear") === kind;
            const off = kind === "log" && !((r.min ?? 0) > 0 && (r.max ?? 0) > 0);
            return (
              <button
                key={kind}
                className={`curvebtn${on ? " on" : ""}`}
                title={off ? "Log needs From and To above 0" : label}
                aria-label={label}
                aria-pressed={on}
                disabled={off && !on}
                onClick={() => onChange({ ...r, map: kind === "linear" ? undefined : kind })}
              >
                <CurveThumb kind={kind} />
              </button>
            );
          })}
        </div>
      )}
      {desc && c.kind === "switch" && (
        <div className="row">
          <span className="lbl">Off</span>
          <ValueField desc={desc} label={`${name} off`} value={r.off} onChange={(v) => onChange({ ...r, off: v })} />
          <span className="lbl">On</span>
          <ValueField desc={desc} label={`${name} on`} value={r.on} onChange={(v) => onChange({ ...r, on: v })} />
        </div>
      )}
      {desc && c.kind === "selector" && (
        <div className="row">
          {(c.options ?? []).map((o, i) => (
            <label key={i} className="pos">
              <span className="lbl">{o || `#${i + 1}`}</span>
              <ValueField
                desc={desc}
                label={`${name} at ${o}`}
                value={r.values?.[i]}
                onChange={(v) => onChange({ ...r, values: (c.options ?? []).map((_, k) => (k === i ? v : (r.values?.[k] ?? v))) })}
              />
            </label>
          ))}
        </div>
      )}
      {!desc && <div className="row hint">This parameter no longer exists.</div>}
      <div className="row">
        <span className="lbl">Combine</span>
        <div className="seg">
          {(["set", "add"] as const).map((m) => (
            <button
              key={m}
              className={(r.mode ?? "set") === m ? "on" : ""}
              disabled={m === "add" && !canAdd}
              title={m === "set" ? "Replaces the parameter value" : canAdd ? "Adds on top of the other routes and the block's own value" : "Add mode needs a numeric block parameter"}
              onClick={() => onChange({ ...r, mode: m === "set" ? undefined : "add" })}
            >
              {m === "set" ? "Set" : "Add"}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

export function ControlEditor({ draft, control: c, catalog, effective, problems, onChange, onRemove }: Props) {
  const [picker, setPicker] = useState(false);
  const routed = new Set(c.route.map((r) => r.target));
  const setRoute = (i: number, r: RouteSpec) => onChange({ ...c, route: c.route.map((x, k) => (k === i ? r : x)) });

  const addRoute = (target: string, desc: ParamDesc) => {
    const { node } = resolveTarget(draft, catalog, target);
    onChange({ ...c, route: [...c.route, newRoute(c, target, desc, node?.params[splitTarget(target).param])] });
    setPicker(false);
  };
  const setOptions = (options: string[]) => onChange(resizeSelector(c, options));
  const def = controlDefault(c);

  return (
    <div className="dz-ved" data-editor={c.id}>
      <div className="row">
        <input type="text" className="grow" aria-label="Control name" value={c.label} onChange={(e) => onChange({ ...c, label: e.target.value })} />
        <div className="seg" role="group" aria-label="Control kind">
          {CONTROL_KINDS.map((k: ControlType) => (
            <button key={k} className={c.kind === k ? "on" : ""} aria-pressed={c.kind === k} onClick={() => onChange(convertControl(draft, catalog, c, k))}>
              {kindLabel(k)}
            </button>
          ))}
        </div>
      </div>
      {c.kind === "selector" && (
        <div className="row">
          <span className="lbl">Positions</span>
          {(c.options ?? []).map((o, i) => (
            <input
              key={i}
              type="text"
              className="opt"
              aria-label={`Position ${i + 1}`}
              value={o}
              onChange={(e) => setOptions((c.options ?? []).map((x, k) => (k === i ? e.target.value : x)))}
            />
          ))}
          <button className="icon" title="Add a position" aria-label="Add a position" onClick={() => setOptions([...(c.options ?? []), `Option ${(c.options ?? []).length + 1}`])}>
            +
          </button>
          <button className="icon" title="Remove the last position" aria-label="Remove a position" disabled={(c.options ?? []).length <= 2} onClick={() => setOptions((c.options ?? []).slice(0, -1))}>
            −
          </button>
        </div>
      )}
      <div className="row">
        <span className="lbl">Starts at</span>
        {c.kind === "knob" && <NumField label="Default position" value={def} onChange={(v) => onChange({ ...c, default: Math.max(0, Math.min(1, v)) })} />}
        {c.kind === "knob" && <span className="hint">0 to 1</span>}
        {c.kind === "switch" && (
          <div className="seg">
            {[false, true].map((on) => (
              <button key={String(on)} className={(def >= 0.5) === on ? "on" : ""} onClick={() => onChange({ ...c, default: on })}>
                {on ? "On" : "Off"}
              </button>
            ))}
          </div>
        )}
        {c.kind === "selector" && (
          <select aria-label="Default position" value={def} onChange={(e) => onChange({ ...c, default: Number(e.target.value) })}>
            {(c.options ?? []).map((o, i) => (
              <option key={i} value={i}>
                {o || `#${i + 1}`}
              </option>
            ))}
          </select>
        )}
      </div>
      <h3>Drives</h3>
      {c.route.length === 0 && <div className="hint">Nothing yet: add a route to an inner parameter.</div>}
      {c.route.map((r, i) => {
        const { node, item, desc } = resolveTarget(draft, catalog, r.target);
        const n = effective.get(r.target) ?? (node && desc ? numeric(desc, node.params[splitTarget(r.target).param]) : undefined);
        return <RouteRow key={`${r.target}#${i}`} control={c} route={r} desc={desc} item={item} now={formatValue(desc, n)} onChange={(x) => setRoute(i, x)} onRemove={() => onChange({ ...c, route: c.route.filter((_, k) => k !== i) })} />;
      })}
      <div className="dz-picker-wrap">
        <button id="dz-add-route" onClick={() => setPicker((p) => !p)}>
          + Add route
        </button>
        {picker && (
          <>
            <div className="dz-backdrop" onClick={() => setPicker(false)} />
            <div className="dz-menu" role="menu">
              {draft.node.length === 0 && <div className="sec">No blocks inside the pedal.</div>}
              {draft.node.map((n) => {
                const item = catalog.get(n.type);
                return (
                  <div key={n.id}>
                    <div className="sec">
                      {item?.label ?? n.type} · {n.id}
                    </div>
                    {(item?.params ?? []).map((p) => {
                      const target = `${n.id}.${p.id}`;
                      const why = p.kind === "file" ? "a file" : c.kind === "knob" && p.kind === "choice" ? "a choice: use a switch or selector" : routed.has(target) ? "already routed" : "";
                      const val = effective.get(target) ?? numeric(p, n.params[p.id]);
                      return (
                        <button key={p.id} role="menuitem" data-target={target} disabled={!!why} title={why} onClick={() => addRoute(target, p)}>
                          <span>{p.label || p.id}</span>
                          <small>{p.kind === "file" ? "file" : formatValue(p, val)}</small>
                        </button>
                      );
                    })}
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>
      {problems.map((p, i) => (
        <div key={i} className="dz-err">
          {p}
        </div>
      ))}
      <div className="row">
        <span className="grow" />
        <button className="danger-hover" onClick={onRemove}>
          Remove control
        </button>
      </div>
    </div>
  );
}
