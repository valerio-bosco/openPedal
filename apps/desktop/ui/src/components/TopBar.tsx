import type { ReactNode } from "react";
import type { AudioInfo, MetersMsg } from "../generated/protocol";
import type { Status } from "../ws";

const STATUS_TEXT: Record<Status, string> = {
  connecting: "Connecting…",
  online: "Connected",
  mock: "Demo (no server)",
  offline: "Disconnected, reconnecting…",
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
  /** The graph differs from the last load / save. */
  dirty: boolean;
  audio?: AudioInfo;
  meters?: MetersMsg;
  disabledPresets: boolean;
  onLoadPreset: (name: string) => void;
  onSave: () => void;
  onDelete: () => void;
  onTonelab: () => void;
  extra?: ReactNode;
}

export function TopBar({ status, presets, graphName, dirty, audio, meters, disabledPresets, onLoadPreset, onSave, onDelete, onTonelab, extra }: Props) {
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
      {dirty && <span className="dirty" title="Unsaved changes" aria-label="Unsaved changes" />}
      <button onClick={onSave} disabled={disabledPresets}>
        Save preset
      </button>
      <button
        className="icon danger-hover"
        onClick={onDelete}
        disabled={disabledPresets || !presets.includes(graphName)}
        title={presets.includes(graphName) ? `Delete preset «${graphName}»` : "The current graph is not a saved preset"}
        aria-label="Delete preset"
      >
        🗑
      </button>
      {extra}
      <button id="tonelab-open" onClick={onTonelab} disabled={disabledPresets} title="Make a preset from a song">
        Tone Lab
      </button>
      <div className="spacer" />
      <div className="meters">
        <Meter label="IN" db={meters?.in_db ?? -90} />
        <Meter label="OUT" db={meters?.out_db ?? -90} />
        <div className="dsp" title="Audio thread load">
          DSP {Math.round((meters?.dsp ?? 0) * 100)}%
          {(meters?.xruns ?? 0) > 0 && <span className="xruns"> · {meters?.xruns} xrun</span>}
        </div>
      </div>
      <div className={`status status-${status}`} title={audio ? `${audio.sample_rate} Hz, buffer ${audio.buffer}` : ""}>
        <span className="dot" />
        {STATUS_TEXT[status]}
        {audio && !audio.running && status !== "mock" && " · audio off"}
      </div>
    </header>
  );
}
