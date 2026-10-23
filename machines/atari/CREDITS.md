# Credits and sources

## Atari 8-bit

- **XL/XE and expanded Media facts**: Altirra Hardware Reference 2026-01-02,
  sections 2.5-2.7 (PORTB and independent CPU/ANTIC bank enables), ANTIC reset,
  dynamic DMA/bitmap feedback, phantom PM receive, GTIA special/pseudo-E modes
  and HITCLR bus phase; chapter 9 and appendix C (motor, serial framing).
  [A8CAS format specification](https://a8cas.sourceforge.net/format-cas.html)
  defines FUJI/baud/data chunks. Atari800 revision
  `63a381884175e3cbfc745ff00dfb3d70d928eb50`,
  [`DOC/cart.txt`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/DOC/cart.txt)
  and `src/cartridge_info.c` define CAR headers, checksums and mapper types
  1/2/12/13/14; `src/sio.c` defines SD/ED/DD XFD geometry.
  Hardware/container facts are independently implemented, not translated code.
- **AltirraOS XL/XE 3.11 replacement**, Avery Lee, `src/roms/altirraos_xl.c`
  at that pinned Atari800 revision. Same notice-preserving permissive terms
  as the 800 replacement below. SHA-1
  `738b9590a29f424ad2c9d3f9cb1d170de79241f7`; no ROM bytes are committed.
  Real-OS CIO/SIOV/cartridge tests exercise the replacement's own entry points,
  not host loader traps.
- **XEX and ATR format facts**, Atari800 7.2.1, revision
  `63a381884175e3cbfc745ff00dfb3d70d928eb50`:
  [`src/binload.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/binload.c)
  (`BINLOAD_Loader`, `loader_cont`: signature, separators, little-endian inclusive segments);
  [`src/afile.h`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/afile.h)
  (`AFILE_ATR_Header`: fields and magic);
  [`src/sio.c`](https://github.com/atari800/atari800/blob/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/sio.c)
  (`SIO_Mount`, `SIO_SizeOfSector`: paragraph count, protection, logical boot sectors).
  `atari-media` independently implements the container facts, not emulator code.
  It rejects noncanonical/extended ATR geometry rather than guessing; no filesystem,
  loader execution, copy protection, or SIO behavior lives in the format crate.

- **Altirra Hardware Reference Manual, 2026-01-02**, Avery Lee:
  <https://www.virtualdub.org/downloads/Altirra%20Hardware%20Reference%20Manual.pdf>.
  Hardware description, not translated implementation code. Chapters 2/4/5/6/14:
  clock/raster counts, 400/800 map, PIA wiring, ANTIC display lists/DMA/WSYNC/NMI,
  POKEY divisors/polynomials/keyboard matrix/serial status and GTIA registers.
  Chapter 2 (page 30) identifies ANTIC's HALT bus ownership; section 4.14
  (page 87) specifies nine blank-line refresh slots at 25, 29, ..., 57.
  The WSYNC discussion (pages 77-78) separately specifies RDY, which cannot
  stop NMOS write cycles. The Machine distinguishes DMA HALT from WSYNC RDY;
  no CPU cycle count is changed to compensate for arbitration.
  Section 4.8 (pages 74-76), "DLI timing", places NMI at cycle 8;
  chapter 3, "Overlapping interrupts", describes NMI hijacking BRK/IRQ.
  Accurate mode connects the existing Hardware NMI edge to the shared
  Core's vector-selection hook. Fast mode remains instruction-batched.
  Sections 4.7-4.14 source scrolling, NMIST/NMIRES, NMIEN deadlines and late
  enable delay, blocked NMI pulses, VCOUNT, WSYNC, normal playfield/PM DMA,
  refresh displacement and virtual bus sampling. The Machine's status-push
  callback handles the short pulse lost during interrupt acknowledgment.
  Chapters 5 and 14.4 source the shared noise register, delayed noise taps,
  prescalers, byte counters, borrow/IRQ pipelines, STIMER and two-tone resync.
  Sections 6.5-6.10 and 14.3 source PMG sizing, retriggering, VDELAY, collision
  latches, priorities and delayed graphics/output phases. Unsupported effects
  remain documented rather than inferred from a passing unit test.
- **Atari800 7.2.1**, GPL-2.0-or-later, source
  `63a381884175e3cbfc745ff00dfb3d70d928eb50`,
  <https://github.com/atari800/atari800>. Reference boot/raw Program execution
  uses a private portable Win64 build, explicit ROMs, no ROM patches, monitor
  frame boundaries and its native screenshots. No emulator implementation was
  translated. ZIP SHA-256:
  `82eb30836613a03b3a82d594da71b47667f2eef844d9cd808e3a367f250613ab`.
- **AltirraOS 800 3.11 and replacement BASIC displaying 1.59**, Avery Lee,
  distributed as `src/roms/altirraos_800.c` and `altirra_basic.c` at that pinned
  Atari800 revision. Their notices permit copying/distribution with copyright
  and notice preserved, without warranty. Private extraction retains both
  notices; no ROM bytes are committed. These are replacements, not original
  Atari firmware. Exact hashes are in the intake and `firmware.toml`.
- **MAME Atari 400/800 driver**, revision
  `9071ca1a6d1f1625045054c6f1fd0346a7e5a511`,
  `src/mame/atari/atari400.cpp`: independent memory-map and ROM metadata
  cross-check; not copied implementation.
- **Acid800 1.2**, copyright 2010-2022 Avery Lee, MIT as stated in its
  `README.html`, <https://www.virtualdub.org/downloads/Acid800-1.2.7z>.
  Downloaded privately, archive SHA-256
  `4074e6a40db1833e97603e334ca130f5923bff34a0d4f8a83e2ecf9617de1b20`.
  Executed privately through PIA/POKEY SIO without ROM loader traps.
  `cpu_illtiming.s` measures VCOUNT over 210 iterations, not a POKEY timer;
  its refresh allowance exposed accurate mode letting writes skip DMA.
  ANTIC status, interrupt, DMA, scroll and PMG kernels and the POKEY noise,
  timer, initialization, serial and GTIA graphic kernels provide private
  phase/deadline observations. Hardware facts and independently written
  mechanism tests are used; no diagnostic source or binary is committed.
  Current observations and remaining failures are in `docs/atari800-gaps.md`.
- **NMOS 6502 and MOS 6520 PIA**: reused from `retroclawd-crates` revision
  `6aad1a0af5fb820b85bc085e8b6d9cc017316c38`; the shared repository's
  credits cover the Core conformance suites and PIA sources.

## Atari ST

- **EmuTOS 1.4** (GPL-2.0, https://emutos.sourceforge.io/, files at
  https://sourceforge.net/projects/emutos/files/emutos/1.4/): the firmware the machine is developed and tested
  with. Fetched by the host's `--firmware auto` from the official zip, never committed. Facts used from the release
  (readme.txt in `emutos-192k-1.4.zip`, and the image header): the 192 KB images are for the ST, STf and Mega ST,
  start with `BRA.S` to `$30`, and have their reset PC at `$00FC0030` (ROM at `$FC0000`).
- **The Atari ST Internals** (transcription, https://atarimuseum.ctrl-alt-rees.com/computers/16bits/STINTERN.htm):
  memory map, vector table, MFP and ACIA assignments, register addresses. A secondary source; every fact used in code
  is checked against a primary source where one is available and cited there.
- **MC68000**: the core is `mc68000` of retroclawd-crates (its own CREDITS).
- **Hatari 2.6.1** (GPL-2.0+, https://www.hatari-emu.org/, Windows build
  https://framagit.org/hatari/releases/-/raw/main/v2.6.1/hatari-2.6.1_windows64.zip, SHA-256
  `996ff8df1fd6bd909ded90d5460d5639f3d8dee2c74921f3983a4cd10ff02812`): the reference emulator, run as a black box
  only (`retroclawd-tools/scripts/capture_hatari_trace.py` makes the CPU trace `tests/emutos.rs` compares). No code,
  table or comment of it is copied.
- **Hardware documents** (read, never committed; each fact used is listed with its page in
  `docs/hardware-facts.md`):
  - *Atari ST Internals*, Abacus Software 1985, https://www.atarimania.com/documents/Atari-ST-Internals.pdf
    (SHA-256 `961b981dd4e9c3206216aa70daece969f8bcc51814822b7159514e5409d3effd`).
  - Motorola *MC68901 Multi-Function Peripheral*, Advance Information, January 1984,
    https://bitsavers.mirrorservice.org/components/motorola/68000/MC68901_Multi-Function_Peripheral_Jan84.pdf
    (SHA-256 `5441a092562cc5b55bb167298030647622132e0061c0be8d976e25fb60ba959c`).
  - DrCoolZic, *Atari Image File Formats* 1.0 (2014), "MSA file format" (Damien Burke's description, 1997),
    http://info-coach.fr/atari/documents/_mydoc/FD_Image_File_Format.pdf
    (SHA-256 `35335051c6c86c5cd59d9032af65d8d305da8dda8c04bba37d68596872bbffd3`; used for `st-floppy`'s `.msa`).
  - *Atari Compendium* chapter 2, "GEMDOS executable files" (the program file header and fixup table),
    http://bus-error.nokturnal.pl/atari_compendium/html/chap2.htm (page SHA-256
    `6941f8b1af4f32561703335fc5644944c929febf569c34724cc09a79b62a6fd9`; used for `st-floppy`'s boot floppy and
    `retroclawd-tools`' `.PRG` writer).
  - TOS documentation, "The system vectors", https://freemint.github.io/tos.hyp/en/the_system_vectors.html.
  - "A millisecond timer", ST-Log 34 (July 1989), https://www.atarimagazines.com/st-log/issue34/60_1_A_MILLISECOND_TIMER.php
    (the MFP timer clock).

- **Phase 3 oracle** (Hatari 2.6.1 again, still a black box): `capture_hatari_mem.py` runs it with `--machine st --memsize 1
  --cpu-exact true --video-timing ws3 --monitor rgb|mono --sound off --fast-forward on --confirm-quit off --statusbar off
  --drive-led off`, an empty `--configfile` and a debugger script (`--parse`) with a VBL breakpoint and `savebin` (Hatari
  manual, "Debug options" and the debugger's `savebin`); `capture_hatari_screen.py` adds `--borders on|off`;
  `capture_hatari_trace.py --trace-extra fdc` takes its controller log; `--acsi 0=file` gives probe P22 a disk. Every
  timing fact found this way is labelled `[O]` in `docs/hardware-facts.md` with its probe (`docs/timing-oracle.md`).
- **MC68000 User's Manual** (Motorola), Table 8-14, exception and interrupt processing times: the figures
  `retroclawd-crates`' `tests/interrupt_timing.rs` pins (document SHA-256
  `89b690b1923f8a3cff508567090bcab0bcd07511c2deff8ffa393a08efda18e1`, never committed).
- **SingleStepTests 68000/v1** (https://github.com/SingleStepTests/68000, MIT): bus transactions of every instruction;
  `scripts/fetch_singlesteptests.sh` in `retroclawd-crates` fetches it, never committed.
