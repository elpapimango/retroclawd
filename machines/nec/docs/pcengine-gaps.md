# PC Engine / TurboGrafx-16: what is not done

Status 2026-11-22: HuCards boot and play their title screens; 27 of 30 games sampled across the owner's 417 TurboGrafx-16 files agree
with Beetle PCE Fast within a mean luminance difference of 8 (most within 1). Everything below is open.

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
- **Tools**: `rcasm`, `rcdis` and `rcld` have no HuC6280 target; the test cards are assembled by `scenarios/build.py`.
- **Save-state compatibility**: states are bincode of the whole machine (`RCPC` v1); there is no versioned migration yet.
