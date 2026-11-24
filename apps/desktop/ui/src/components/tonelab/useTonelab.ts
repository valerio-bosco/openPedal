// Tone Lab state: talks to the server (or to the local demo when there is none), keeps the job being
// worked on, the progress of the running step and the fit results.
import { useCallback, useEffect, useRef, useState } from "react";
import { TonelabLocalMock } from "./localMock";
import type { TonelabFit, TonelabJobInfo, TonelabJobSummary, TonelabTemplateResult } from "../../generated/protocol";
import type { TonelabClientMsg, TonelabEvent, TonelabServerMsg } from "../../ws";

export type TonelabCaps = Extract<TonelabServerMsg, { type: "tonelab_caps" }>;
export type TonelabProgress = Extract<TonelabServerMsg, { type: "tonelab_progress" }>;

export interface Transport {
  send: (m: TonelabClientMsg) => boolean;
  subscribe: (f: (m: TonelabEvent) => void) => () => void;
}

export interface FitResult {
  fit: TonelabFit;
  templates: TonelabTemplateResult[];
}

/** Where a job's files are served (HTTP, same server as the WebSocket). */
export const fileUrl = (job: string, path: string) => `/tonelab/file/${encodeURIComponent(job)}/${path}`;

export function useTonelab(transport: Transport, demo: boolean, toast: (t: string) => void) {
  const [caps, setCaps] = useState<TonelabCaps>();
  const [jobs, setJobs] = useState<TonelabJobSummary[]>([]);
  const [job, setJob] = useState<TonelabJobInfo>();
  const [progress, setProgress] = useState<TonelabProgress>();
  /** A step was asked for and its first progress has not arrived yet. */
  const [starting, setStarting] = useState(false);
  const [results, setResults] = useState<Record<string, FitResult>>({});
  /** The part a recording was asked for, until the server answers. */
  const recording = useRef<{ job: string; part: string }>(undefined);
  /** The server refused the last recording of this part. */
  const [recordError, setRecordError] = useState<{ job: string; part: string; message: string }>();
  const mock = useRef<TonelabLocalMock>(undefined);

  const onMsg = useCallback((m: TonelabEvent) => {
    switch (m.type) {
      case "error":
        if (m.node) return; // a node problem, not a Tone Lab step
        // a step that was asked for failed: stop waiting; a refused recording is shown on its part
        setStarting(false);
        setProgress(undefined);
        if (recording.current) {
          setRecordError({ ...recording.current, message: m.message });
          recording.current = undefined;
        }
        return;
      case "tonelab_caps":
        return setCaps(m);
      case "tonelab_jobs":
        return setJobs(m.jobs);
      case "tonelab_progress":
        // while fetching, the server calls the job "new"; the next progress carries the real id. Cancel
        // always uses the latest one, and `tonelab_job` then opens the new job.
        setStarting(false);
        return setProgress(m);
      case "tonelab_job":
        setStarting(false);
        setProgress(undefined);
        setJob(m.job);
        // a new job appears in the list
        setJobs((js) => (js.some((j) => j.id === m.job.id) ? js : [{ id: m.job.id, title: m.job.title, created: "", duration_s: m.job.duration_s }, ...js]));
        return;
      case "tonelab_fit_result":
        setStarting(false);
        setProgress(undefined);
        setResults((r) => ({ ...r, [`${m.job}/${m.part}`]: { fit: m.fit, templates: m.templates } }));
    }
  }, []);

  const send = useCallback(
    (m: TonelabClientMsg) => {
      if (!demo && transport.send(m)) return;
      mock.current?.handle(m);
    },
    [transport, demo],
  );

  useEffect(() => {
    mock.current = demo ? new TonelabLocalMock(onMsg) : undefined;
    const unsub = transport.subscribe(onMsg);
    send({ type: "tonelab_caps" });
    send({ type: "tonelab_jobs" });
    return () => {
      unsub();
      mock.current?.stop();
    };
  }, [transport, demo, onMsg, send]);

  const busy = starting || (progress !== undefined && progress.stage !== "cancelled");

  const act = useCallback(
    (m: TonelabClientMsg) => {
      setStarting(true);
      setProgress(undefined);
      if (m.type === "tonelab_record_di") {
        recording.current = { job: m.job, part: m.part };
        setRecordError(undefined);
      }
      send(m);
    },
    [send],
  );

  /** Upload a file, then start the job from the path the server returns. */
  const startFile = useCallback(
    async (file: File) => {
      if (demo) return act({ type: "tonelab_start", source: file.name });
      setStarting(true);
      try {
        const r = await fetch("/tonelab/upload", { method: "POST", headers: { "X-Filename": file.name.replace(/[^\x20-\x7e]/g, "_") }, body: file });
        if (!r.ok) throw new Error(`upload failed (${r.status})`);
        const { path } = (await r.json()) as { path: string };
        act({ type: "tonelab_start", source: path });
      } catch (e) {
        setStarting(false);
        toast(`Tone Lab: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [act, demo, toast],
  );

  return {
    caps,
    jobs,
    job,
    progress,
    busy,
    results,
    recordError,
    startFile,
    startLink: (source: string) => act({ type: "tonelab_start", source }),
    open: (id: string) => act({ type: "tonelab_open", job: id }),
    pick: (j: string, start: number, end: number, channel: "mid" | "left" | "right", label: string) =>
      act({ type: "tonelab_pick", job: j, start, end, channel, label }),
    record: (j: string, part: string, seconds: number) => act({ type: "tonelab_record_di", job: j, part, seconds }),
    fit: (j: string, part: string) => act({ type: "tonelab_fit", job: j, part }),
    save: (j: string, part: string, name: string) => send({ type: "tonelab_save_preset", job: j, part, name }),
    cancel: (j: string) => send({ type: "tonelab_cancel", job: j }),
    close: () => setJob(undefined),
  };
}
