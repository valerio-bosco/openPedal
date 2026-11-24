"""Config: tonelab.toml (committed defaults) + optional tonelab.local.toml (gitignored)."""
from __future__ import annotations

import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class ConfigError(RuntimeError):
    pass


def _merge(base: dict, over: dict) -> dict:
    out = dict(base)
    for k, v in over.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


def load_config(root: Path | None = None) -> dict:
    root = Path(root) if root else ROOT
    cfg: dict = {}
    for name in ("tonelab.toml", "tonelab.local.toml"):
        p = root / name
        if p.exists():
            cfg = _merge(cfg, tomllib.loads(p.read_text()))
    cfg.setdefault("sources", {}).setdefault("youtube", False)
    cfg.setdefault("gpu", {}).setdefault("device", "cuda")
    cfg.setdefault("paths", {}).setdefault("jobs", "~/.local/share/openpedal/tonelab/jobs")
    sep = cfg.setdefault("separate", {})
    sep.setdefault("mode", "auto")
    sep.setdefault("segment", 7)
    return cfg


def jobs_dir(cfg: dict) -> Path:
    return Path(cfg["paths"]["jobs"]).expanduser()


def resolve_device(cfg: dict) -> str:
    """'cuda' fails loudly when CUDA is missing; 'cpu' only when set explicitly."""
    dev = cfg["gpu"]["device"]
    if dev == "cpu":
        return "cpu"
    if dev.startswith("cuda"):
        import torch

        if not torch.cuda.is_available():
            raise ConfigError(
                f'gpu.device is "{dev}" but CUDA is not available. Check the driver and '
                'LD_LIBRARY_PATH (use bin/tonelab), or set device = "cpu" explicitly in '
                "tonelab.local.toml (slow)."
            )
        return dev
    raise ConfigError(f'gpu.device must be "cuda" or "cpu", got "{dev}"')
