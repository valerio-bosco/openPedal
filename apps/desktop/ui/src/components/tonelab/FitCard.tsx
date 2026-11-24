// A part: its input (DI), the fit button, the result in plain words, A/B player and saving.
import { useState } from "react";
import { ABPlayer } from "./ABPlayer";
import { clock, db } from "./format";
import { fileUrl, type FitResult } from "./useTonelab";
import type { TonelabFit, TonelabJobInfo, TonelabPart } from "../../generated/protocol";

interface Props {
  job: TonelabJobInfo;
  part: TonelabPart;
  result?: FitResult;
  busy: boolean;
  /** The server refused to record this part from the panel. */
  recordError?: string;
  onRecord: (seconds: number) => void;
  onFit: () => void;
  onSave: (name: string) => void;
}

const COUNTDOWN_S = 2;

function diText(p: TonelabPart): string {
  if (p.has_user_di) return "your recording";
  if (p.has_synth_di) return "the synthetic input";
  return "nothing yet (the synthetic input is still being built)";
}

function Numbers({ fit }: { fit: TonelabFit }) {
  return (
    <ul className="tl-numbers">
      <li data-testid="err">
        <b>{db(fit.error_db)}</b> spectral distance to the song. It measures how far the tone's colour (bass, mids, treble) is from the song's. Lower is better; under 3 dB is very close.
      </li>
      {fit.holdout_error_db != null && (
        <li>
          <b>{db(fit.holdout_error_db)}</b> on audio the fit never saw (held out). If this is much higher than the number above, the preset may only suit the input it was fitted on.
        </li>
      )}
      {fit.validate_error_db != null && (
        <li>
          <b>{db(fit.validate_error_db)}</b> on a real recording of your guitar. This is the honest number: how the preset sounds with a real input.
        </li>
      )}
    </ul>
  );
}

export function FitCard({ job, part, result, busy, recordError, onRecord, onFit, onSave }: Props) {
  const fit = result?.fit ?? part.fit ?? undefined;
  const [name, setName] = useState(`${job.title} ${part.label}`.replace(/[\\/]/g, "-").trim());
  const secs = Math.ceil(part.end - part.start);
  const [upload, setUpload] = useState(false);

  return (
    <div className="tl-fit" data-part={part.id}>
      <div className="tl-di">
        <span className="lbl">Input for the fit</span>
        <span data-testid="di-use">
          The fit will use <b>{diText(part)}</b>.
        </span>
        <span className="grow" />
        <button
          disabled={busy}
          data-act="record"
          onClick={() => onRecord(secs + COUNTDOWN_S)}
          title={`Recording starts after a ${COUNTDOWN_S} s countdown and lasts ${secs} s: play the part, dry, on your guitar`}
        >
          {part.has_user_di ? "Record again" : "Record my take"}
        </button>
        <button data-act="upload" aria-expanded={upload} onClick={() => setUpload((u) => !u)}>
          Upload my take
        </button>
        <button className="primary" disabled={busy || (!part.has_user_di && !part.has_synth_di)} data-act="fit" onClick={onFit}>
          {fit ? "Fit again" : "Fit this part"}
        </button>
      </div>
      {recordError && (
        <p className="tl-error" role="alert" data-testid="record-error">
          {recordError}
        </p>
      )}
      {(upload || recordError) && (
        <div className="tl-cli" data-testid="record-cli">
          <p>
            Recording from this panel is not available yet. Record your take on the command line, then copy it into the job folder (by default{" "}
            <code>~/.local/share/openpedal/tonelab/jobs/{job.id}</code>):
          </p>
          <pre>{`openpedal --record take.wav --seconds ${secs}\ncp take.wav <job folder>/parts/${part.id}/di-user.wav`}</pre>
          <p className="hint">
            The recording keeps your dry guitar on the left channel, which is the one the fit reads. Reopen the song afterwards and this part will say "your recording".
          </p>
        </div>
      )}
      <p className="hint">
        A recording of you playing this part ({secs} s, after a {COUNTDOWN_S} s countdown) makes the preset fit your own guitar. Without one, a synthetic input built from the detected notes is used.
      </p>
      {fit && (
        <div className="tl-result" data-testid="fit-result">
          <div className="tl-head">
            <h4>
              Result: <span className="tl-tpl">{fit.template}</span>
            </h4>
            <span className="tl-meta">
              {part.label}, {clock(part.start)} to {clock(part.end)}
            </span>
          </div>
          <Numbers fit={fit} />
          {result && result.templates.length > 0 && (
            <table className="tl-table" data-testid="templates">
              <thead>
                <tr>
                  <th title="The kind of amp and pedal chain tried">Template</th>
                  <th title="Spectral distance to the song, lower is better">Error</th>
                  <th title="How many times the chain was played through to test settings">Renders</th>
                  <th title="screen: a quick first look at every template; fit: full tuning of the best ones">Stage</th>
                </tr>
              </thead>
              <tbody>
                {result.templates.map((t) => (
                  <tr key={t.template} className={t.template === fit.template ? "best" : ""}>
                    <td>{t.template}</td>
                    <td>{db(t.error_db)}</td>
                    <td>{t.renders}</td>
                    <td>{t.stage}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <ABPlayer targetUrl={fileUrl(job.id, `parts/${part.id}/target.wav`)} resultUrl={fileUrl(job.id, `parts/${part.id}/fit/render.wav`)} />
          <div className="tl-row">
            <input type="text" id="tl-name" aria-label="Preset name" value={name} onChange={(e) => setName(e.target.value)} />
            <button className="primary" id="tl-save" disabled={!name.trim()} onClick={() => onSave(name.trim())}>
              Save as preset
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
