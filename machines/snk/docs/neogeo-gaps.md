# Known gaps of the Neo Geo machine (2026-10-31)

- **Horizontal shrink pattern: in (2026-11-27, fact N57).** The columns a shrunk sprite keeps follow the wiki's table instead of being
  spread evenly. Open: whether the pattern applies before or after a horizontal flip (`[H?]`, taken before here).
- **Frame timing.** The game's own clock starts a few frames from FinalBurn Neo's (the plane of Aero Fighters 2 is about 4 pixels apart at
  frame 1200), so S3 compares the still part of that picture.
- **Boot offset: found, left as it is (2026-11-27).** The 14 frames this machine is ahead of FinalBurn Neo on Cyber-Lip come almost all from one
  loop: the BIOS (`sp-s2`, ver. 6, the reference's default too) sums its own 128 KiB at `$C11C4E` (`move.b d0,$300001` / `add.b (a0)+,d0` /
  `subq.l #1,d7` / `bne`, 131 070 times, 42 cycles each). Work RAM is equal in both up to frame 207, where the loop starts; it ends at frame
  234 line 79 here (27.2 frames, as 42 cycles a byte gives) and between frames 246 and 248 in the reference (39-41 frames, 60-63 cycles a
  byte). The wiki's memory map gives the system ROM and the I/O area (`$300001`) no wait cycles (fact N58), so this machine keeps the
  documented timing and the reference's longer loop stays unexplained. Probe aid: `RC_DUMP_EVERY=DIR:N`.
- **Sound.** The YM2610 makes FM (four channels, computed sine and envelope: no detune, LFO, SSG-EG or channel 3 split), the SSG,
  six ADPCM-A channels and ADPCM-B, and the timers; the Z80 runs the BIOS's and the cartridge's drivers. Compared with FinalBurn Neo on
  Cyber-Lip (RMS of the left channel): the BIOS jingle 546 against 528 (3-8 s), the title and attract music 1255, 1629, 984 against
  1548, 1883, 1218 (10-20, 25-35, 40-50 s): about 80 percent, peaks 7-8 thousand against 12-13 thousand. The envelope rates, the
  ADPCM-A level law and ADPCM-B are `[H?]` (facts Y5-Y7); the scenario S4 only holds the BIOS's silence at start-up.
- **Raster effects.** The timer interrupt (level 2) is raised (fact N41); the line of the vertical blank interrupt and of the blank reload
  are `[H?]`. Which sets use it (2026-11-27, 3000 frames of each of 23 sets): Metal Slug 2 (about 29 a frame), 8 Man, Aggressors of Dark
  Kombat, Art of Fighting and Double Dragon; the other 18 never enable it. 8 Man and Metal Slug 2 compared with FinalBurn Neo at frames 1200,
  1800 and 2400: no broken bands or misplaced splits in either; what differs is frame timing (blinking text, the logo animation).
- **Burning Fight's stop at frame 1527: resolved 2026-11-27 (fact N55).** It read `$3689C2`, in a range no chip decodes; the read now
  answers the last word on the bus and the game runs on (4000 frames). **Its attract sequence: not a defect (closed 2026-11-27).** This
  machine runs the game 14 frames ahead of FinalBurn Neo (the boot offset below); compared at the same point of the game (our frame N
  against the reference's N + 13 or 14) the work RAM agrees at frames 1000-2950 except the main loop's busy flag `$10000E` and a sprite
  buffer toggle at `$106200` (where in the frame each dump is taken), and the pictures at 3000-3700 are the same (player introduction,
  subway demo, street stage). The lag frames agree too: the palette fade at `$3526` takes the main loop past the vertical blank, so the
  game's loop counter `$100010` advances every second frame in the escalator scene (1527-1640 here, 1544-1654 there) and stalls for two
  frames at 2776-2778 (2790-2792 there). The "dim background at frame 1600" was the fade 14 frames apart. Probe aid: `RC_STACK_AT=PC[:ADDR]`
  prints the interrupted program counter (and a RAM byte) whenever the 68000 reaches an interrupt handler.
- **FM details.** In: detune (fact Y9), the envelope rate law, the saturation of a channel's sum and the output scales (fact Y10). The levels
  now agree with FinalBurn Neo's within 7 percent in all four windows of Cyber-Lip (jingle 574 against 570, music 1518/1574, 1993/1865,
  1150/1225 RMS), and several partials agree within half a percent (197/196 Hz, 1753/1755, 519/518, jingle 390/391). **Exact envelopes: in (2026-11-28, fact Y13):** the integer OPN envelope generator (counter, rate steps, the attack curve, SSG-EG) replaced the averaged law; the match with FinalBurn Neo stays where it was (the spectra correlate 0.907, 0.941, 0.844 in the music windows), so the remaining difference is not the envelope. The LFO is in since 2026-11-27 (fact Y12). Not done: the
  separate frequencies for channel 3 (bits 6-7 of `$27`), which no set checked uses. The detune table is from memory (`[H?]`). The
  reference's bass partial at 76 Hz is still about three times ours and its high partials stronger, which the checks above did not explain.
- **Sweep of 16 sets (2026-10-31, 1500 frames, headless).** 14 run to their attract mode with no stop. Stops: Fatal Fury 2 writes
  `$5555` to `$255552` (a protection chip in the cartridge, not modelled) and V-Liner (a gambling set that is not a normal cartridge) reads
  `$200000`. Defect: Art of Fighting draws its cutscene with the background gone and blocks of wrong tiles with hex digits (the fix layer is
  right); the sprite chains are 20 columns by 16 tiles; not yet understood (not auto-animation, not the shrink). The other pictures were
  judged by eye against the reference (Baseball Stars 2, Aggressors of Dark Kombat differ only in the moment of the transition).
- **uPD4990A clock** is a fixed one-second pulse; no time or date is kept.
- **Save states and the backup RAM file: done (2026-11-27).** Save states (`RCST`, `crates/machines/neogeo/src/state.rs`: both CPUs, the bus with
  the program ROM, the sound side and the YM2610; the BIOS and the S/C/V/M ROMs stay out and are identified by a CRC) resume exactly: Cyber-Lip
  saved at frame 900 and loaded shows the same picture at 1500 as the run that never stopped. The 64 KB backup RAM is the option `backup_ram`
  (default `saves/neogeo-backup.bin`, one file as one MVS board has one RAM; `none` for no file, which the scenario manifest uses so that its
  runs stay the same): read at start, written at exit when it changed. KOF 2001 formats it on the first run and reads it on the next.
  The **memory card** is in since 2026-11-27 (fact N56): option `memory_card` (a file of 2 to 16 KiB; a missing file is a new 2 KiB
  card), `memory_card_protect`, written at exit when it changed, part of save states. Checked: the BIOS probes a new card at start-up
  and Cyber-Lip runs on with it. Against FinalBurn Neo (2026-11-27; its option `fbneo-memcard-mode=per-game`, the card written to
  `<system>/fbneo/<set>.memcard` when `libretro_run.py --unload` unloads the game; the file is a 32-byte header and then one 16-bit word
  per card byte, the card byte in the low half): a new card, Cyber-Lip, a coin at frame 1500, start at 1600 and no input until the game
  is over and the attract mode is back gives the same 2 KiB in both, byte for byte. That content is the BIOS's probe pattern (each pair
  of bytes the big-endian offset times two); neither machine saves anything for Cyber-Lip in that run. Not checked: a game that saves
  to the card and loads from it (none found yet), and the banks of cards over 2 KiB (REG_CRDBANK's layout is not documented).
- **Controllers**: one pad and the coin/start buttons are wired; the arcade `Select` and the second player's keys have no default host keys.
- **Resolved 2026-10-31 (second round).** Art of Fighting's defect was the C ROM layout: four 2 MiB chips with a small program are two
  halves each (fact N46); Fatal Fury 2's protection answers the challenges its code makes (fact N45); tile numbers past the ROM wrap
  (fact N43). Art of Fighting now matches the reference (mean difference 5.2 at frame 1500) and Fatal Fury 2 draws its intro.
  Remaining of the 16-set sweep: V-Liner (a gambling set) stops; Fight Fever runs here but the reference does not (a missing file).
- **Metal Slug 2 logo transition: not a defect (closed 2026-11-27).** The mirrored logo on white is the logo turning over (its back
  side, chain 1 below), and FinalBurn Neo draws it too: 8 Man's end-of-attract logo, captured every 20 frames in both (2300-2560), shows the
  same sequence (mirrored and shrinking on white, then the grey fade, then the plain logo and "MAX 330 MEGA"), the reference about 20 frames
  behind this machine. The single frames compared before fell at different moments of that animation. The history:
  At the end of its attract loop (ours frames 3952-4060) the NEO-GEO logo is drawn mirrored on a white flash, where FinalBurn Neo shows a wide, squashed, un-mirrored logo on grey. The sprite RAM says the game shows chain 1 (sprites 1-18: tiles in descending order, hflip set, horizontal shrink 0 to 15) while chain 2 (sprites 19-36, ascending, unflipped) has height 0, and swaps them at about frame 4060; the renderer draws that data as written. Whether the reference hides chain 1 or the game takes another path there is not known. Probe aids: `RC_SPRITES=1`, `RC_SPRTRACE=<frame>`.
- **Resolved 2026-11-01: the boot offset and Metal Slug 2's WARNING screen (fact N51).** Both had one cause: the watchdog reset was not modelled. The BIOS formats an unformatted backup RAM and lets the watchdog reboot it; FinalBurn Neo therefore boots KOF 2001 and Metal Slug 2 twice, and the second boot leaves the flag at `$D00100` clear. Now exactly those two sets reset once here, Metal Slug 2 reaches its title and KOF 2001 its attract mode. The notes below on both are kept as the history of the search (the 214-frame offset should be re-measured; the clock length of the watchdog is `[H?]`).
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
- **Metal Slug 3, 4 and 5 (RomM 7063-7065, 2026-11-01, facts N52, N53, second documented exception).** All three showed the BIOS test grid or stopped:
  the program of Metal Slug 3 (SMA chip, with the `neo-sma` file) and Metal Slug 5 (PVC chip, the two program files woven by words) is
  scrambled, Metal Slug 4 and 5 have a NEO-PCM2 sample ROM, and Metal Slug 4 writes into program ROM. Now all three run their attract demos
  for 8000 frames; Metal Slug 5 matches FinalBurn Neo's scenes (frames 1500, 2500). Metal Slug 4's fix layer text was wrong
  (copyright lines, "INSERT COIN", HUD); fixed (fact N54) by taking the glyphs from the 128 KiB that starts 384 KiB before the end of the
  sprite data, the one place that matches FinalBurn Neo's screen (the reference's own rule for the board's 512 KiB fix region is not
  understood: the first and last 128 KiB of that region show other data). Open: the sound of the three games is not
  compared (the PCM2 descramble is unverified by ear), and the PVC chip's other functions are not modelled beyond what Metal Slug 5 uses.
- **NEO-CMC (2026-10-31, fact N49, documented exception).** Sprite decryption (CMC42 and CMC50), the fix layer taken from the sprite data
  and the CMC50 Z80 program decryption are in for five sets (Zupapa!, Ganryu, Nightmare in the Dark, Sengoku 3, KOF 2001): all draw their
  title or attract screens. Not yet: a comparison with FinalBurn Neo's pictures and sound, the other keys (KOF 99/2000/2002/2003, Garou,
  Metal Slug 3-5, Samurai Shodown 5, ...) and the extra chips those boards also carry (SMA, PCM2, PVC, bootleg handlers).
- **Sound level (2026-11-01, fact Y11).** ADPCM-B was half as loud as FinalBurn Neo's; fixed, Metal Slug 4's music now agrees within 2 percent in RMS. Other ADPCM-B users were not re-measured.
- **Auto-animation (2026-11-01, fact N34).** The attribute bits were swapped (bit 3 is the eight-frame animation, bit 2 the four-frame one): Metal Slug 3's underwater stage showed dark triangle blocks in its rock background; now smooth like FinalBurn Neo's. Other sets were not re-compared by eye; the scenario manifest still passes.
- **Metal Slug 3 logo (2026-11-01, fact N54).** The "METAL SLUG 3" logo over the demos and the title screen's copyright and SNK logo are fix-layer tiles; they were blank because the fix glyphs were taken from the last 128 KiB of the sprite data instead of the block 384 KiB before the end (as for Metal Slug 4). Fixed; the logo matches FinalBurn Neo. Other 64 MiB CMC games (KOF 99-2003, Garou, ...) probably need the same check when they are added.
- **LFO: in (2026-11-27, fact Y12); SSG-EG: not used.** A survey of 23 sets over 4000 frames (the probe's `RC_YM` line "fm features
  switched on") found the LFO switched on by Aggressors of Dark Kombat, Art of Fighting 1 and 2, Cyber-Lip, Fatal Fury and Fight Fever,
  and non-zero AM/PM sensitivities in many; no set switches SSG-EG on (bit 3 of `$90-$9F`), so it stays unimplemented. The LFO now moves
  pitch (PMS) and the level of operators with AM on (AMS); unit tests pin both. Against FinalBurn Neo on Cyber-Lip (the four windows
  above) it changes nothing measurable: the averaged-spectrum correlation goes from 0.9065 to 0.9068 in 10-20 s and stays the same
  elsewhere, as the LFO is on only briefly there. The waveforms are `[H?]`.

