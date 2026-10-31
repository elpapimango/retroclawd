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
- **FM details.** In: detune (fact Y9), the envelope rate law, the saturation of a channel's sum and the output scales (fact Y10). The levels
  now agree with FinalBurn Neo's within 7 percent in all four windows of Cyber-Lip (jingle 574 against 570, music 1518/1574, 1993/1865,
  1150/1225 RMS), and several partials agree within half a percent (197/196 Hz, 1753/1755, 519/518, jingle 390/391). Not done: the LFO
  (register `$22`, the AM and PM sensitivities), SSG-EG (registers `$90-$9F`) and separate frequencies for channel 3 (bits 6-7 of `$27`):
  Cyber-Lip writes `$22` three times at start-up and never uses the other two, so they could not be checked and their tables were not
  guessed. The envelope law and the detune table are from memory (`[H?]`); a game that uses the LFO or SSG-EG will sound different. The
  reference's bass partial at 76 Hz is still about three times ours and its high partials stronger, which the checks above did not explain.
- **Sweep of 16 sets (2026-10-31, 1500 frames, headless).** 14 run to their attract mode with no stop. Stops: Fatal Fury 2 writes
  `$5555` to `$255552` (a protection chip in the cartridge, not modelled) and V-Liner (a gambling set that is not a normal cartridge) reads
  `$200000`. Defect: Art of Fighting draws its cutscene with the background gone and blocks of wrong tiles with hex digits (the fix layer is
  right); the sprite chains are 20 columns by 16 tiles; not yet understood (not auto-animation, not the shrink). The other pictures were
  judged by eye against the reference (Baseball Stars 2, Aggressors of Dark Kombat differ only in the moment of the transition).
- **uPD4990A clock** is a fixed one-second pulse; no time or date is kept.
- **Save states, backup RAM and memory card files** are not written (the backup RAM is lost at exit).
- **Controllers**: one pad and the coin/start buttons are wired; the arcade `Select` and the second player's keys have no default host keys.
- **Resolved 2026-10-31 (second round).** Art of Fighting's defect was the C ROM layout: four 2 MiB chips with a small program are two
  halves each (fact N46); Fatal Fury 2's protection answers the challenges its code makes (fact N45); tile numbers past the ROM wrap
  (fact N43). Art of Fighting now matches the reference (mean difference 5.2 at frame 1500) and Fatal Fury 2 draws its intro.
  Remaining of the 16-set sweep: V-Liner (a gambling set) stops; Fight Fever runs here but the reference does not (a missing file).
- **Metal Slug 2 (RomM 7062, 2026-10-31).** Its stop at `$2FFFF0` was the program's bank register (fact N47, done: a program over 2 MiB is
  banked, and the `.sp2` second program file is read). The game then shows its "WARNING: this ROM cartridge is for use with original SNK
  boards only" screen. It is **not** a NEO-CMC game (the set is plain): the program takes the path to WARNING when the BIOS country is 2
  (`sp-s2`, the first BIOS tried) and the MVS flag is set and the word at `$D00100` is non-zero; with the USA BIOS (`sp-u2.sp1`) it
  proceeds. Why the order differs from FinalBurn Neo (which BIOS it defaults to, whether its backup RAM starts different) is open.
- **KOF 2001 against FinalBurn Neo (2026-10-31).** The game's frame counter (`$10008A`) advances one per frame in both, so the attract loop runs
  at the same speed; only the start differs: ours reaches the same picture **214 frames earlier** (the reference's boot is longer; cause not
  found, Cyber-Lip differs by about 15). With that offset the pictures agree (mean difference 1.5-2.7 at four of four frames). The game's main
  loop waits for the vertical blank flag at `$2785(a5)` (one tick per blank, no timer interrupt used). The backdrop was read from colour 0 of
  palette 255 instead of colour 15 (fact N29), which showed in the game's fade to white: fixed. Sound level is 0.95-1.3 times the reference's.
- **Boot offset, what is known (2026-10-31).** The reference boots **twice** for exactly the games whose backup RAM word `$D00100` is 1 after
  our run: KOF 2001 and Metal Slug 2 (the BIOS's RAM-test garbage at frames 120-240, then colour-fill test screens and the same garbage again
  at 360-450, then the game); Cyber-Lip, Fatal Fury 2, Art of Fighting 2, Zupapa!, Ganryu, Sengoku 3, Nightmare in the Dark and Aero Fighters 2
  boot once in both and have `$D00100` = 0 here. It is not the CMC (the CMC games boot once) and not the Neo-Geo mode option (MVS/AES alike).
  The two games run their MVS save routine (Metal Slug 2 `$87240`: clear the flag at its start, set it at its end) during start-up, and
  the same game code shows its WARNING screen when the flag is set on an MVS board (`$964`); the reference's backup RAM at frame 900 has the
  flag 0 and the date `26/10/31`. Why the reference reboots and ends with the flag clear is not found: next step would be to trace the
  reference with its debug build, but that is a Windows program with no headless mode, so it was not used. What the reference's work RAM
  shows frame by frame (KOF 2001): the address-pattern fill of the BIOS RAM test (frames 120-246), a clear, the BIOS variables set
  (`$10FD80`: MVS flag, country 2), then a **second RAM test with the fill values `$55` (262-277), `$AA` (278-285) and the address pattern again
  (286 on)**, and only then the game; ours goes from the first fill and clear straight to the game. So the BIOS code takes a longer test
  path in the reference, decided by something it reads after setting its variables (not the BIOS file: ours with `sp-s2`, `sp-s3` and
  others does not take it, and the stacked return addresses of the reference match `sp-s3` closely but not exactly). Open. Probe aids: `RC_COUNT_PC`, `RC_PCHIST`, `RC_DUMP_WORK`, `RC_DUMP_BACKUP`.
- **NEO-CMC (2026-10-31, fact N49, documented exception).** Sprite decryption (CMC42 and CMC50), the fix layer taken from the sprite data
  and the CMC50 Z80 program decryption are in for five sets (Zupapa!, Ganryu, Nightmare in the Dark, Sengoku 3, KOF 2001): all draw their
  title or attract screens. Not yet: a comparison with FinalBurn Neo's pictures and sound, the other keys (KOF 99/2000/2002/2003, Garou,
  Metal Slug 3-5, Samurai Shodown 5, ...) and the extra chips those boards also carry (SMA, PCM2, PVC, bootleg handlers).
- **SSG-EG and LFO use.** Eleven of the thirteen sets checked write the SSG-EG registers (`$90-$9F`) a few times each at start-up (init
  writes) and four write the LFO register `$22` only at start-up; none was seen using them in music, so they remain unimplemented.

