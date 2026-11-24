// Tone Lab protocol v4 (docs/PLAN.md §14.5), written by hand, exactly mirroring the Rust messages
// (serde `tag = "type"`, snake_case). They will be replaced by the generated ones
// (`../../generated/protocol.ts`) at merge. `Option<T>` fields are `T | null` and may be absent.

export type Channel = "mid" | "left" | "right";
export type SectionKind = "centre" | "double" | "wide" | "none";
export type Stage = "fetch" | "separate" | "analyse" | "parts" | "synth" | "record" | "fit" | "cancelled";

export interface JobSummary {
  id: string;
  title: string;
  created: string;
  duration_s: number;
}

export interface Section {
  start: number;
  end: number;
  kind: SectionKind;
  guitar_db: number;
  corr: number;
}

export interface FitSummary {
  template: string;
  error_db: number;
  holdout_error_db?: number | null;
  validate_error_db?: number | null;
}

export interface Part {
  id: string;
  label: string;
  start: number;
  end: number;
  channel: Channel;
  notes: number;
  has_user_di: boolean;
  has_synth_di: boolean;
  fit?: FitSummary | null;
}

export interface Job {
  id: string;
  title: string;
  source_kind: string;
  duration_s: number;
  tempo_bpm?: number | null;
  key?: string | null;
  sections: Section[];
  parts: Part[];
}

export interface TemplateRow {
  template: string;
  error_db: number;
  renders: number;
  stage: string;
}

export type TonelabClientMsg =
  | { type: "tonelab_caps" }
  | { type: "tonelab_jobs" }
  | { type: "tonelab_start"; source: string }
  | { type: "tonelab_open"; job: string }
  | { type: "tonelab_pick"; job: string; start: number; end: number; channel: Channel; label: string }
  | { type: "tonelab_record_di"; job: string; part: string; seconds: number }
  | { type: "tonelab_fit"; job: string; part: string }
  | { type: "tonelab_save_preset"; job: string; part: string; name: string }
  | { type: "tonelab_cancel"; job: string };

export type TonelabServerMsg =
  | { type: "tonelab_caps"; available: boolean; youtube: boolean; device: string; reason?: string | null }
  | { type: "tonelab_jobs"; jobs: JobSummary[] }
  | { type: "tonelab_progress"; job: string; stage: Stage; message: string; fraction?: number | null }
  | { type: "tonelab_job"; job: Job }
  | { type: "tonelab_fit_result"; job: string; part: string; fit: FitSummary; templates: TemplateRow[] };

export type TonelabCaps = Extract<TonelabServerMsg, { type: "tonelab_caps" }>;
export type TonelabProgress = Extract<TonelabServerMsg, { type: "tonelab_progress" }>;

/** True for a message of the Tone Lab family (they bypass the generated guards). */
export const isTonelabType = (t: unknown): boolean => typeof t === "string" && t.startsWith("tonelab_");
