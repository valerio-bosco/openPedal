import type { ReactNode } from "react";
import type { AudioInfo, MetersMsg } from "../protocol";
import type { Status } from "../ws";

const STATUS_TEXT: Record<Status, string> = {
  connecting: "Connessione…",
  online: "Connesso",
  mock: "Demo (senza server)",
  offline: "Disconnesso, riconnessione…",
};

function Meter({ label, db }: { label: string; db: number }) {
  const pct = Math.max(0, Math.min(1, (db + 60) / 60)) * 100;
  return (
    <div className="meter" title={`${label} ${db.toFixed(1)} dB`}>
      <span className="meter-label">{label}</span>
      <div className="meter-bar">
        <div className="meter-fill" style={{ width: `${pct}%` }} />
        <div className="meter-clip" style={{ opacity: db > -3 ? 1 : 0 }} />
      </div>
      <span className="meter-db">{db > -90 ? db.toFixed(0) : "−∞"}</span>
    </div>
  );
}

interface Props {
  status: Status;
  presets: string[];
  graphName: string;
  audio?: AudioInfo;
  meters?: MetersMsg;
  disabledPresets: boolean;
  onLoadPreset: (name: string) => void;
  onSave: () => void;
  extra?: ReactNode;
}

export function TopBar({ status, presets, graphName, audio, meters, disabledPresets, onLoadPreset, onSave, extra }: Props) {
  return (
    <header className="topbar">
      <div className="logo">
        open<b>Pedal</b>
      </div>
      <label className="preset">
        Preset
        <select
          disabled={disabledPresets}
          value={presets.includes(graphName) ? graphName : ""}
          onChange={(e) => e.target.value && onLoadPreset(e.target.value)}
        >
          <option value="" disabled>
            {graphName || "—"}
          </option>
          {presets.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </label>
      <button onClick={onSave} disabled={disabledPresets}>
        Salva preset
      </button>
      {extra}
      <div className="spacer" />
      <div className="meters">
        <Meter label="IN" db={meters?.in_db ?? -90} />
        <Meter label="OUT" db={meters?.out_db ?? -90} />
        <div className="dsp" title="Carico del thread audio">
          DSP {Math.round((meters?.dsp ?? 0) * 100)}%
          {(meters?.xruns ?? 0) > 0 && <span className="xruns"> · {meters?.xruns} xrun</span>}
        </div>
      </div>
      <div className={`status status-${status}`} title={audio ? `${audio.sample_rate} Hz, buffer ${audio.buffer}` : ""}>
        <span className="dot" />
        {STATUS_TEXT[status]}
        {audio && !audio.running && status !== "mock" && " · audio spento"}
      </div>
    </header>
  );
}
