"""`tonelab <command>`: every command reads/writes a job folder (PLAN §14.2)."""
from __future__ import annotations

import argparse
import sys

from . import analyse as analyse_mod
from . import parts as parts_mod
from .config import ConfigError, jobs_dir, load_config
from .job import Job


def _open(args, cfg) -> Job:
    return Job.open(jobs_dir(cfg), args.job)


def cmd_fetch(args, cfg):
    from .fetch import fetch

    job = fetch(args.source, cfg, jobs_dir(cfg), args.title)
    print(job.id)


def cmd_separate(args, cfg):
    from .separate import separate

    mode = "never" if args.skip else "always" if args.force else None
    print(separate(_open(args, cfg), cfg, mode))


def cmd_analyse(args, cfg):
    print(analyse_mod.format_table(analyse_mod.analyse(_open(args, cfg))))


def cmd_parts(args, cfg):
    part = parts_mod.add_part(_open(args, cfg), args.start, args.end, args.channel, args.label)
    warn = part.pop("_warning")
    print(f"{part['id']}  {part['label']}  {len(part['notes'])} notes")
    if warn:
        print(f"warning: no notes written. {warn}", file=sys.stderr)


def cmd_synth(args, cfg):
    print(parts_mod.synth_part(_open(args, cfg), args.part, args.match_di))


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
