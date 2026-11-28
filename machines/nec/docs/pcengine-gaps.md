# PC Engine / TurboGrafx-16: what is not done

Status 2026-11-22: HuCards boot and play their title screens; of 84 games sampled across the owner's 417 TurboGrafx-16 files (every 5th,
`scenarios/sweep.py --step 5`, picture at frame 600) 68 agree with Beetle PCE Fast within a mean luminance difference of 8 and most within 1. The 16 that
do not are listed below. Everything here is open.

## Backup memory: done (2026-11-27)
The backup unit is in (facts M9, M10): 2 KB at bank `$F7` behind the `$1807`/`$1803` lock, a newly formatted RAM when there is no file,
option `backup_ram` (default `saves/pcengine-backup.bin`; `none` keeps no file, `off` removes the unit), written at exit when it changed, part
of save states (format `RCPC` 2; a format 1 state is refused with a message). Lady Sword (RomM 7347) now saves its record and reaches its title:
after 600 frames its backup RAM equals the reference's byte for byte and the picture is 0.18 apart; manifest scenario `G-ladysword-600` passes.
Not known: what `$1800-$1BFF` read back with only a backup unit attached (`$FF` here).

## CD-ROM² and Super CD-ROM²
Not emulated: the CD interface, the ADPCM chip, the CD-DA player and the System Card. The owner's RomM server holds no System
Card (firmware) and no game for the CD platforms (134 has no entries), so nothing could be tried. HuCards need no firmware.

## SuperGrafx: in (2026-11-27), checked with a test card only
Option `supergrafx` (facts S1-S3): the second video controller, the priority controller (modes, windows, the ST target) and 32 KB of RAM. The
RomM server has no SuperGrafx games (platform 124 is empty), so it was checked with the `sgx` test card (`scenarios/cards/sgx.asm`) against
Beetle PCE (the accurate core; Beetle PCE Fast does not run a card as a SuperGrafx): five manifest scenarios (`SGX-*`, mean difference
0.3-1.8). Not checked: games, mid-line changes of the priority controller, what `$0018-$001F` and the controller registers read back, timing
differences between the two controllers. The machine is chosen by the option, not by the `.sgx` file name.

## Games that do not match yet (sweep of 84, frame 600; 2026-11-28: 83 of 84 within 8, was 68 on 2026-11-27 morning)
Fixed 2026-11-27, each with the fact it needed: Battle Royale's timer seed loop (M11), narrow pictures centred (C5: Galaga '90, TV Sports
Hockey, Toy Shop Boys), the picture window fixed in the frame (C3: Heavy Unit, Jack Nicklaus' Golf, Aoi Blink), the vertical blank of a
picture that runs past the frame's end (V13: Drop Off, Bullfight Ring no Hasha), VRAM writes past `$7FFF` lost instead of wrapped (V14:
Turrican), the sprite table transfer's end flag some lines after the blank (V15: Kore ga Pro Yakyuu '90, Walkure no Densetsu) and the CPU's
port buffer (M12: the 'Download' card, Cyber-Core). Fixed 2026-11-28: After Burner II's crash (an interrupt handler that unmasks itself
nested until the stack wrapped; M13: a mask write reaches the interrupt decision one instruction late) and its frame length (V16: never
shorter than the colour encoder's); Momotaro Densetsu II's one-frame lead (C6: the first vertical blank after power-on is 259 lines
away). Measured with test cards in both and added: the extra cycle of every access to the video chips (M14, `vdcwait`). Debugging aids:
probe `VRAMOUT`, `libretro_run.py --vram-at`, the `timerpoll`, `hpos`, `vdcwait`, `vbphase` and `vbslow` test cards. Still different:
- Neutopia II (7395): the same fade 46 frames later. Not a defect here: its start-up delay loop runs in slow mode, which the reference
  does not slow down (fact H5); without the extra video cycle of M14 it was 35 frames later and just within the threshold.
- Battle Royale's demo goes another way (its random seed comes from the timer; the timer's first interval after a start differs: 480
  polls there, 660 here on the `timerpoll` card; a start phase of 3060 master clocks gives 480 but breaks the `timer5` cards, so the
  cause is something else), within the threshold.
- V15's 4 lines are a guess.

## Other
- **Populous** (a 256 KB card with 32 KB of battery RAM at banks `$40-$43`) and other cards with save RAM have no RAM bank.
- **Sprite collision** is raised for sprite 0 only; the reference was not probed for the other pairs (fact V10).
- **VRAM DMA and the sprite DMA** finish at once (their real duration is a few hundred cycles); a game that polls the busy bit sees it clear.
- **Display start and end** (`HSR`, `HDR` bits 8-14) are not used for the position, as in the reference (fact C5); a picture narrower than the
  window is centred. CG mode (`MWR` bit 7) is ignored.
- **Mid-line effects**: palette, scroll and register writes take effect at the next line, never mid-line.
- **Frame-rate dependent games**: the frame length follows the video controller's registers (fact V6) but the machine always reports 59.83 Hz to the
  host's pacing.
- **Sound**: noise period, the LFO and the direct D/A path were written from the documentation and are not compared with the reference (fact P6);
  the overall level is calibrated on one channel (P5). Game music through a mixed set of channels was compared only by level.
- **Multitap**: pads 2-5 are read only with the `multitap` option; the second pad's keys are named `Joy2*`.
- **Region**: bit 6 of the I/O port's polarity is a guess (fact M5); the `console` option selects it and no card seen cares.
- **Tools**: retroclawd-tools 0.2 has the HuC6280 (`rcasm`, `rcdis`) and `rcld` writes HuCards; the test cards are built by them (`scenarios/cards/`).
- **Save-state compatibility**: states are bincode of the whole machine (`RCPC` v1); there is no versioned migration yet.
