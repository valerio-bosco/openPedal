# Cab IRs

Put WAV files with impulse responses (IRs) of real cabs here. Then, in a graph node or a pedal:

```toml
[[node]]
id = "speaker"
type = "cab"
params = { ir = "irs/file-name.wav", max_ms = 80, level_db = 0 }   # path relative to the openPedal folder
```

Supported: mono or stereo WAV (the left channel is used), 32-bit float or 16/24/32-bit PCM,
any sample rate (it is converted). The volume is normalized automatically.

There are many free IR packs (search for "free guitar cab IR"). Check each one's license.
