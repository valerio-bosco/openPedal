"""Run demucs.cpp's converter under torch>=2.6 (torch.load defaults to weights_only=True there)."""
import runpy, sys, torch
_orig = torch.load
torch.load = lambda *a, **k: _orig(*a, **{**k, "weights_only": False})
script = sys.argv[1]
sys.argv = sys.argv[1:]
runpy.run_path(script, run_name="__main__")
