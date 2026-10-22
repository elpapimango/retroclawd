# Credits and sources

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
