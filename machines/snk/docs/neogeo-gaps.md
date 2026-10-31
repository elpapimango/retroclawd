# Known gaps of the Neo Geo machine (2026-10-31)

- **Horizontal shrink pattern.** The vertical shrink uses the real table (`000-lo.lo`); the 16-pixel columns kept by a horizontal shrink are spread
  evenly, not the hardware's pattern (fact N36). Only matters for a sprite narrower than 16 pixels.
- **Frame timing.** The game's own clock starts a few frames from FinalBurn Neo's (the plane of Aero Fighters 2 is about 4 pixels apart at
  frame 1200), so S3 compares the still part of that picture.
- **Sound.** The Z80 runs the BIOS's and the cartridge's sound programs and answers the 68000, but the YM2610 only stores its registers
  (no timers, no FM, no SSG, no ADPCM) and the audio stream is empty.
- **Raster effects.** The timer interrupt (level 2) is not raised: games that use it for split screens will not show them. The
  interrupt line of the vertical blank is `[H?]`.
- **uPD4990A clock** is a fixed one-second pulse; no time or date is kept.
- **Save states, backup RAM and memory card files** are not written (the backup RAM is lost at exit).
- **Controllers**: one pad and the coin/start buttons are wired; the arcade `Select` and the second player's keys have no default host keys.
