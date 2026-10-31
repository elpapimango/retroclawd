# Known gaps of the Neo Geo machine (2026-10-31)

- **Horizontal shrink pattern.** The vertical shrink uses the real table (`000-lo.lo`); the 16-pixel columns kept by a horizontal shrink are spread
  evenly, not the hardware's pattern (fact N36). Only matters for a sprite narrower than 16 pixels.
- **Frame timing.** The game's own clock starts a few frames from FinalBurn Neo's (the plane of Aero Fighters 2 is about 4 pixels apart at
  frame 1200), so S3 compares the still part of that picture.
- **Sound.** The YM2610 makes FM (four channels, computed sine and envelope: no detune, LFO, SSG-EG or channel 3 split), the SSG,
  six ADPCM-A channels and ADPCM-B, and the timers; the Z80 runs the BIOS's and the cartridge's drivers. Compared with FinalBurn Neo on
  Cyber-Lip (RMS of the left channel): the BIOS jingle 546 against 528 (3-8 s), the title and attract music 1255, 1629, 984 against
  1548, 1883, 1218 (10-20, 25-35, 40-50 s): about 80 percent, peaks 7-8 thousand against 12-13 thousand. The envelope rates, the
  ADPCM-A level law and ADPCM-B are `[H?]` (facts Y5-Y7); the scenario S4 only holds the BIOS's silence at start-up.
- **Raster effects.** The timer interrupt (level 2) is raised (fact N41) but no game with a visible split screen has been compared with
  the reference yet; the line of the vertical blank interrupt and of the blank reload are `[H?]`.
- **FM details.** No detune, LFO, SSG-EG or separate frequencies for channel 3, and the envelope is computed, not the chip's rate tables
  (facts Y6): their tables were not available as facts, and without a way to compare note by note with the reference they were not guessed.
- **Sweep of 16 sets (2026-10-31, 1500 frames, headless).** 14 run to their attract mode with no stop. Stops: Fatal Fury 2 writes
  `$5555` to `$255552` (a protection chip in the cartridge, not modelled) and V-Liner (a gambling set that is not a normal cartridge) reads
  `$200000`. Defect: Art of Fighting draws its cutscene with the background gone and blocks of wrong tiles with hex digits (the fix layer is
  right); the sprite chains are 20 columns by 16 tiles; not yet understood (not auto-animation, not the shrink). The other pictures were
  judged by eye against the reference (Baseball Stars 2, Aggressors of Dark Kombat differ only in the moment of the transition).
- **uPD4990A clock** is a fixed one-second pulse; no time or date is kept.
- **Save states, backup RAM and memory card files** are not written (the backup RAM is lost at exit).
- **Controllers**: one pad and the coin/start buttons are wired; the arcade `Select` and the second player's keys have no default host keys.
