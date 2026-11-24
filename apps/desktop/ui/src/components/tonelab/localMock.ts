// Browser-only fallback for the Tone Lab panel (no server at all, "Demo" mode): answers the Tone Lab
// messages with a fake job so the whole flow can be tried. Audio files do not exist in this mode.
import type { TonelabJobInfo, TonelabPart, TonelabStage, TonelabTemplateResult } from "../../generated/protocol";
import type { TonelabClientMsg, TonelabEvent } from "../../ws";

const DEMO: TonelabJobInfo = {
  id: "2026-11-24-demo-song",
  title: "Demo song",
  source_kind: "file",
  duration_s: 180,
  tempo_bpm: 92,
  key: "E minor",
  sections: [
    { start: 0, end: 16, kind: "none", guitar_db: -60, corr: 0 },
    { start: 16, end: 48, kind: "centre", guitar_db: -22, corr: 0.93 },
    { start: 48, end: 80, kind: "double", guitar_db: -20, corr: 0.41 },
    { start: 80, end: 112, kind: "wide", guitar_db: -24, corr: 0.12 },
    { start: 112, end: 160, kind: "centre", guitar_db: -19, corr: 0.9 },
    { start: 160, end: 180, kind: "none", guitar_db: -55, corr: 0 },
  ],
  parts: [],
};

const RECORD_UNAVAILABLE =
  "recording the DI from the live input is not available yet: record it with `openpedal --record` and put the dry channel into parts/<part>/di-user.wav";

const TEMPLATES: TonelabTemplateResult[] = [
  { template: "fuzz-octave", error_db: 1.9, renders: 352, stage: "fit" },
  { template: "fuzz", error_db: 2.1, renders: 348, stage: "fit" },
  { template: "overdrive", error_db: 3.4, renders: 61, stage: "screen" },
  { template: "highgain", error_db: 3.9, renders: 60, stage: "screen" },
  { template: "crunch", error_db: 4.6, renders: 60, stage: "screen" },
  { template: "clean", error_db: 7.2, renders: 59, stage: "screen" },
];

export class TonelabLocalMock {
  private job: TonelabJobInfo | undefined;
  private timers: number[] = [];
  constructor(private emit: (m: TonelabEvent) => void) {}

  stop() {
    this.timers.forEach((t) => window.clearTimeout(t));
    this.timers = [];
  }

  private steps(job: string, plan: [TonelabStage, string][], done: () => void, ms = 350) {
    plan.forEach(([stage, message], i) => {
      this.timers.push(
        window.setTimeout(() => {
          this.emit({ type: "tonelab_progress", job, stage, message, fraction: (i + 1) / (plan.length + 1) });
        }, ms * (i + 1)),
      );
    });
    this.timers.push(window.setTimeout(done, ms * (plan.length + 1)));
  }

  private sendJob() {
    if (this.job) this.emit({ type: "tonelab_job", job: structuredClone(this.job) });
  }

  handle(m: TonelabClientMsg) {
    switch (m.type) {
      case "tonelab_caps":
        return this.emit({ type: "tonelab_caps", available: true, youtube: true, device: "cpu (demo)", reason: null });
      case "tonelab_jobs":
        return this.emit({ type: "tonelab_jobs", jobs: [{ id: DEMO.id, title: DEMO.title, created: "2026-11-24T10:00:00Z", duration_s: DEMO.duration_s }] });
      case "tonelab_open":
        this.job = this.job ?? structuredClone(DEMO);
        return this.sendJob();
      case "tonelab_start":
        this.job = structuredClone(DEMO);
        return this.steps(
          DEMO.id,
          [
            ["fetch", "Reading the song"],
            ["separate", "Separating the instruments"],
            ["analyse", "Looking for the guitar"],
          ],
          () => this.sendJob(),
        );
      case "tonelab_pick": {
        const job = this.job;
        if (!job) return;
        const part: TonelabPart = {
          id: `p${job.parts.length + 1}`,
          label: m.label,
          start: m.start,
          end: m.end,
          channel: m.channel,
          notes: Math.round((m.end - m.start) * 4.5),
          has_user_di: false,
          has_synth_di: true,
          fit: null,
        };
        return this.steps(
          job.id,
          [
            ["parts", "Cutting the part and finding the notes"],
            ["synth", "Building a synthetic input"],
          ],
          () => {
            job.parts.push(part);
            this.sendJob();
          },
        );
      }
      case "tonelab_record_di":
        // like the real server today
        return this.emit({ type: "error", message: RECORD_UNAVAILABLE });
      case "tonelab_fit": {
        const part = this.job?.parts.find((p) => p.id === m.part);
        return this.steps(
          m.job,
          [
            ["fit", "Trying every template"],
            ["fit", "Fitting the best two"],
            ["fit", "Checking on held-out audio"],
          ],
          () => {
            const fit = { template: "fuzz-octave", error_db: 1.9, holdout_error_db: 5.3, validate_error_db: part?.has_user_di ? null : 5.3 };
            if (part) part.fit = fit;
            this.emit({ type: "tonelab_fit_result", job: m.job, part: m.part, fit, templates: TEMPLATES });
            this.sendJob();
          },
        );
      }
      case "tonelab_cancel":
        this.stop();
        return this.emit({ type: "tonelab_progress", job: m.job, stage: "cancelled", message: "Cancelled", fraction: null });
      case "tonelab_save_preset":
        return;
    }
  }
}
