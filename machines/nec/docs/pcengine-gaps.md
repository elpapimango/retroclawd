# PC Engine / TurboGrafx-16: what is not done

Status 2026-11-22: HuCards boot and play their title screens; of 84 games sampled across the owner's 417 TurboGrafx-16 files (every 5th,
`scenarios/sweep.py --step 5`, picture at frame 600) 68 agree with Beetle PCE Fast within a mean luminance difference of 8 and most within 1. The 16 that
do not are listed below. Everything here is open.

## Backup memory
Lady Sword (RomM 7347, a 1 MB card) shows "no backup memory" at start-up and the reference goes on to the title. The card is
meant to be used with a backup unit (Tennokoe-style memory or the System Card's), which is not emulated: banks `$F7` (2 KB of backup RAM
behind a lock) and the unit's registers in `$1800-$1BFF` read `$FF`. Manifest scenario `G-ladysword-600` is a known gap; it will start
passing when a backup RAM bank exists.

## CD-ROM² and Super CD-ROM²
Not emulated: the CD interface, the ADPCM chip, the CD-DA player and the System Card. The owner's RomM server holds no System
Card (firmware) and no game for the CD platforms (134 has no entries), so nothing could be tried. HuCards need no firmware.

## SuperGrafx
Not emulated: the second video controller (HuC6270), the video priority controller (HuC6202) and 32 KB of RAM. The RomM server has no SuperGrafx
games (platform 124 is empty).

## Timer counter polling
Battle Royale (RomM 7178) seeds its random numbers by reading the timer counter in a loop until it is non-zero, with a reload-0 timer running. The reference stores `$7F` within the first frame; here the counter reads 0 for ever and the game waits. The timer's sequence matches the reference on four test cards (`scenarios/build.py` `pce-timer*.pce`) so the difference is in the instant of the underflow and the read (fact M3).

## Games that do not match yet (sweep of 84, frame 600)
Cause unknown unless stated; each was compared with the reference by work-RAM dumps at the same frames (`probe` `RAMOUT`, `libretro_run.py --ram-at`).
- Ours stays black or at an earlier screen while the game's zero page keeps pace with the reference (rendering or VRAM-loading path): Turrican (7540: the game
  clears VRAM from `$4400` up at 512 words every two frames and has not drawn by frame 600; the reference draws at frame 100), Kore ga Pro Yakyuu '90 (7341),
  Cyber-Core (7226, draws but dark), Heavy Unit (7307), Jack Nicklaus' Golf (7318), Bullfight Ring no Hasha (7204), Walkure no Densetsu (7542),
  Aoi Blink (7167), Toy Shop Boys (7535), Momotaro Densetsu II (7374), a 'Download' card (7249).
- Ours waits for an interrupt-driven counter that stops advancing after the first timer interrupt: Drop Off (7257: `$25` stays 1, the reference reaches 58 by
  frame 60; the timer counts and underflows here, the cause is not found). Battle Royale (7178): the timer-counter seed loop above.
- Close but a different animation phase or small positional difference (mean 9-14): Galaga '90 (7290), TV Sports Hockey (7512), After Burner II (7160, 36: a
  different frame of the scene).

## Other
- **Populous** (a 256 KB card with 32 KB of battery RAM at banks `$40-$43`) and other cards with save RAM have no RAM bank.
- **Sprite collision** is raised for sprite 0 only; the reference was not probed for the other pairs (fact V10).
- **VRAM DMA and the sprite DMA** finish at once (their real duration is a few hundred cycles); a game that polls the busy bit sees it clear.
- **Display start and end** (`HSR`, `HDR` bits 8-14) are not used: the picture always starts at the first dot and is cut to the dot clock's nominal
  width. A game that centres a narrow picture in the border is drawn at the left. CG mode (`MWR` bit 7) is ignored.
- **Mid-line effects**: palette, scroll and register writes take effect at the next line, never mid-line.
- **Frame-rate dependent games**: the frame length follows the video controller's registers (fact V6) but the machine always reports 59.83 Hz to the
  host's pacing.
- **Sound**: noise period, the LFO and the direct D/A path were written from the documentation and are not compared with the reference (fact P6);
  the overall level is calibrated on one channel (P5). Game music through a mixed set of channels was compared only by level.
- **Multitap**: pads 2-5 are read only with the `multitap` option; the second pad's keys are named `Joy2*`.
- **Region**: bit 6 of the I/O port's polarity is a guess (fact M5); the `console` option selects it and no card seen cares.
- **Tools**: retroclawd-tools 0.2 has the HuC6280 (`rcasm`, `rcdis`) and `rcld` writes HuCards; the test cards are built by them (`scenarios/cards/`).
- **Save-state compatibility**: states are bincode of the whole machine (`RCPC` v1); there is no versioned migration yet.
