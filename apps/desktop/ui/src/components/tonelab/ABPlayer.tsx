// A/B player: the song's separated part against the preset's render, one toggle, same playback position.
import { useRef, useState } from "react";

interface Props {
  targetUrl: string;
  resultUrl: string;
}

export function ABPlayer({ targetUrl, resultUrl }: Props) {
  const [side, setSide] = useState<"target" | "result">("result");
  const [playing, setPlaying] = useState(false);
  const refs = { target: useRef<HTMLAudioElement>(null), result: useRef<HTMLAudioElement>(null) };

  const toggle = (next: "target" | "result") => {
    if (next === side) return;
    const from = refs[side].current;
    const to = refs[next].current;
    if (from && to) {
      to.currentTime = Math.min(from.currentTime, to.duration || from.currentTime);
      if (playing) void to.play().catch(() => setPlaying(false));
      from.pause();
    }
    setSide(next);
  };
  const playPause = () => {
    const a = refs[side].current;
    if (!a) return;
    if (playing) {
      a.pause();
      setPlaying(false);
    } else {
      void a.play().then(() => setPlaying(true), () => setPlaying(false));
    }
  };
  const ended = () => {
    setPlaying(false);
    for (const k of ["target", "result"] as const) if (refs[k].current) refs[k].current!.currentTime = 0;
  };

  return (
    <div className="tl-ab" data-testid="ab" data-side={side}>
      <button onClick={playPause} id="tl-ab-play">
        {playing ? "Pause" : "Play"}
      </button>
      <div className="seg">
        <button className={side === "target" ? "on" : ""} id="tl-ab-target" aria-pressed={side === "target"} onClick={() => toggle("target")} title="The guitar separated from the song">
          Song (target)
        </button>
        <button className={side === "result" ? "on" : ""} id="tl-ab-result" aria-pressed={side === "result"} onClick={() => toggle("result")} title="The synthetic or recorded input played through the fitted preset">
          Preset (result)
        </button>
      </div>
      <span className="hint">Switch while it plays: both start from the same moment.</span>
      <audio ref={refs.target} src={targetUrl} preload="auto" onEnded={ended} />
      <audio ref={refs.result} src={resultUrl} preload="auto" onEnded={ended} />
    </div>
  );
}
