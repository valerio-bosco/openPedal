"""`tonelab <command>`: every command reads/writes a job folder (PLAN §14.2)."""
from __future__ import annotations

import argparse
import json
import sys

from . import analyse as analyse_mod
from . import parts as parts_mod
from .config import ConfigError, jobs_dir, load_config
from .job import Job


def progress(stage: str, fraction: float | None, message: str) -> None:
    """`PROGRESS <stage> <fraction|-> <message>` on stderr: read by the openPedal server."""
    frac = "-" if fraction is None else f"{fraction:.3f}"
    print(f"PROGRESS {stage} {frac} {message}", file=sys.stderr, flush=True)


def cmd_caps(args, cfg):
    caps = {
        "youtube": bool(cfg["sources"]["youtube"]),
        "device": cfg["gpu"]["device"],
        "jobs_dir": str(jobs_dir(cfg)),
    }
    print(json.dumps(caps) if args.json else "\n".join(f"{k}: {v}" for k, v in caps.items()))


def _open(args, cfg) -> Job:
    return Job.open(jobs_dir(cfg), args.job)


def cmd_fetch(args, cfg):
    from .fetch import fetch

    progress("fetch", None, "reading the source")
    job = fetch(args.source, cfg, jobs_dir(cfg), args.title)
    progress("fetch", 1.0, "source ready")
    print(job.id)


def cmd_separate(args, cfg):
    from .separate import separate

    mode = "never" if args.skip else "always" if args.force else None
    job = _open(args, cfg)
    progress("separate", None, "separating the stems (a minute or two on the GPU)")
    print(separate(job, cfg, mode))
    progress("separate", 1.0, "stems ready")


def cmd_analyse(args, cfg):
    job = _open(args, cfg)
    progress("analyse", None, "analysing the song")
    print(analyse_mod.format_table(analyse_mod.analyse(job)))
    progress("analyse", 1.0, "analysis ready")


def cmd_parts(args, cfg):
    job = _open(args, cfg)
    progress("parts", None, "extracting the part and transcribing the notes")
    part = parts_mod.add_part(job, args.start, args.end, args.channel, args.label)
    progress("parts", 1.0, "part ready")
    warn = part.pop("_warning")
    print(f"{part['id']}  {part['label']}  {len(part['notes'])} notes")
    if warn:
        print(f"warning: no notes written. {warn}", file=sys.stderr)


def cmd_synth(args, cfg):
    job = _open(args, cfg)
    progress("synth", None, "synthesising the DI")
    print(parts_mod.synth_part(job, args.part, args.match_di))
    progress("synth", 1.0, "DI ready")


def cmd_run(args, cfg):
    from .fetch import fetch
    from .separate import separate

    job = fetch(args.source, cfg, jobs_dir(cfg), args.title)
    print(f"job {job.id}", file=sys.stderr)
    print(separate(job, cfg), file=sys.stderr)
    print(analyse_mod.format_table(analyse_mod.analyse(job)))
    print(job.id)


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="tonelab", description="Tone Lab: from a song to a guitar part")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("caps", help="what this install supports (youtube, device, jobs dir)")
    p.add_argument("--json", action="store_true")
    p.set_defaults(fn=cmd_caps)

    p = sub.add_parser("fetch", help="path or URL -> new job; prints the job id")
    p.add_argument("source")
    p.add_argument("--title")
    p.set_defaults(fn=cmd_fetch)

    p = sub.add_parser("separate", help="Demucs htdemucs_6s -> stems/")
    p.add_argument("job")
    g = p.add_mutually_exclusive_group()
    g.add_argument("--force", action="store_true", help="always separate")
    g.add_argument("--skip", action="store_true", help="never separate (guitar.wav = mix)")
    p.set_defaults(fn=cmd_separate)

    p = sub.add_parser("analyse", help="sections, tempo, key, low notes -> analysis.json")
    p.add_argument("job")
    p.set_defaults(fn=cmd_analyse)

    p = sub.add_parser("parts", help="pick a section + channel -> parts/<id>/target.wav")
    p.add_argument("job")
    p.add_argument("--start", type=float, required=True)
    p.add_argument("--end", type=float, required=True)
    p.add_argument("--channel", choices=parts_mod.CHANNELS, required=True)
    p.add_argument("--label")
    p.set_defaults(fn=cmd_parts)

    p = sub.add_parser("synth-di", help="synthetic DI from a part's notes")
    p.add_argument("job")
    p.add_argument("--part", required=True)
    p.add_argument("--match-di", help="wav of a real DI to match spectrally")
    p.set_defaults(fn=cmd_synth)

    p = sub.add_parser("run", help="fetch + separate + analyse")
    p.add_argument("source")
    p.add_argument("--title")
    p.set_defaults(fn=cmd_run)
    return ap


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        args.fn(args, load_config())
    except (ConfigError, parts_mod.MissingModule, FileNotFoundError, ValueError, KeyError) as e:
        print(f"tonelab: {e}", file=sys.stderr)
        return 2
    except Exception as e:  # sources / ffmpeg / yt-dlp
        from .sources import SourceError

        if isinstance(e, SourceError):
            print(f"tonelab: {e}", file=sys.stderr)
            return 2
        raise
    return 0


if __name__ == "__main__":
    sys.exit(main())
