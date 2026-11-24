// The song map: sections coloured by kind on a time ruler, click-drag to select a range, play it.
import { useEffect, useRef, useState } from "react";
import { fileUrl } from "./useTonelab";
import { KIND_COLOR, KIND_TEXT, clock, db, tickStep } from "./format";
import type { TonelabChannel, TonelabJobInfo } from "../../generated/protocol";

interface Props {
  job: TonelabJobInfo;
  disabled: boolean;
  onPick: (start: number, end: number, channel: TonelabChannel, label: string) => void;
}

const CHANNELS: { id: TonelabChannel; label: string; hint: string }[] = [
  { id: "mid", label: "Middle", hint: "What is in the centre of the stereo image" },
  { id: "left", label: "Left", hint: "Only the left speaker" },
  { id: "right", label: "Right", hint: "Only the right speaker" },
];

export function SongMap({ job, disabled, onPick }: Props) {
  const [sel, setSel] = useState<[number, number]>();
  const [channel, setChannel] = useState<TonelabChannel>("mid");
  const [label, setLabel] = useState("");
  const [pos, setPos] = useState<number>();
  const [playing, setPlaying] = useState<"selection" | "mix">();
  const track = useRef<HTMLDivElement>(null);
  const audio = useRef<HTMLAudioElement>(null);
  const drag = useRef<number>(undefined);
  const stopAt = useRef<number>(undefined);
  const dur = Math.max(job.duration_s, 1);

  // a different job: forget the selection and stop playing
  useEffect(() => {
    setSel(undefined);
    stop();
  }, [job.id]);

  const timeAt = (clientX: number) => {
    const r = track.current!.getBoundingClientRect();
    return Math.max(0, Math.min(dur, ((clientX - r.left) / r.width) * dur));
  };
  const onDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = timeAt(e.clientX);
    setSel([drag.current, drag.current]);
  };
  const onMove = (e: React.PointerEvent) => {
    if (drag.current === undefined) return;
    const t = timeAt(e.clientX);
    setSel([Math.min(drag.current, t), Math.max(drag.current, t)]);
  };
  const onUp = () => {
    drag.current = undefined;
    setSel((s) => (s && s[1] - s[0] < 0.5 ? undefined : s));
  };

  function stop() {
    audio.current?.pause();
    stopAt.current = undefined;
    setPlaying(undefined);
    setPos(undefined);
  }
  function play(what: "selection" | "mix") {
    const a = audio.current;
    if (!a) return;
    if (playing === what) return stop();
    a.src = fileUrl(job.id, what === "mix" ? "mix.wav" : "stems/guitar.wav");
    const from = what === "selection" && sel ? sel[0] : 0;
    stopAt.current = what === "selection" && sel ? sel[1] : undefined;
    const go = () => {
      a.currentTime = from;
      void a.play().catch(() => setPlaying(undefined));
    };
    if (a.readyState >= 1) go();
    else a.addEventListener("loadedmetadata", go, { once: true });
    setPlaying(what);
  }
  const onTime = () => {
    const a = audio.current;
    if (!a) return;
    setPos(a.currentTime);
    if (stopAt.current !== undefined && a.currentTime >= stopAt.current) stop();
  };

  const step = tickStep(dur);
  const ticks: number[] = [];
  for (let t = 0; t <= dur; t += step) ticks.push(t);
  const pct = (t: number) => `${(t / dur) * 100}%`;
  const selLen = sel ? sel[1] - sel[0] : 0;
  const canPick = !!sel && selLen >= 1 && !disabled;

  return (
    <section className="tl-card" data-testid="songmap">
      <div className="tl-head">
        <h3>Song map</h3>
        <span className="tl-meta">
          {job.tempo_bpm ? `${Math.round(job.tempo_bpm)} BPM` : "tempo unknown"} · {job.key ?? "key unknown"} · {clock(job.duration_s)}
        </span>
      </div>
      <p className="hint">Drag across the map to select the part you want to copy (a riff or a chorus, 5 to 30 seconds works best).</p>
      <div className="tl-ruler">
        {ticks.map((t) => (
          <span key={t} style={{ left: pct(t) }}>
            {clock(t)}
          </span>
        ))}
      </div>
      <div className="tl-track" ref={track} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
        {job.sections.map((s, i) => (
          <div
            key={i}
            className={`tl-section k-${s.kind}`}
            data-kind={s.kind}
            style={{ left: pct(s.start), width: pct(s.end - s.start), background: KIND_COLOR[s.kind] }}
            title={`${clock(s.start)} to ${clock(s.end)}. ${KIND_TEXT[s.kind]}. Guitar level ${db(s.guitar_db)}, left/right similarity ${s.corr.toFixed(2)}`}
          />
        ))}
        {sel && <div className="tl-sel" data-testid="selection" style={{ left: pct(sel[0]), width: pct(selLen) }} />}
        {pos !== undefined && <div className="tl-play" style={{ left: pct(pos) }} />}
      </div>
      <div className="tl-legend">
        {(["centre", "double", "wide", "none"] as const).map((k) => (
          <span key={k} title={KIND_TEXT[k]}>
            <i style={{ background: KIND_COLOR[k] }} />
            {k === "none" ? "No guitar" : k[0].toUpperCase() + k.slice(1)}
          </span>
        ))}
      </div>
      <div className="tl-row">
        <span className="lbl">TonelabChannel</span>
        <div className="seg">
          {CHANNELS.map((c) => (
            <button key={c.id} className={c.id === channel ? "on" : ""} aria-pressed={c.id === channel} title={c.hint} onClick={() => setChannel(c.id)}>
              {c.label}
            </button>
          ))}
        </div>
        <span className="tl-sel-info" data-testid="sel-info">
          {sel && selLen >= 0.5 ? `${clock(sel[0])} to ${clock(sel[1])} (${selLen.toFixed(1)} s)` : "Nothing selected"}
        </span>
        <span className="grow" />
        <button disabled={!sel || selLen < 0.5} onClick={() => play("selection")}>
          {playing === "selection" ? "Stop" : "Play selection"}
        </button>
        <button onClick={() => play("mix")}>{playing === "mix" ? "Stop" : "Play full mix"}</button>
      </div>
      <div className="tl-row">
        <input type="text" className="tl-label" aria-label="Part name" placeholder="Part name (riff, chorus, solo…)" value={label} onChange={(e) => setLabel(e.target.value)} />
        <button
          className="primary"
          id="tl-pick"
          disabled={!canPick}
          title={canPick ? "" : "Select a range of at least one second"}
          onClick={() => sel && onPick(sel[0], sel[1], channel, label.trim() || `Part at ${clock(sel[0])}`)}
        >
          Pick this part
        </button>
      </div>
      <audio ref={audio} preload="none" onTimeUpdate={onTime} onEnded={stop} />
    </section>
  );
}
