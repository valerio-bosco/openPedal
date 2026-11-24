// Where a song comes from: a file (drop or pick), or a link when the server allows it.
import { useRef, useState } from "react";
import type { TonelabCaps } from "./useTonelab";

interface Props {
  caps: TonelabCaps;
  busy: boolean;
  onFile: (f: File) => void;
  onLink: (url: string) => void;
}

export function SourceBox({ caps, busy, onFile, onLink }: Props) {
  const [over, setOver] = useState(false);
  const [link, setLink] = useState("");
  const input = useRef<HTMLInputElement>(null);

  return (
    <section className="tl-card" data-testid="source">
      <h3>Song</h3>
      <div
        className={`tl-drop${over ? " over" : ""}${busy ? " off" : ""}`}
        id="tl-drop"
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          const f = e.dataTransfer.files[0];
          if (f && !busy) onFile(f);
        }}
      >
        <div>Drop an audio file here (mp3, wav, m4a, a voice memo…)</div>
        <button disabled={busy} onClick={() => input.current?.click()}>
          Choose a file…
        </button>
        <input
          ref={input}
          id="tl-file"
          type="file"
          accept="audio/*,video/mp4,.m4a,.mp3,.wav,.flac,.ogg"
          hidden
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) onFile(f);
          }}
        />
      </div>
      {caps.youtube && (
        <form
          className="tl-row"
          onSubmit={(e) => {
            e.preventDefault();
            if (link.trim()) onLink(link.trim());
          }}
        >
          <input type="text" id="tl-link" aria-label="Link to the song" placeholder="…or paste a link (personal use only)" value={link} onChange={(e) => setLink(e.target.value)} />
          <button type="submit" disabled={busy || !link.trim()}>
            Fetch
          </button>
        </form>
      )}
    </section>
  );
}
