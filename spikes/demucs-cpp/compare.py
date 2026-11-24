"""Compare guitar stems: C++ (demucs.cpp) vs Python Demucs. Usage: compare.py work/"""
import sys, numpy as np, soundfile as sf
from scipy.signal import resample_poly, correlate, welch
W = sys.argv[1]
def load(p, sr=44100):
    x, r = sf.read(p, always_2d=True); x = x.astype(np.float64)
    if r != sr:
        from math import gcd
        g = gcd(sr, r); x = resample_poly(x, sr // g, r // g, axis=0)
    return x
full = load('/home/voodoo/.local/share/openpedal/tonelab/jobs/2026-11-24-cryogen/stems/guitar.wav')
ref_full = full[6*44100:30*44100]
py = load(f'{W}/pyout/htdemucs_6s/excerpt/guitar.wav')
cands = {'pyfull(6-30s)': ref_full, 'pyexcerpt': py, 'pyexcerpt_shifts0': load(f'{W}/pyout_s0/htdemucs_6s/excerpt/guitar.wav'), 'pyexcerpt_run2': load(f'{W}/pyout_r2/htdemucs_6s/excerpt/guitar.wav')}
for k in ['out-t1', 'outmt-t1', 'outmt-t4', 'outmt-t8']:
    cands['cpp ' + k] = load(f'{W}/{k}/target_4_guitar.wav')
def align(a, b):
    n = min(len(a), len(b)); a, b = a[:n], b[:n]
    c = correlate(a.mean(1), b.mean(1), mode='full', method='fft')
    lag = int(np.argmax(c)) - (n - 1)
    return lag
def third_oct(x, sr=44100):
    f, p = welch(x.mean(1), sr, nperseg=8192)
    cs = 1000 * 2 ** (np.arange(-14, 11) / 3)  # 40 Hz .. 10 kHz
    cs = cs[(cs >= 40) & (cs <= 10000)]
    return cs, np.array([10*np.log10(p[(f >= c/2**(1/6)) & (f < c*2**(1/6))].sum() + 1e-20) for c in cs])
def metrics(a, b):
    n = min(len(a), len(b)); a, b = a[:n], b[:n]
    corr = np.corrcoef(a.ravel(), b.ravel())[0, 1]
    cl = np.corrcoef(a[:,0], b[:,0])[0,1]; cr = np.corrcoef(a[:,1], b[:,1])[0,1]
    sdr = 10*np.log10((a**2).sum() / (((a-b)**2).sum() + 1e-20))
    _, sa = third_oct(a); _, sb = third_oct(b)
    d = sa - sb; d -= 0  # absolute level difference kept
    rms = np.sqrt(np.mean(d**2))
    gain = 10*np.log10((b**2).sum() / (a**2).sum())
    return corr, cl, cr, sdr, rms, gain, np.sqrt(np.mean((a**2)))
print('lengths(s):', {k: round(len(v)/44100, 2) for k, v in cands.items()})
for refname in ['pyexcerpt_shifts0', 'pyexcerpt', 'pyfull(6-30s)']:
    ref = cands[refname]
    print(f'\n== reference: {refname}  (rms {np.sqrt(np.mean(ref**2)):.4f})')
    print(f'{"candidate":22s} lag  corr   corrL  corrR  SDRdB  1/3oct_rms_dB  level_dB')
    for k, v in cands.items():
        if k == refname: continue
        lag = align(ref, v)
        a, b = (ref[max(0,-lag):], v[max(0,lag):])
        m = metrics(a, b)
        print(f'{k:22s} {lag:4d} {m[0]:.4f} {m[1]:.4f} {m[2]:.4f} {m[3]:6.2f} {m[4]:8.2f}      {m[5]:+.2f}')
