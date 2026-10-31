# Known gaps of the Neo Geo machine (2026-10-31)

- **Sprite shrinking.** SCB2 (`$8000 + n`) holds a horizontal shrink (bits 11-8, 15 = full width) and a vertical shrink (bits 7-0,
  255 = full height) for every sprite; the renderer draws every sprite at full size. The S3 scenarios of Fatal Fury and Aero Fighters 2
  (`neogeo.manifest.toml`) differ from FinalBurn Neo's pictures by 11.6 and 13.6 (0 to 255, mean over a 64x48 luminance grid) because
  their logos and scrolling pictures are built from shrunk sprites. The Cyber-Lip title screen (full-size sprites and the fix layer) matches
  (S1, under 6). Needs the horizontal shrink pattern and the vertical table (`000-lo.lo` in the system set) as facts.
- **Sound.** The Z80 runs the BIOS's and the cartridge's sound programs and answers the 68000, but the YM2610 only stores its registers
  (no timers, no FM, no SSG, no ADPCM) and the audio stream is empty.
- **Raster effects.** The timer interrupt (level 2) is not raised: games that use it for split screens will not show them. The
  interrupt line of the vertical blank is `[H?]`.
- **uPD4990A clock** is a fixed one-second pulse; no time or date is kept.
- **Save states, backup RAM and memory card files** are not written (the backup RAM is lost at exit).
- **Controllers**: one pad and the coin/start buttons are wired; the arcade `Select` and the second player's keys have no default host keys.
