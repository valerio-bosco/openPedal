# Muse - "Cryogen" guitar tones

Single released 24 April 2026 (Warner), third single of *The WOW! Signal* (album 26 June 2026). Length about 5:02. Written by Muse and Dan Lancaster, produced by Muse and Lancaster, additional production Aleks von Korff.

Confidence tags: **confirmed** (primary or direct statement), **likely** (fan database or consistent inference), **speculative** (educated guess).

## Sources

- [MuseWiki: Cryogen (song)](https://musewiki.org/Cryogen_(song)): opens with a wailing Fuzz Factory oscillator sound; riff compared to "Plug In Baby" with the same fuzzy tone; string section in the middle 8; outro riff with a short bass lick.
- [NME](https://www.nme.com/news/music/muse-give-another-taster-of-the-wow-signal-with-old-school-space-rocker-cryogen-3942135) (via search summary; page not readable): "squelching intro", then a "sky-scraping spiral of guitar"; Lancaster plays keys and additional guitar live.
- [Bass Magazine](https://bassmagazine.com/muse-release-new-single-cryogen-listen/): "grinding bass" (no bass gear detail).
- [Equipboard: Bellamy gear](https://equipboard.com/pros/matthew-bellamy?hide_incorrect_submissions=true&page=6&sort=new): fan-compiled list crediting Aleks von Korff's posts with Human Gear Animato, Gamechanger Plasma Coil (Third Man), ZVEX/Vexter Fat Fuzz Factory, MXR Phase 95, Way Huge Havalina on "Cryogen"; Eventide H9 Max and Hologram Chroma Console only seen in the studio background. No links to the original posts.
- [Guitar World, Bellamy x Manson interview 2026](https://www.guitarworld.com/artists/guitarists/matt-bellamy-muse-manson-guitar-works-2026-interview): Bellamy built the Fuzz Factory circuit into many stage guitars. Nothing specific to Cryogen.
- [MuseWiki: Guitar Rig](https://musewiki.org/Guitar_Rig): 2026 tour table puts Cryogen in the "Standard Bender" row (Chrome FR 2.0 or MB Floyd Sus Wow! Signal Red).
- [Wikipedia: Cryogen](https://en.wikipedia.org/wiki/Cryogen_(song)): credits, 5:03, genre notes only.
- Community tabs on [Ultimate Guitar](https://tabs.ultimate-guitar.com/tab/muse/cryogen-tabs-6232220) (not readable here) and a TikTok breakdown, via search summaries: standard tuning, bends and a whammy-style note in the outro.

## Facts and confidence

| Claim | Confidence |
|---|---|
| Fuzz Factory is the core sound; intro is Fuzz Factory oscillation | confirmed (MuseWiki; Bellamy's guitars have the circuit built in) |
| Standard tuning (EADGBE) | likely (tabs, tour table "Standard Bender") |
| Guitar with Floyd Rose-type bridge ("Bender"), whammy bar bends | likely |
| Fat Fuzz Factory, Plasma Coil, Havalina (fuzzes), Animato (overdrive), Phase 95 on the recording | likely (single fan database citing von Korff) |
| Amp is Marshall-style valve amp | speculative (his usual live sound; nothing about this record) |
| Octave pedal on the riff | speculative (none reported; do not add) |
| Bass is fuzzed (Wolstenholme) | likely, but irrelevant to these presets |

## Parts

Time stamps: no reliable ones found. Rough order from descriptions: squelching fuzz intro, spiralling riff, verse, arena chorus, breakdown, middle 8 with strings, return, outro riff plus bass lick. Fill in times once a recording is in `recordings/`.

1. **Intro** (`cryogen-intro`): Fuzz Factory pushed to gated oscillation, hand-played (gate/stab knobs, possibly Kaoss pad or whammy). Likely chain: guitar, Fuzz Factory (built in), amp, cab.
2. **Main riff** (`cryogen-riff`): fast spiralling figure, fuzzy, "Plug In Baby" family. Chain: guitar, Fuzz Factory, Marshall-style amp, 4x12. Extra fuzz/overdrive pedals may add colour (likely).
3. **Chorus / rhythm** (`cryogen-chorus`): sustained wide chords. Less gating, more sustain; phaser likely (Phase 95). All of this part's detail is inferred (speculative).

Not covered: strings in the middle 8 and any keys/synth (Lancaster), bass.

## Mapping to openPedal blocks

| Real gear | Block | Notes |
|---|---|---|
| Fuzz Factory | `fuzz` | drive, gate, comp, bias, tone approximate Volume, Gate, Comp, Stab, Drive |
| Marshall amp | `tonestack` (marshall) + `drive` (asym) | |
| 4x12 cab | `cab` | |
| Room | `reverb` | |
| Phase 95 | none: `chorus` as a weak stand-in | missing phaser block |
| Whammy / Floyd bends, Kaoss pad | none | no pitch-shifter block, no live control of params by a pad |
| Havalina, Plasma Coil, Animato | none specific; `drive` could be stacked | not added to keep chain simple |
| Octave down (Whammy or Octavider) | `octave` (`sub`) in `cryogen-chorus` | from the audio: the guitar reaches B1 (62 Hz) with the bass silent |

Missing blocks that would help: phaser, pitch shifter/whammy, an oscillating fuzz (self-oscillation of the Fuzz Factory when the gate/stab knobs are extreme).

## Analysis of the recording (2026-11-24)

`recordings/Cryogen.mp3` (the user's copy, 5:04) was separated with Demucs `htdemucs_6s` on the GPU
(guitar, bass, drums, vocals, piano, other). Findings from the guitar stem:

- **Key and tempo:** B minor with the A# leading tone (pitch classes F#, B, D, A#, C#), about 132 BPM.
- **One main guitar, centred:** in the intro riff (8-28 s), 80-92 s, 188-196 s and 220-228 s the stem is
  almost mono (L/R correlation 0.9).
- **Double-tracked choruses:** 48-76 s, 116-140 s, 200-212 s and 248-288 s are wide (correlation 0.4-0.5)
  but left and right play the same notes (chroma similarity 0.97): two takes of the same part panned hard,
  not two different parts.
- **232-246 s:** fully decorrelated guitar with bass and drums silent: a breakdown with a stereo effect.
- **Octave down:** the guitar stem goes down to B1 (62 Hz) also where the bass stem is silent (232-244 s).
  A standard-tuned guitar stops at E2 (82 Hz), so an octave-down effect (or a lower tuning) is used.
- **Riff notes (pYIN, approximate):** B3 D4 F#4 B4 A#4 B4 C5 C#5 ... a spiralling B-minor arpeggio.

**Tuning method.** There is no clean DI of the song. A synthetic DI was made from the transcription
(onsets + pYIN, Karplus-Strong strings), its spectrum matched to the user's real DI
(`recordings/plug-in-baby.wav`) and its RMS matched (-29.8 dBFS). Each preset was rendered on it and its tone
parameters were fitted (Nelder-Mead, ~350 renders) to the separated guitar's 1/3-octave spectrum plus crest
factor. Check on the real DI (not used in the fit), rms spectral error against the stem:

| Preset | Before | After |
|---|---|---|
| `cryogen-riff` (vs 8-28 s) | 15.1 dB | 5.9 dB |
| `cryogen-chorus` (vs 64-76 s) | 10.8 dB | 6.7 dB |

The originals were too narrow (a 2-4 kHz peak), 10-15 dB short on 150-1000 Hz, dark above 8 kHz and too
compressed. What remains: 1-4 kHz still 5-9 dB short on the real DI. Scripts were throwaway, kept out of the repo.

## When a reference clip arrives

Put it in `recordings/` and measure per part: spectral centroid and band energy (low cut around 100-150 Hz, 2-4 kHz presence, rolloff above 5-6 kHz), RMS against peak (compression of the fuzz), decay behaviour of notes (gate sputter: does the tail cut abruptly?), presence of a phaser notch sweep (rate, depth) in the chorus, pitch glides (whammy) in the riff and outro, and whether the intro oscillation sits at a fixed pitch. Listen for: octave content, stereo doubling, how much room reverb is audible. Then tune `fuzz.gate`, `fuzz.bias` and `tone_hz` first.

## Presets

Rendered with `openpedal render` on `plug-in-baby.wav` and `q256.wav`: peaks about -16 dBFS and -13 dBFS, RMS about -35 dBFS, in line with `plug-in-baby`. Level of the fuzz gate depends on input level (notes in the preset files).
