import { useRef, useState } from "react";
import { formatNumber, fromNorm, toNorm, type NumberDesc } from "../model";

interface Props {
  desc: NumberDesc;
  value: number;
  color: string;
  onChange: (v: number) => void;
}

const SIZE = 46;
const R = 18;
const START = 135; // degrees: the arc goes from 135° to 405° (270° of travel)

function polar(deg: number) {
  const a = (deg * Math.PI) / 180;
  return [SIZE / 2 + R * Math.cos(a), SIZE / 2 + R * Math.sin(a)];
}
function arc(from: number, to: number) {
  const [x1, y1] = polar(from);
  const [x2, y2] = polar(to);
  return `M ${x1} ${y1} A ${R} ${R} 0 ${to - from > 180 ? 1 : 0} 1 ${x2} ${y2}`;
}

/** Circular knob: vertical drag (Shift = fine), double-click to type the value. */
export function Knob({ desc, value, color, onChange }: Props) {
  const drag = useRef<{ y: number; t: number } | null>(null);
  const [editing, setEditing] = useState(false);
  const t = toNorm(desc, value);
  const end = START + 270 * t;

  const onDown = (e: React.PointerEvent) => {
    if (editing) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { y: e.clientY, t };
  };
  const onMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    const px = e.shiftKey ? 800 : 160;
    onChange(fromNorm(desc, drag.current.t + (drag.current.y - e.clientY) / px));
  };
  const onUp = () => {
    drag.current = null;
  };
  const onWheel = (e: React.WheelEvent) => {
    onChange(fromNorm(desc, t - Math.sign(e.deltaY) * 0.02));
  };

  const commit = (text: string) => {
    setEditing(false);
    const v = parseFloat(text.replace(",", "."));
    if (Number.isFinite(v)) onChange(Math.min(desc.max, Math.max(desc.min, v)));
  };

  return (
    <div className="knob nodrag nopan">
      <svg
        width={SIZE}
        height={SIZE}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onWheel={onWheel}
        onDoubleClick={(e) => {
          e.stopPropagation();
          setEditing(true);
        }}
      >
        <circle cx={SIZE / 2} cy={SIZE / 2} r={R + 5} className="knob-body" />
        <path d={arc(START, START + 270)} className="knob-track" />
        {t > 0.003 && <path d={arc(START, end)} className="knob-arc" style={{ stroke: color }} />}
        <line
          x1={SIZE / 2}
          y1={SIZE / 2}
          x2={polar(end)[0]}
          y2={polar(end)[1]}
          className="knob-needle"
        />
      </svg>
      {editing ? (
        <input
          className="knob-edit"
          autoFocus
          defaultValue={String(value)}
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => commit(e.currentTarget.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") commit(e.currentTarget.value);
            if (e.key === "Escape") setEditing(false);
          }}
        />
      ) : (
        <div className="knob-value">
          {formatNumber(desc, value)}
          {desc.unit && <span className="knob-unit"> {desc.unit}</span>}
        </div>
      )}
    </div>
  );
}
