import { createContext, memo, useContext } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { INPUT, OUTPUT, PEDAL_IN, PEDAL_OUT, type CatalogItem, type ParamValue } from "../protocol";
import { CATEGORY_COLOR, paramValue, type PNode } from "../model";
import { Knob } from "./Knob";

export interface EditorCtx {
  catalog: Map<string, CatalogItem>;
  setParam: (nodeId: string, paramId: string, value: ParamValue) => void;
  removeNode: (nodeId: string) => void;
}
export const EditorContext = createContext<EditorCtx>({ catalog: new Map(), setParam: () => {}, removeNode: () => {} });

function PedalNodeView({ id, data, selected }: NodeProps<PNode>) {
  const { catalog, setParam, removeNode } = useContext(EditorContext);
  const item = catalog.get(data.kind);
  const color = CATEGORY_COLOR[item?.category ?? "utility"];
  const hasIn = data.kind !== INPUT && data.kind !== PEDAL_IN;
  const hasOut = data.kind !== OUTPUT && data.kind !== PEDAL_OUT;
  const params = item?.params ?? [];
  const cols = params.length <= 4 ? Math.max(1, params.length) : params.length <= 9 ? 3 : 4;
  const special = !hasIn || !hasOut;

  return (
    <div
      className={`pedal${selected ? " selected" : ""}${data.error ? " has-error" : ""}${special ? " special" : ""}${
        item?.composite ? " composite" : ""
      }`}
      style={{ ["--cat" as string]: color }}
    >
      {hasIn && <Handle type="target" position={Position.Left} />}
      {hasOut && <Handle type="source" position={Position.Right} />}
      <div className="pedal-head">
        <span className="led" />
        <span className="pedal-title">{item?.label ?? data.kind}</span>
        {item?.composite && <span className="pedal-badge" title="Pedale composto: doppio clic per modificarlo">composto</span>}
        {!special && (
          <button className="nodrag pedal-remove" title="Elimina pedale (Canc)" onClick={() => removeNode(id)}>
            ×
          </button>
        )}
      </div>
      {!item && <div className="pedal-unknown">Tipo sconosciuto: {data.kind}</div>}
      {params.length > 0 && (
        <div className="pedal-params" style={{ gridTemplateColumns: `repeat(${cols}, auto)` }}>
          {params.map((p) => {
            const v = paramValue(p, data.params[p.id]);
            return (
              <div className="param" key={p.id}>
                {p.kind === "number" && <Knob desc={p} value={v} color={color} onChange={(x) => setParam(id, p.id, x)} />}
                {p.kind === "choice" && (
                  <select
                    className="nodrag nopan param-select"
                    value={v}
                    onChange={(e) => setParam(id, p.id, p.options[Number(e.target.value)] ?? e.target.value)}
                  >
                    {p.options.map((o, i) => (
                      <option key={o} value={i}>
                        {o}
                      </option>
                    ))}
                  </select>
                )}
                {p.kind === "file" && (
                  <input
                    className="nodrag nopan param-file"
                    placeholder={p.extensions.map((e) => "." + e).join(" ")}
                    value={String(data.params[p.id] ?? "")}
                    onChange={(e) => setParam(id, p.id, e.target.value)}
                    onKeyDown={(e) => e.stopPropagation()}
                  />
                )}
                <div className="param-label">{p.label}</div>
              </div>
            );
          })}
        </div>
      )}
      {data.error && <div className="pedal-error">{data.error}</div>}
    </div>
  );
}

export const PedalNodeMemo = memo(PedalNodeView);
