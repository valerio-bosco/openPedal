# Tone Lab: a preset from a song

Tone Lab listens to a song (or a voice memo of a riff), finds the guitar, and builds an openPedal
preset that sounds like the part. You pick the part, the rest is automatic.

For setup and the commands in detail, see [the developer README](../tools/tonelab/README.md).

## The flow

1. **Get the song.** Start with a file: an mp3, a wav, or a phone voice memo. Links need the YouTube
   option (see the README). Run:

   ```
   bin/tonelab run ~/Music/song.mp3 --title "Song"
   ```

   It prints the job id at the end.

2. **Wait for the separation.** Demucs splits the song into stems (guitar, bass, drums, vocals, piano,
   other). A guitar-only file is not split again.

3. **Read the song map.** `run` prints a table of sections with the guitar in them. Each section has a kind:
   - **centre**: one guitar, in the middle. Easy: use the mid channel.
   - **double**: the same part played twice, one take left and one right. Pick one side.
   - **wide**: a spread guitar sound, or a stereo effect. Hard to match, so pick another section if you can.
   - **none**: no guitar here.

4. **Choose the part.** Choose a section by time and side:

   ```
   bin/tonelab parts <job> --start 8 --end 28 --channel mid --label riff
   ```

   For a double-tracked part, use `--channel left` or `--channel right`. This creates `p1`. Use a new part
   for each guitar.

5. **Give the fit a DI.** The DI is the clean guitar signal the fit plays through the presets. Two options:
   - **Record your own take (best).** Your guitar, your interface, so the result fits your rig. Play the part
     along with `parts/p1/target.wav` and record it with openPedal:

     ```
     openpedal content/presets/clean.toml --record take.wav --seconds 20
     ```

     The left channel is the dry guitar, which is what the fit uses. The recording must be 48 kHz.
   - **Synthetic DI.** No recording needed:

     ```
     bin/tonelab synth-di <job> --part p1
     ```

     For a better match, give it a short recording of your own guitar: `--match-di take.wav`.

6. **Run the fit.** It tries each template (chain) and keeps the best one. The Cryogen riff took about 76 seconds:

   ```
   openpedal tone-match <job-dir> --part p1 --di take.wav
   ```

   Leave out `--di` to use the synthetic DI. The summary lists the templates and the best one.

7. **Compare A/B.** Listen to the two files back to back:
   - `parts/p1/target.wav`: the song's guitar part
   - `parts/p1/fit/render.wav`: your DI through the fitted preset

   If the fit is close, save it. If it is not, try a different section or a different template (`--template`).

8. **Save the preset.** The fit writes `parts/p1/fit/preset.toml`, a normal openPedal preset. Copy it into
   `content/presets/` with a name you like, then check it:

   ```
   openpedal check content/presets/my-tone.toml
   ```

## Tips

- **Pick a section where the guitar is exposed.** A riff with no drums or vocals on top works best. Use the
  kind column to find centre parts.
- **10-20 seconds is enough.** Longer parts do not make the fit better and take longer.
- **Set the Scarlett gain so the peaks stay around -12 dBFS.** Too hot and the fuzz and drive clip in the
  recording; too quiet and the noise dominates.
- **Double-tracked parts: pick the left or the right side.** Both sides play the same notes. One side is enough.
- **Read the numbers, not only your ears.** Lower is better. The `holdout` and `validation` numbers are the
  honest ones; see the README for what they mean.

## Limits

- **The studio mix colours the result.** Reverb, compression and EQ on the record end up in the preset.
  The reverb is fitted, but the rest is not removed.
- **Modulation and delay are not detected yet.** A phaser, chorus or delay on the song will not be copied.
- **Two guitars in the same place at the same time cannot be separated.** Pick a section where only one
  guitar plays.
- **The octave-down hint may miss.** The song map does not always show that a guitar goes an octave lower.
  The `fuzz-octave` template is tried anyway, so the fit still finds it when it fits better.

## Example: Muse "Cryogen" (riff)

From the research and tests in [`docs/tones/cryogen.md`](tones/cryogen.md) and the Tone Lab plan:

- Part: the main riff, 8-28 s, mid channel.
- DI: a synthetic DI, matched to a real DI of the guitarist.
- Result: the fit chose `fuzz-octave` (an octave down, then the fuzz). Error on the fit window: **1.89 dB**.
- Validation on the real DI, which the fit never saw: **5.33 dB**. The hand-made preset from earlier
  reached 5.9 dB on the same check.
- The whole fit took about 76 seconds.

The 5.33 dB is the number to trust: it is measured on audio the fit did not use.
