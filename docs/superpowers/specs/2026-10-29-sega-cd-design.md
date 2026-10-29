# Sega CD / Mega CD: design

Status: **draft for owner approval**. No implementation code written. Date: 2026-10-29.

Brief: add the Sega CD (Mega CD in Europe and Japan) to the `retroclawd-sega` family, on top of the finished Genesis. Functional first,
accurate later (owner decision 2026-10-29). Disc formats: CUE/BIN, ISO and CHD (owner decision 2026-10-29). The graphics ASIC and the
FMV games come after the basics boot (owner decision 2026-10-29).

## Claim labels

Same as the Genesis design (`2026-10-24-sega-genesis-design.md`): **[R]** repository or server fact read on 2026-10-29; **[H]** hardware
fact from a named document; **[H?]** hardware claim from memory, never coded until a source is found; **[O]** observed in a reference
emulator by a probe; **[P]** proposal; **[D]** datasheet. Every hardware fact used goes into `machines/sega/docs/hardware-facts.md` with its
label and source, as rows continuing the Genesis ones. A register or address with no fact stops the machine with a bounded report. The
clean-room rules of the Genesis design apply unchanged (reference emulators are black boxes; no source, table or comment is copied).

## 1. Starting point [R]

- RomM platform `segacd` (id 109): **29 games**, listed as 53 entries: 24 games as a `.cue` entry plus a `.bin` entry (separate RomM entries, one BIN per game, so a disc has to be put together in one folder) and 5 games as a single `.chd` (checked 2026-10-29 by reading the server's file list and the first two sectors of a BIN: sync, header 00:02:00 mode 1, `SEGADISCSYSTEM`; 150-630 MB per game: After Burner III,
  Eye of the Beholder, Final Fight CD, Lunar, Silpheed, Snatcher, Shining Force CD, Corpse Killer, Mad Dog McCree, Demolition Man ...)
  and **37 firmware files**: 3 Sega CD BIOS generations in US, EU and JP form (`us_scd1_9210`, `us_scd2_9306`, `eu_mcd1_9210`,
  `eu_mcd2_9303`, `eu_mcd2_9306`, `jp_mcd1_9111`, `jp_mcd1_9112`, `jp_mcd2_921222`, ...), several duplicates under other names, plus
  backup-RAM files (`scd_U.brm`, `cart.brm`) and `.srm` saves of games. All BIOS files are 131072 bytes; they are matched by SHA-1.
- The Genesis plugin (`sega_machine_plugin`, machine id `genesis`) has the 68000 and Z80 cores, the VDP, YM2612, SN76489, the cartridge
  and the save-state envelope. Oracles that can run Sega CD: Genesis Plus GX, PicoDrive and clownmdemu (BlastEm cannot); MAME's `genesis`
  driver with the `segacd` slot is the tie-break. All are run as black boxes.

## 2. Decisions

- **[P] Machine id `segacd`** in the same plugin library (RomM slug), a second profile next to `genesis`. The Mega CD hangs on the
  Genesis cartridge port, so `segacd` reuses the Genesis crates (`genesis`'s bus, VDP, Z80, YM2612, PSG) and adds a second bus. How the
  Genesis `Bus` is opened up (an expansion hook for `$000000-$3FFFFF`, `$A12000` and the cartridge-port lines, versus a wrapper
  machine) is decided in slice 2 from what the code needs; the hook, if any, is a family-internal change, not an ABI change.
- **[P] New crates in the family** (`retroclawd-sega`): `crates/formats/disc` (CUE/BIN, ISO, CHD readers, sector addressing, TOC,
  subcode-Q), `crates/chips/rf5c164` (the PCM chip), `crates/chips/lc89510` (the CD controller and its host/sub-CPU DMA paths),
  `crates/chips/mcd-gate-array` (main/sub registers, Word RAM, PRG RAM banking, timers, interrupts), `crates/chips/mcd-asic` (stamp
  and rotation, later), `crates/machines/segacd`. The CD drive's microcontroller (CDD) is modelled at its command/status protocol
  level, inside the gate-array or its own small crate when its size justifies it. The 68000 core is the shared `mc68000`, used a
  second time (the sub-CPU, 12.5 MHz [H?]).
- **[P] Disc hand-off without an ABI change: by path, not by bytes.** Discs are hundreds of megabytes; the plugin protocol carries media
  as `bytes: Vec<u8>` through a bincode message. The host therefore sends `AttachMedia { slot: Other("disc"), kind: Other("disc-path"),
  name, bytes: <UTF-8 path of the .cue/.iso/.chd file> }` and the plugin streams sectors from the files itself (seek and read, a small
  sector cache; `std::fs` only, still `forbid(unsafe_code)`). The host's input layer (`inputs.rs`) first unpacks a `.zip` or a URL into the
  run's temp folder with **every** entry (a cue sheet needs its BIN files beside it) and passes the extracted cue path. Cue sheets are
  parsed by the plugin, `FILE`/`TRACK`/`INDEX`/`PREGAP`/`POSTGAP` and relative paths, refusing paths that leave the cue's folder.
  This is the one generic host change that Sega CD needs (section 5).
- **[P] Firmware by region.** Option `rom_bios` (`File`, 131072 bytes, the BIOS), `--firmware auto` via `machines/sega/firmware.toml`
  with `when = { console_region = ... }` and a preferred hash per region, verified by SHA-1 and by the header text. Preferred (RomM
  names; the table is written from the server's own list, hashes verified when first used): US `us_scd2_9306`
  (`5adb6c3af218c60868e6b723ec47e36bbdf5e6f0`), EU `eu_mcd2_9303` (`7063192ae9f6b696c5b81bc8f0a9fe6f0c400e58`), JP `jp_mcd2_921222`
  (`d203cfe22c03ae479dd8ca33840cf8d9776eb3ff`); the older and region-free files stay as lower preferences. Region follows the disc header
  unless `--console-region` says otherwise; the BIOS file must match the region (an error naming both, not a silent wrong boot).
- **[P] Save data.** Internal backup RAM (8 KB [H?]) is the file option `backup_ram` (default `<disc stem>.brm` next to the cue, or
  `--sram FILE` like the Genesis), RAM cartridge size option `ram_cart = off|128|256|512` with its own file option; both are written
  when the guest flushes and at exit, through the existing `MediaSlot::Other("sram")` export. The `.brm` files on RomM are raw images
  and are used as they are, never overwritten unasked.
- **[P] Audio.** The mix stays stereo at the Genesis rate; CD-DA (44.1 kHz stereo 16-bit, read from the audio tracks) and the RF5C164
  (about 32.5 kHz [H?]) are resampled to it by the machine, in integer ratios or a documented fixed-point resampler (no floats in timing).
- **[P] Debugger.** Three CPUs in the profile (`main` = 68000, `sub` = 68000, `z80`); `OnCpu` accepts all three. Memory spaces:
  main bus, sub bus, Word RAM and PRG RAM views.
- **[P] Timing model.** Master clock as the Genesis. The sub-CPU, the CDC and the CDD are brought up to the main clock lazily before
  every access that can see them, exactly the Genesis approach for the Z80. Functional phase: both CPUs instruction-granular, CD sector
  delivery at the nominal rate (75 sectors/s single speed [H?]); accurate phase: bus-cycle sharing of Word RAM and PRG RAM, CDC DMA
  timing and seek times from probes.

## 3. Hardware scope (one row = one slice of work)

| Block | Needed for | Accuracy target |
|---|---|---|
| Sub-CPU 68000, 24-bit bus, 12.5 MHz [H?] | the BIOS and every game's CD side | the shared core, per instruction first |
| BIOS ROM 128 KB, boot overlay at the main bus `$000000` | boot | exact map |
| PRG RAM 512 KB [H?], 128 KB window on the main bus, write protect | the sub program | exact |
| Word RAM 256 KB [H?], 2M mode and 1M/1M mode, ownership handover, DMA to VDP | data exchange | exact switching, bus sharing later |
| Gate array registers (main `$A12000`, sub `$FF8000` [H?]): reset/halt/bus request, interrupts, mailbox, communication words, stopwatch, timer | everything | each register a fact row |
| CDC LC89510: sector buffer, header/data decoding, DMA to Main (via Word RAM/PRG), Sub, PCM RAM | loading | transfer level, then timing |
| CDD: status/command packets, TOC, seek, play, pause, ... | disc access | protocol level |
| CD-DA output, volume fades | music | sample-level |
| RF5C164: 8 channels, 64 KB PCM RAM, banked | game audio | sample-exact against two oracles |
| Backup RAM 8 KB and RAM cartridge | saves | exact |
| Graphics ASIC: stamp map, scaling and rotation into Word RAM | Silpheed, Final Fight CD, FMV titles | pixel-exact against two oracles |
| Disc: CUE/BIN, ISO, CHD v5, subchannel Q, pregaps | all games | exact TOC and sector content |

## 4. Facts to source before coding them (all start as [H?])

Sub-CPU clock and its relation to the master; memory maps of both buses; every gate-array register and bit (main and sub), the
interrupt levels and their sources, the Word RAM mode and swap protocol, the PRG RAM bank bits; the CDC register map, the sector header
format and DMA destinations; the CDD packet format and status codes, the BIOS's expected sequence; RF5C164 registers, envelope, loop
markers, bank select, output rate; the ASIC's stamp format, size and rotation table, trace vector layout; backup RAM format; CD-DA and
volume fades. Sources to look for: the Sega Mega-CD hardware manuals as collected by segaretro.org, Charles MacDonald's Sega CD
documentation, the Sanyo LC89510 and Ricoh RF5C164 datasheets (read, never committed), Nemesis's and others' research posts, and ECMA-130
(sector structure). Each gets a row before it is coded.

## 5. Verification

1. Unit tests per block against documented behaviour; the disc crate against hand-made cue sheets and images (sizes of MB, not GB).
2. Probe programs for the sub-CPU and main CPU in the style of the Genesis probes (`machines/sega/scenarios/probe.py`): they run from
   a tiny disc image the probe tool builds (valid boot sector, a few data sectors), write results to Word RAM and PRG RAM, and are
   compared with two oracles. A disagreement is decided by documentation, then MAME.
3. Boot pictures of the BIOS (CD player screen with no disc, with an audio disc) and of each of the 29 games' first screens compared
   with Genesis Plus GX by the same similarity rule the Genesis used (`scenarios/compare_frames.py`, `batch_compare.py`).
4. Scenario manifest `machines/sega/segacd.manifest.toml`: boot, input, tone, CD-DA RMS, frame length, save/restore, backup RAM
   persistence; goldens made by the oracle, never committed.
5. Fuzz targets: cue sheet parser, CHD reader, CDC command stream, sub-CPU random programs for panics and hangs (`RC_SWEEP`).

## 6. Generic host changes (not ABI changes)

1. **Disc routing**: `.cue`, `.chd` and `.iso` given to `--disc` (new common option, or the machine's declared media kind) go to the
   plugin by path; `inputs.rs` unpacks zips and URLs with every entry beside each other. No Sega CD extension logic in the host beyond
   the media kind list the plugin declares.
2. `--sram`-style persistent files for backup RAM and the RAM cartridge reuse the Genesis route.
3. Disc swap and eject as actions (`load_disc`, `eject_disc`) in the file-action list, like tape/disk; the machine reports the lid.
4. The debugger follows the CPU addressed (already done for the Genesis) for three CPUs.
5. Window geometry needs nothing new: the picture is the Genesis VDP's.

## 7. Slices

Phase 1, functional:

1. **Disc crate**: cue/bin, ISO, TOC, sector read, subcode-Q, path hand-off, host routing; tested with generated images.
2. **Sub-CPU system**: second 68000, BIOS overlay, PRG RAM, Word RAM modes, gate-array registers, reset/halt/bus request, interrupts,
   mailbox; the BIOS runs and reaches its wait for the drive.
3. **CDD and CDC**: command/status protocol, TOC, seek/read/play, sector buffer, DMA paths; the BIOS shows the CD player screen and
   starts a data disc.
4. **Peripherals**: stopwatch, timers, backup RAM and RAM cartridge, subcode; saves persist.
5. **Audio**: RF5C164 and CD-DA, mixer, volume.
6. **Games**: run the data-disc titles of the 36, fix what they find; manifest and frame comparison.
7. **Graphics ASIC**: stamps, scaling, rotation.
8. **FMV**: sector rate, DMA throughput, the FMV titles.
9. **CHD** (5 of the 29 games are CHD, so this moves up if their games are wanted early): v5 reader (zlib, LZMA, FLAC and the CD-specific codecs from the file format description), with existing permissive crates
   for the codecs only after their licences are checked.

Phase 2, accurate: probe-driven bus-cycle sharing and CDC/CDD timing, `--execution-mode accurate`, save states (`RCST` v3 with the
sub-CPU and CD state), benchmarks, documentation, the separate reviewer pass.

## 8. Exit criteria

- BIOS boots to the CD player screen without a disc, plays an audio CD (CD-DA) and starts a data disc, in all three regions.
- Every one of the 29 RomM games starts, reaches its title screen and takes input; the ones needing the ASIC or FMV speed do so after
  slices 7-8; each exception is a documented `known-gap`.
- Frames agree with Genesis Plus GX by the similarity rule on the boot list; PCM and CD-DA agree in RMS and tone with two oracles.
- Backup RAM and RAM cartridge persist; save states round-trip; `retroclawd segacd --help` shows every option.
- `tools/check_all.sh` passes; `docs/benchmarks.md` has the speed; `TODO.md` lists what is parked.

## 9. Non-goals and parked

32X (its own design), Sega Pico, LaserActive and the Pioneer PAC, TeraDrive, Wondermega and X'Eye specific hardware (their BIOS files
are accepted as BIOS only if they behave as a Sega CD), Light-gun CD titles' gun (Menacer exists on the Genesis; Mad Dog McCree and
Corpse Killer use it, so port options `port1`/`port2` apply as they do there), netplay, rewind, disc image writing.

## 10. Risks

- **Size**: this is as big as the Genesis was. Mitigation: slices that each boot something visible; the BIOS is the first test.
- **Facts**: the documentation is thinner than for the Genesis and some behaviour is known only from emulators. Mitigation: oracle
  agreement rule from the Genesis design and probes on a tiny generated disc.
- **Test discs are copyrighted**: only the RomM discs and probe discs generated by our tools are used; neither is committed.
- **Memory and I/O**: streaming from files, bounded sector cache; a corrupt CHD or cue sheet is an error, never a panic (fuzzed).
- **Firmware variants**: region-free and hacked BIOS files exist on the server; the preferred-hash list and the header check keep a
  wrong file from booting silently.
