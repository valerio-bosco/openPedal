// Tone Lab state: talks to the server (or to the local demo when there is none), keeps the job being
// worked on, the progress of the running step and the fit results.
import { useCallback, useEffect, useRef, useState } from "react";
import { TonelabLocalMock } from "./localMock";
import type { Job, JobSummary, TemplateRow, TonelabCaps, TonelabClientMsg, TonelabProgress, TonelabServerMsg, FitSummary } from "./types";

export interface Transport {
  send: (m: TonelabClientMsg) => boolean;
  subscribe: (f: (m: TonelabServerMsg) => void) => () => void;
}

export interface FitResult {
  fit: FitSummary;
  templates: TemplateRow[];
}

/** Where a job's files are served (HTTP, same server as the WebSocket). */
export const fileUrl = (job: string, path: string) => `/tonelab/file/${encodeURIComponent(job)}/${path}`;

export function useTonelab(transport: Transport, demo: boolean, toast: (t: string) => void) {
  const [caps, setCaps] = useState<TonelabCaps>();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const [job, setJob] = useState<Job>();
  const [progress, setProgress] = useState<TonelabProgress>();
  /** A step was asked for and its first progress has not arrived yet. */
  const [starting, setStarting] = useState(false);
  const [results, setResults] = useState<Record<string, FitResult>>({});
  const mock = useRef<TonelabLocalMock>(undefined);

  const onMsg = useCallback((m: TonelabServerMsg) => {
    switch (m.type) {
      case "tonelab_caps":
        return setCaps(m);
      case "tonelab_jobs":
        return setJobs(m.jobs);
      case "tonelab_progress":
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
