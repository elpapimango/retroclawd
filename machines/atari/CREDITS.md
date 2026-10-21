# Credits and sources

## Atari 8-bit

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
  `b5375a3c5477ad4e485eaf78da6fbf19e7292a4b`; the shared repository's
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
