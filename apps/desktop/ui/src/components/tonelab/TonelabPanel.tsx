// Tone Lab: make a preset that sounds like a guitar part of a song (docs/PLAN.md §14).
import "./tonelab.css";
import { clock } from "./format";
import { FitCard } from "./FitCard";
import { SongMap } from "./SongMap";
import { SourceBox } from "./SourceBox";
import { useTonelab, type Transport } from "./useTonelab";

interface Props {
  transport: Transport;
  /** No server at all: the panel runs on its own demo data. */
  demo: boolean;
  toast: (text: string) => void;
  onClose: () => void;
}

const STAGE_TEXT: Record<string, string> = {
  fetch: "Reading the song",
  separate: "Separating the instruments",
  analyse: "Looking for the guitar",
  parts: "Cutting the part",
  synth: "Building a synthetic input",
  record: "Recording your take",
  fit: "Fitting a preset",
  cancelled: "Cancelled",
};

export function TonelabPanel({ transport, demo, toast, onClose }: Props) {
  const t = useTonelab(transport, demo, toast);
  const { caps, job, progress } = t;
  const state = !caps ? "waiting" : caps.available ? "ready" : "unavailable";

  return (
    <div className="designer tonelab" role="dialog" aria-label="Tone Lab" data-state={state} onKeyDown={(e) => e.key === "Escape" && !(e.target instanceof HTMLInputElement) && onClose()}>
      <div className="dz-bar">
        <span className="dz-title">Tone Lab</span>
        <span className="tl-meta">Make a preset that sounds like a guitar in a song</span>
        <span className="grow" />
        {caps?.available && <span className="tl-meta" title="Where the heavy work (separating the instruments) runs">Processing on {caps.device}</span>}
        <button id="tl-close" onClick={onClose}>
          Close
        </button>
      </div>
      <div />
      <div className="tl-body">
        {!caps && <p className="hint">Connecting to Tone Lab…</p>}
        {caps && !caps.available && (
          <section className="tl-card tl-unavailable" data-testid="unavailable">
            <h3>Tone Lab is not set up</h3>
            <p>{caps.reason || "The Tone Lab tool was not found."}</p>
            <p className="hint">
              It needs the Python tool in <code>tools/tonelab</code>. Follow <code>tools/tonelab/README.md</code> (install, GPU, config), then reopen this panel.
            </p>
          </section>
        )}
        {caps?.available && (
          <>
            <aside className="tl-side">
              <SourceBox caps={caps} busy={t.busy} onFile={(f) => void t.startFile(f)} onLink={t.startLink} />
              <section className="tl-card" data-testid="jobs">
                <h3>Past songs</h3>
                {t.jobs.length === 0 && <p className="hint">Nothing yet. Songs you analyse show up here so you can come back to them.</p>}
                <ul className="tl-jobs">
                  {t.jobs.map((j) => (
                    <li key={j.id}>
                      <button className={j.id === job?.id ? "on" : ""} data-job={j.id} disabled={t.busy} onClick={() => t.open(j.id)}>
                        <span>{j.title}</span>
                        <span className="tl-meta">{clock(j.duration_s)}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            </aside>
            <main className="tl-main">
              {t.busy && (
                <section className="tl-card tl-progress" data-testid="progress">
                  <div className="tl-head">
                    <h3>{progress ? STAGE_TEXT[progress.stage] ?? progress.stage : "Starting…"}</h3>
                    <button id="tl-cancel" disabled={!progress} onClick={() => progress && t.cancel(progress.job)}>
                      Cancel
                    </button>
                  </div>
                  <div className="tl-bar" role="progressbar" aria-valuenow={Math.round((progress?.fraction ?? 0) * 100)}>
                    <div className={progress?.fraction == null ? "indet" : ""} style={{ width: progress?.fraction == null ? undefined : `${progress.fraction * 100}%` }} />
                  </div>
                  <p className="hint">{progress?.message ?? "Working…"}</p>
                </section>
              )}
              {progress?.stage === "cancelled" && !t.busy && <p className="hint" data-testid="cancelled">The last step was cancelled.</p>}
              {!job && !t.busy && <p className="hint tl-empty">Add a song, or open a past one, to see where the guitar is.</p>}
              {job && (
                <>
                  <div className="tl-title">
                    <h2>{job.title}</h2>
                  </div>
                  <SongMap
                    job={job}
                    disabled={t.busy}
                    onPick={(start, end, channel, label) => t.pick(job.id, start, end, channel, label)}
                  />
                  <section className="tl-card" data-testid="parts">
                    <h3>Parts</h3>
                    {job.parts.length === 0 && <p className="hint">Select a range on the song map and pick it. Each part you pick gets its own preset.</p>}
                    {job.parts.map((p) => (
                      <div key={p.id} className="tl-part" data-testid="part">
                        <div className="tl-head">
                          <h4>{p.label}</h4>
                          <span className="tl-meta">
                            {clock(p.start)} to {clock(p.end)} · {p.channel} · {p.notes} notes ·{" "}
                            <span data-testid="di-status">{p.has_user_di ? "your recording" : p.has_synth_di ? "synthetic input" : "no input yet"}</span>
                          </span>
                        </div>
                        <FitCard
                          job={job}
                          part={p}
                          result={t.results[`${job.id}/${p.id}`]}
                          busy={t.busy}
                          recordError={t.recordError?.job === job.id && t.recordError.part === p.id ? t.recordError.message : undefined}
                          onRecord={(s) => t.record(job.id, p.id, s)}
                          onFit={() => t.fit(job.id, p.id)}
                          onSave={(name) => {
                            t.save(job.id, p.id, name);
                            toast(`Saved preset "${name}". Find it in the preset list.`);
                          }}
                        />
                      </div>
                    ))}
                  </section>
                </>
              )}
            </main>
          </>
        )}
      </div>
    </div>
  );
}
