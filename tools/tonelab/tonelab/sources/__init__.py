"""Sources: each turns a reference into mix.wav. `file` always; `youtube` only when enabled."""
from __future__ import annotations

import importlib


class SourceError(RuntimeError):
    pass


def is_url(ref: str) -> bool:
    return ref.startswith(("http://", "https://"))


def load_source(kind: str, cfg: dict):
    """Return the source module. YouTube is imported lazily and only when enabled."""
    if kind == "file":
        return importlib.import_module("tonelab.sources.file")
    if kind == "youtube":
        if not cfg.get("sources", {}).get("youtube", False):
            raise SourceError(
                "YouTube is disabled ([sources] youtube = false in tonelab.toml). "
                "Set it to true (and install the extra: uv sync --extra youtube) to use links."
            )
        try:
            return importlib.import_module("tonelab.sources.youtube")
        except ModuleNotFoundError as e:
            raise SourceError(
                f"YouTube source unavailable ({e.name} missing). Install it with "
                "`uv sync --extra youtube`, or set [sources] youtube = false."
            ) from e
    raise SourceError(f"unknown source kind: {kind}")
