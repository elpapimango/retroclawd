# Sega Mega Drive / Genesis: design

Status: **draft for owner approval**. No implementation code written. Date: 2026-10-24.

Brief: add the Sega Mega Drive / Genesis as a machine family plugin, built to run to a functioning emulator once started.
Accuracy first, speed second. Firmware policy follows Genesis Plus GX (optional TMSS boot ROM, lock-on cartridges).

## Claim labels

Same as the Atari ST work (`2026-10-19-atari-st-design.md`): **[R]** repository fact read on 2026-10-24; **[H]** hardware fact
from a document that is named; **[H?]** hardware claim from memory, never coded until a source is found; **[O]** observed in a
reference emulator by a probe; **[P]** proposal; **[D]** datasheet. Every hardware fact used goes into
`machines/sega/docs/hardware-facts.md` with its label and source. A register or address with no fact stops the machine with a
bounded report instead of returning an invented value.

## 1. Decisions

- **[P] New family repository `elpapimango/retroclawd-sega`** (submodule `machines/sega`), shaped like the other families.
  Removing `sega_machine_plugin.{dll,so,dylib}` removes Sega support. The host gets no Sega code, except the generic items in 5.
  The repository is created locally first. Creating it on GitHub and pushing is outward-facing and waits for the owner.
- **[P] Machine id `genesis`** (RomM slug `genesis`). One id, region by option, not three ids. Mega CD, 32X, Master System mode,
  Pico and the TeraDrive are non-goals of this design (section 7); the layout leaves room for them.
- **[P] Reference emulators: run as oracles and read for understanding, never copied.** Genesis Plus GX (non-commercial licence),
  PicoDrive, BlastEm (GPLv3) and clownmdemu (AGPLv3) are run through the oracle host, and their source may be read to learn how
  the hardware behaves (owner decision 2026-10-24). No source, table, constant or comment is copied or transliterated; a fact
  learnt from source is written in our own words, labelled, and confirmed by a document or a probe on two oracles before it is
  coded. Tables that real hardware derives from a formula (the YM2612 sine and exponent tables) are generated from the formula in
  a documented `const fn`, never pasted. Source is read from a clone in `software/refsrc/<name>/` (git-ignored).
- **[P] The reference oracle is a libretro host.** `retroclawd-tools/scripts/libretro_run.py` (Python `ctypes`) loads a core DLL,
  runs N frames with scripted input, and writes the frame (PNG), the audio (WAV) and the 68000 work RAM / Z80 RAM
  (`RETRO_MEMORY_SYSTEM_RAM`) to files. The cores are the ones installed in RetroArch (`C:\Users\matti\Workspace\Emulators\RetroArch-Win64\cores`: Genesis Plus GX v1.7.4, PicoDrive 2.05, BlastEm 1.0.1-pre, clownmdemu 1.6.12), loaded in place by path; their SHA-256 is recorded in the facts document. MAME's `genesis` driver, already installed in `Emulators/Mame`, is a fifth
  oracle for disagreements. Where two or more oracles agree and the documentation does not contradict them, that is a fact.
- **[P] Probe ROMs, as in Phase 3 of the Atari work.** Small 68000 (and Z80) programs built with `rcasm`, wrapped into a valid
  ROM by `scenarios/mkrom.py`, write their results to work RAM; the oracle and retroclawd must leave identical bytes.
  Only one retail game is on RomM (Altered Beast, `romm:6174`), so probes and public test ROMs are the main corpus. Test ROMs
  that are freely redistributable are fetched by hash into their own directory (`tools/fetch_diagnostics`), never committed.
- **[P] Timing model.** `clock_hz` is the **master clock** (NTSC 53 693 175 Hz, PAL 53 203 424 Hz, [H?] section 3), `step()`
  returns master clocks. The 68000 runs at master/7, the Z80 at master/15, the YM2612 at master/42 (68000 clock/6), the PSG at
  master/15, the VDP pixel clock at master/8 (H32) or master/10 (H40) [H?]. Time comes only from the scheduler (no wall clock,
  no floats in timing). `step()` executes one 68000 instruction; the Z80, VDP, YM2612 and PSG are brought up to the same master
  time lazily before every access that can see them (the Atari `events.rs` approach). The frame is `lines x 3420` master clocks
  [H?]: 262 lines NTSC, 313 PAL, reported exactly through `frame_rate`.
- **[P] Two CPUs need no ABI change.** Protocol 11 already has `MachineProfile::cpus` (`main` = 68000 first, `z80`), `OnCpu`,
  `addr_bits` per space and stereo `audio_format`. The plugin accepts both `main` and `z80` in `OnCpu`. The host debugger's
  per-CPU disassembly must follow the CPU addressed (section 5, item 3).
- **[P] Chips start inside the family** (`vdp-315-5313`, `ym2612`, `sn76489`) and are promoted to `retroclawd-crates` when a second
  consumer appears (the `wd1770` precedent). The SN76489 is the likely first (Master System). The 68000 and Z80 cores stay in
  `retroclawd-crates`; a core change needed here lands there as its own reviewed commit and pin bump.
- **[P] Firmware, following Genesis Plus GX.** `bios_MD.bin` (2048 bytes, SHA-1 `3f50b76b0529db7f79c396b5e808cc0786ffc311`,
  also on RomM as `tmss.md` / `GEN_boot.md`) is the optional TMSS boot ROM; GPGX checks for the text "GENESIS OS" at offset `$120` [R:
  retrobios page]. Without it the machine starts with the TMSS lock already open (what GPGX does). Lock-on firmware is
  optional and on RomM: `sk.bin` (2 MiB, `88d6499d874dcb5721ff58d76fe1b9af811192e3`), `sk2chip.bin` (256 KiB,
  `70429f1d80503a0632f603bf762fe0bbaa881d22`), `ggenie.bin` (32 KiB, `937e1878ebd104f489e6bdbc410a184f79f1144a`), `areplay.bin` (32 KiB,
  `1e0f246826be4ebc7b99bb3f9de7f1de347122e5`). Mega CD BIOSes (`bios_CD_*.bin`) are out of scope. GPGX's `rom.db` and `carthw.cfg`
  (on RomM) are its own mapper and game database: **not used** (we detect hardware by header and behaviour, and list known
  special cartridges ourselves from documentation).
- **[P] Region.** Option `console_region` = `auto | japan | usa | europe` (default `auto`, chosen from the ROM header's region field;
  the priority is our rule and is confirmed against the oracles). The host's `--region` (pal/ntsc/japan) maps to it when given.
  Version register `$A10001`: overseas flag, PAL flag, version nibble [H?].
- **[P] Input.** 3-button and 6-button pads on both ports (`Joy1Up` ... `Joy1Mode`, `Joy2*`, extra pad names with the host
  defaults of the existing `Joy{N}*` mechanism), mouse later, light gun and multitap non-goals. Option `pad1`/`pad2` =
  `three|six|none`.
- **[P] Save data.** Battery SRAM and EEPROM persist through a new generic host mechanism (section 5, item 2), `.srm` beside the ROM
  unless `--sram FILE`. Save states use the `RCST` envelope the Atari work built.

## 2. Hardware scope (one row = one slice of work)

| Block | Needed for | Accuracy target |
|---|---|---|
| MC68000 @ master/7, 24-bit bus | everything | existing core: SingleStepTests-exact, per-bus-cycle transactions |
| Z80 @ master/15, 8 KB RAM, bank window, bus request / reset | sound drivers, some games' main logic | instruction-exact, bus-contention with 68000 |
| VDP 315-5313: planes A/B, window, sprites (+ per-line limits), H32/H40, V28/V30, scroll modes, shadow/highlight, interlace 1/2, DMA (bus, fill, copy), FIFO, HV counter, interrupts, CRAM dots | all video | slot-accurate (Phase 3): FIFO, DMA stalls, mid-line writes |
| YM2612: 6 FM channels, 4 operators, LFO, SSG-EG, timers A/B, CH3 special mode, CSM, DAC, ladder effect (option) | music | sample-exact against a probe-validated model; cross-checked with two oracles |
| SN76489 variant (16-bit LFSR, Sega noise taps) | music | exact |
| I/O: version, 2 pad ports + expansion, TH multiplexing, 6-button state machine with timeout | input | exact against probe |
| TMSS: `$A14000` lock/unlock, boot-ROM overlay | boot ROM | boot with and without `bios_MD.bin` |
| Cartridge: plain up to 4 MB, SSF2 bank mapper, battery SRAM (8/16-bit), EEPROM (I2C 24Cxx), S&K lock-on, Game Genie | game compatibility | the list the corpus needs; each mapper a fact row |

## 3. Facts to source before coding them (all start as [H?])

Master clocks and dividers; 3420 clocks per line, 262/313 lines; H32/H40 active widths and pixel clocks; VBlank and HBlank
positions; Z80 INT pulse position and width; VDP register map and mode bits; DMA rates per mode and per H-mode; FIFO depth (4 words) and
its slot behaviour; sprite limits per line (80 sprites / 20 per line in H40, 64 / 16 in H32) and sprite pixel limit; the
YM2612 register map, phase/envelope generators, rates, key scaling; the PSG's noise feedback; pad TH protocol and timeouts; the
Z80 memory map; the VDP-in-Z80-space quirks; TMSS register. Sources to look for: Sega's published hardware manuals (via
segaretro.org), Charles MacDonald's VDP and I/O documents, the plutiedev site, SpritesMind threads (Nemesis' YM2612 and VDP
research), the MC68000 Programmer's Reference Manual already used. Each gets a row before it is coded.

## 4. Verification

1. Unit tests per chip against documented behaviour (including the 68000 SingleStepTests already in `retroclawd-crates`).
2. Probe ROMs (memory-dump exact) against two or more oracles; a disagreement between oracles is recorded and decided by
   documentation or by MAME as the tie-break.
3. Scenario manifest `machines/sega/genesis.manifest.toml` (S1 boot picture by similarity; S2 input; S4 audio tone/RMS; S6 frame
   length; save/restore) with goldens made by the oracle, never committed.
4. Altered Beast (RomM) boots, runs its attract mode, takes input, and its frames agree with Genesis Plus GX by the same similarity
   rule the Atari desktop used, with and without the TMSS ROM.
5. Fuzz targets (`mkrom`-built ROMs, random VDP command streams, random YM2612 register writes) for panics and hangs.

## 5. Generic host changes this needs (not ABI changes)

1. A `.md/.gen/.smd/.bin` ROM goes to the plugin as a cartridge (`MediaKind::Cartridge`) without extension special-casing in the host.
2. **Persistent save RAM**: `--sram FILE` read at start and written at exit and on a hotkey; one generic `MediaSlot::Other("sram")`
   with the export used by `--acsi-save`, not Genesis code in the host.
3. Debugger: the disassembly, assembler and symbol table follow the CPU addressed by `OnCpu` (today the host picks by the
   machine's primary `CpuFamily`). Memory view and registers per CPU.
4. Window pixel aspect and 320x224 / 256x224 / 320x240 size changes mid-run (Atari already switches size).

## 6. Exit criteria ("functioning emulator")

- Altered Beast plays with correct picture, input and music/effects, in NTSC and PAL, with and without `bios_MD.bin`.
- All probe ROMs either exact against the oracles or listed `known-gap` with the measured difference and a reason.
- Public test ROMs we can fetch run and their results are recorded (pass / documented difference).
- Save states, SRAM persistence, 6-button pad, two-CPU debugging work through the host; `retroclawd genesis --help` shows all options.
- `tools/check_all.sh` passes; `docs/benchmarks.md` has the speed figure; `TODO.md` lists what is parked and what each item needs.

## 7. Non-goals and parked

Mega CD, 32X, Master System / Game Gear mode of the VDP, Pico, TeraDrive, EverDrive-style mappers, Menacer / multitap / Team Player,
cheat-code loading, netplay, rewind. The corpus is one retail game, so mapper support beyond SSF2 and plain SRAM/EEPROM is
implemented from documentation and probes, and anything without a source is parked in `TODO.md`.

## 8. Risks

- **Test corpus is thin** (one retail ROM). Mitigation: probes, public test ROMs, and the oracles as the comparison.
- **The YM2612 is the largest fidelity risk** (die-level behaviour is documented by research, not by Sega). Mitigation: build the
  documented model, then compare sample streams from two oracles on probe-driven register sequences; record residual differences.
- **Mid-line VDP effects and DMA/FIFO timing** are what the accurate mode exists for; Phase 3 starts only after Phase 2 plays games.
- **Licence**: reference emulators are black boxes (section 1). Cores are downloaded binaries and are untrusted.
