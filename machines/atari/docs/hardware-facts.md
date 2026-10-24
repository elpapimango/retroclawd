# Atari ST hardware facts used in code

Every hardware fact the emulation relies on, with its source. Labels:

| Label | Meaning |
|---|---|
| `[D]` | component datasheet |
| `[H]` | hardware documentation: Atari's hardware specification, *ST Internals*, TOS documentation, contemporary technical articles |
| `[O]` | observed in Hatari 2.6.1, run as a black box |
| `[H?]` | unresolved: the value used is a documented choice, to be checked when a source turns up |

An `[O]` fact covers only the single address or value observed. Applying it to a range needs `[H]` or `[D]`. Where
the timing behind a register is unknown, the machine stops at that register ("no hardware fact") instead of
inventing a value.

## Sources

| Short name | Document |
|---|---|
| *Internals* | *Atari ST Internals*, Abacus Software 1985, PDF https://www.atarimania.com/documents/Atari-ST-Internals.pdf (SHA-256 `961b981dd4e9c3206216aa70daece969f8bcc51814822b7159514e5409d3effd`); page numbers are the book's |
| *MC68901* | Motorola MC68901 Multi-Function Peripheral, Advance Information, January 1984, https://bitsavers.mirrorservice.org/components/motorola/68000/MC68901_Multi-Function_Peripheral_Jan84.pdf (SHA-256 `5441a092562cc5b55bb167298030647622132e0061c0be8d976e25fb60ba959c`) |
| *TOS hyp* | TOS documentation, "The system vectors", https://freemint.github.io/tos.hyp/en/the_system_vectors.html |
| *ST-Log 34* | "A millisecond timer", ST-Log issue 34, July 1989, p.60, https://www.atarimagazines.com/st-log/issue34/60_1_A_MILLISECOND_TIMER.php |
| *Hatari io* | Hatari 2.6.1, `--machine st --tos etos192uk.img --memsize <n> --trace io_all,cpu_exception`; `retroclawd-tools/scripts/capture_hatari_trace.py --kind io` |
| *forum* | atari-forum.com threads cited in `scheduler.rs` (frame timing) |
| *MC6850* | Motorola MC6850 data sheet (DS9493R, 1985), https://docs.ampnuts.ru/eevblog.docs/Tektronix/TLA_Ensoniq/Datasheets/MC6850.pdf (SHA-256 `294a8b9f557d03a2a7f4d736d060244f39216dad2d5479d2e47954feb4f34655`) |
| *IKBD* | Atari, *Intelligent Keyboard (ikbd) Protocol*, revised 26 February 1985, scan in https://bitsavers.org/pdf/atari/ST/Atari_ST_GEM_Programming_1986/GEM_0883.pdf (SHA-256 `9f216bf7a02b9141b2bc3a9bb97225c18149d261aae4059b186e3119d3779d82`) |
| *WD177X* | Western Digital WD177X-00 data sheet, *1986 Storage Management Products Handbook* pp.1-1 to 1-26, https://bitsavers.org/components/westernDigital/_dataBooks/1986_Storage_Management_Products_Handbook.pdf (SHA-256 `72382885abfde5cdee875384aed7ca97c17112db047d7792d3a4b4d3ed0c895f`) |
| *RP5C15* | Ricoh RP/RF/RJ5C15 Real-Time Clock, No. EK-086-9908, June 1995 (owner's copy `docs/RP5C15.pdf`, not committed; SHA-256 `70d5df25f6106491657460d9b88375b28b9cca8cc29bfe0cb59dea7ee8e35414`) |
| *LMC1992* | National Semiconductor LMC1992/LMC1993 Computer Controlled Tone and Volume Circuits, advance information (owner's copy `docs/LMC1993.pdf`, not committed; SHA-256 `245647c3a2ca7e5d11cec36720f5da564746f1d86ed7145e23ec16b9064f07db`) |
| *ST spec* | Atari Corp, *Engineering Hardware Specification of the Atari ST Computer System*, 7 January 1986, 38 pages, a scan (owner's copy `docs/Atari_ST_Engineering_Hardware_Specification.pdf`, not committed; SHA-256 `93b1670a4fedd0e8b664a8ab276ea95a229adf95207bc0b5f16e4eb9f10e8efe`); page numbers are the document's own (its page 1 is the third PDF page) |
| *STe manual* | *Atari STe 520/1040 Computer Field Service Manual*, part C302481-001 Rev A, August 1991 (owner's copy `docs/520ste.pdf`, text; SHA-256 `5f0169321cd62e783096f3b6d50a737eeb004c9f7f002adcf6568633b72e5781`); section numbers |
| *TT030 manual* | *Atari TT030 Hardware Reference Manual*, June 1990 (owner's copy `docs/Atari_TT030_Hardware_Reference_Manual.pdf`; SHA-256 `5d71eb8813ba130322bcd48e87a3df9cf60bdb316b5027d67603ee13adba0178`); only its ST-compatible register list was read |
| *YM2149* | Yamaha YM2149 catalog, September 1992, https://bitsavers.org/components/yamaha/YM2149_199209.pdf (SHA-256 `a4b946bf525fc1a7c6dec9e7662ce2342d3a3c627f980c7c3b1a396df7e91d20`) |

The Atari *Engineering Hardware Specification* (1986, *ST spec*) was added on 2026-10-24 and read page by page; the rows it settles
are in "Engineering Hardware Specification" at the end of this file and a few labels were changed in place. The STe service manual
and the TT030 manual were searched for the STE registers (they have no register list for the pads, the video extras or the
Blitter). Two more documents were added and not used: the Mega STE schematic (Rev 2, SHA-256 `db28103077566c6af59ee70bbd6428d35d77510399560a0111c96f8e2571d2d0`,
a TT-class board with the SCU and VME bus) and the STBook specification (`f243c769fe7822cf7552500bd1ebc08b63e473f9968baa2a4fa3ead8b5129b9c`, not read).
Every `[H?]` and `[O]` row below not in that section is still unchecked.

## Memory map and decode (`io.rs`, `bus.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| I/O area | `$FF8000-$FFFFFF`, supervisor only, user access = bus error | `[D]` | *ST spec* p.25 (the first 2 KB and the I/O space are protected for supervisor references; user state access is a bus error), *Internals* p.55 |
| Device pages | `$FF8000` memory configuration, `$FF8200` video, `$FF8400` reserved, `$FF8600` DMA, `$FF8800` sound, `$FFFA00` MFP, `$FFFC00` ACIAs | `[H]` | *Internals* p.55 |
| `$FF8006` | bus error | `[O]` | *Hatari io*: "Bus error reading at address $ffff8006 PC=$fc007c" |
| `$FF8A3C` | bus error (no Blitter on the ST) | `[O]` | *Hatari io*: "Bus error reading at address $ffff8a3c PC=$fc0638" |
| `$FF860F` byte | open: reads `$FF`, no bus error | `[O]` | *Hatari io*: "IO read.b $ffff860f = $ff pc=fc0638" |
| `$FFFC21`, `$FFFC25`, `$FFFC27`, `$FFFC3B` bytes | open: read `$FF`, writes ignored, no bus error (EmuTOS's Mega ST clock probe) | `[O]` | *Hatari io* lines 5-12 |
| Any other I/O address | unknown: the machine stops | — | — |
| Cartridge window | `$FA0000-$FBFFFF`, up to 128 KB, read only | `[D]` | *ST spec* p.24 (128 KB at most, 15 address lines, ROM3 and ROM4 selects), *Internals* p.96 |
| Write to the cartridge window | bus error (the GLUE) | `[H]` | *Internals* p.96: "a write access to these addresses creates a bus error. The GLUE takes care of this." |
| Empty cartridge slot, and the window past a shorter image | open bus: bytes read `$FF`, no bus error | `[H?]` | slice 1 choice; *ST spec* p.24 says "the disclosure of cartridge presence or memory size is not supported in hardware", which gives no level |
| Application header | TOS compares the first long at `$FA0000` with `$ABCDEF42` and runs the application chain from there; the emulator does not check it | `[H]` | *Internals* ROM listing p.273 (`cmp.l #$ABCDEF42,(A0)+`). The `$FA52235F` of the Phase 2 plan was not found in any source read and is not used |
| `$400000-$4FFFFF` with 4 MB | bus error | `[O]` | *Hatari io* `--memsize 4`: "Bus error reading at address $4fffff" (agrees with slice 1's nonexistent-memory rule) |

## MMU (`mmu.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Register | `$FF8001`, byte, read/write; bits 3-2 bank 0, bits 1-0 bank 1: `00` 128K, `01` 512K, `10` 2M, `11` reserved | `[D]` | *ST spec* p.27 (the table of the sixteen codes, `11xx` and `0011`, `0111`, `1011` reserved), *Internals* pp.55-56 |
| Bits 7-4 | read back as 0 | `[H?]` | Hatari reads `$00` at power-on; nothing documents the upper bits |
| Power-on value | `$00` | `[D]` / `[O]` | *ST spec* p.4 "Memory Configuration Register (read/write, reset: all zeros)"; *Hatari io* line 1 "IO read.b $ffff8001 = $00" |
| Banks fitted per RAM size | 256K = 128K + 128K, 512K = 512K + none, 1M = 512K + 512K, 2M = 2M + none, 2.5M = 2M + 512K, 4M = 2M + 2M | `[O]` | final `$FF8001` written by EmuTOS per `--memsize`: `$00`, `$04`, `$05`, `$08`, `$09`, `$0A` |
| Address multiplexing | the word address splits into a column (low n bits) and a row (next n bits), n = 8/9/10 for a bank configured 128K/512K/2M; smaller chips ignore the top lines, so in a 2M bank `$408` aliases `$8` on 512K and `$208` on 128K | `[D]` for the aliasing, `[H?]` for the exact split | DRAM row/column addressing (16 chips of 64K/256K/1M bits, *Internals* p.55); agrees with the offsets EmuTOS 1.4's sizing checks (`$40008`, `$208`, `$80008`, `$408`, `$8` at `$FC01BC-$FC0202`) and with Hatari's final value for all six sizes |
| No RAM at an address (above the configured total, or an empty bank) | reads `$FF`, writes ignored, no bus error | `[H?]` | slice 1 choice; a forum report says writes vanish and reads return bus noise |

## Shifter (`st-shifter`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Video base | `$FF8201` bits 23-16, `$FF8203` bits 15-8; 256-byte steps; read/write; reset 0 | `[D]` | *ST spec* p.7 ("any 256 byte half page boundary", "reset: all zeros", RAM only), p.27; *Internals* p.56 |
| Video address counter | `$FF8205/7/9`, read only; reset 0 | `[D]` | *ST spec* p.7, p.27; *Internals* p.56 |
| Counter timing | unknown: an access stops | — | — |
| Sync mode | `$FF820A` bit 1: 0 = 60 Hz, 1 = 50 Hz; bit 0: 1 = external sync | `[D]` | *ST spec* p.27 ("External/_Internal Sync", "50 Hz/_60 Hz Field Rate"), *Internals* p.57 |
| Palette | 16 words `$FF8240-$FF825E`, `$0RGB` with 3 bits per gun (`$700`, `$070`, `$007`); medium resolution uses 0-3; read/write; not changed by a reset | `[D]` | *ST spec* p.8 (nine bits an entry on low nibble boundaries, 512 colours; only the first four entries in medium resolution; reset "not affected"), *Internals* p.57 |
| Unused bits read back | palette `$F888` bits and `$FF820A` bits 7-2 read as 0 | `[H?]` | not documented in *Internals* |
| `$FF8260` bits 7-2 | read as 1 | `[O]` | Hatari reads `$FC` in low resolution (EmuTOS at `$FC69C2`) |
| Resolution | `$FF8260` bits 1-0: 0 = 320×200 4 planes, 1 = 640×200 2 planes, 2 = 640×400 1 plane, 3 reserved; reset 0 | `[D]` | *ST spec* p.8, p.28; *Internals* p.58 |
| Resolution 3 | treated as a black picture | `[H?]` | reserved |
| High resolution colour | palette 0 bit 0: set = a set bit is black on white (EmuTOS leaves palette 0 at `$777`), clear = inverted | `[O]` | Hatari 2.6.1 mono desktop (S15) shows EmuTOS' `$777` as black on white; *Internals* p.58 says clear = black on white, so the book is taken to be wrong here |
| Bitplanes | each 16-pixel group is N consecutive words (plane 0 first), most significant bit = leftmost pixel | `[H]` | *Internals* video RAM chapter ("Video display"), and the screen formats in *TOS hyp* |
| Colour monitor in high resolution, mono monitor in low or medium | no picture (black) | `[H?]` | *Internals* p.57: "High-res operation puts the ST at a 70 Hz screen frequency" |
| 3-bit level to 8-bit RGB | `level * 255 / 7` | — | a display choice, not a hardware fact |

## Timing and interrupts (`scheduler.rs`, `machine.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Colour frame | 50 Hz: 512 clocks × 313 lines; 60 Hz: 508 × 263 | `[H]` | *forum*, cited in `scheduler.rs` |
| Mono line | 224 clocks | `[H]` | *forum* |
| Mono lines per frame | 501 (224 clocks each, a frame of 112 224 clocks); the picture is 400 lines of 80 bytes from raster line 34 to 433, with the fetch of a line starting at clock 8 | `[O]` | Hatari 2.6.1 `--monitor mono`, probes P14 (501 HBLs a frame, a counted loop fills 112 224 clocks) and P15 (video counter at every HBL: first line 34, last 433, 80 bytes a line, the fetch start fitted to the counter inside a line) |
| Sync mode at power-on | 50 Hz on a PAL machine, 60 Hz on an NTSC one | `[H?]` (a choice that differs from `[D]`) | *ST spec* p.14 and p.27: the register resets to all zeros, which is 60 Hz and internal sync, on every machine; the emulator keeps the region's rate because the bare-metal tests, the recorded fast-mode table and the EmuTOS trace comparisons assume it, and TOS writes the register early (*Hatari io* line 39) |
| HBL | level 2 autovector, vector `$68` | `[D]` | *ST spec* p.31, *TOS hyp* |
| VBL | level 4 autovector, vector `$70` | `[D]` | *ST spec* p.31, *TOS hyp* |
| MFP | level 6, vectored | `[D]` | *ST spec* p.31 (NMI 7, MFP 6, VBL 4, HBL 2), *TOS hyp* (`$78` "Level 6, MFP"), *Internals* p.33 |
| MFP channels | 0 BUSY, 1 DCD, 2 CTS, 3 Blitter done, 4 Timer D, 5 Timer C (200 Hz), 6 ACIA, 7 FDC/ACSI, 8 Timer B (display enable), 9-12 USART, 13 Timer A, 14 RI, 15 mono detect | `[D]` (*ST spec* p.31, the priority table; channel 3 "GPU operation done" is the Blitter's on the STE) | *TOS hyp* vectors `$100-$13C`; *MC68901* §3 channel order |
| MFP timer clock | 2 457 600 Hz | `[H]` | *ST-Log 34*: "the 68901 timers count cycles of a 2.4576 MHz input clock"; consistent with TOS's 200 Hz Timer C programmed ÷64 × 192 (*Hatari io* lines 70-74) |
| TBI | display enable; not connected yet (its timing is unknown) | — | *TOS hyp* `$120` |
| GPIP wiring | I0 Centronics BUSY, I1 DCD, I2 CTS, I3 reserved, I4 keyboard and MIDI ACIAs, I5 FDC and HDC, I6 RI, I7 mono detect; Timer B input counts lines with display enable | `[H]` | *Internals* p.61 |
| GPIP I7 | mono detect: low = mono monitor; a change interrupts when enabled | `[D]` | *ST spec* p.14 (video connector pin 4, "active low, any change will generate the interrupt if enabled"), *Internals* p.~95 |
| GPIP I5 | FDC/ACSI interrupt, high when idle | `[O]` | GPIP read `$A1` (*Hatari io* line 67) |
| GPIP I0 | Centronics BUSY, high with nothing attached | `[O]` | same read |
| GPIP I1-I3, I6 | low | `[O]` | same read |
| GPIP I4 | the ACIAs' shared IRQ, active low; low until both ACIAs have had a master reset | `[H]` *Internals* p.61; the power-on level `[O]` | `$A1` before EmuTOS resets the ACIAs, `$B1` after |
| RESET instruction | resets the MFP | `[D]` | *MC68901* §2.2.3 (RESET input); other devices are not reset until sourced |

## MFP (`mc68901`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Registers | 24, at RS1-RS5 = 0..23: GPIP, AER, DDR, IERA, IERB, IPRA, IPRB, ISRA, ISRB, IMRA, IMRB, VR, TACR, TBCR, TCDCR, TADR, TBDR, TCDR, TDDR, SCR, UCR, RSR, TSR, UDR | `[D]` | *MC68901* Table 1-? (register map) |
| ST addresses | `$FFFA01 + 2 × reg`, byte access | `[H]` | *Internals* pp.60-61 register list |
| Reset | all registers cleared except TADR-TDDR, UDR, TSR and VR; VR = `$0F`; timers stopped; GPIP lines inputs; timer outputs low | `[D]` | *MC68901* §2.2.3 |
| TSR at power-on | `$80` | `[O]` | *Hatari io* line 88 "IO read.b $fffffa2d = $80" |
| Timers | delay mode, prescalers 4, 10, 16, 50, 64, 100, 200; data 0 counts 256; event count and pulse width on A and B | `[D]` | *MC68901* §5 |
| Interrupts | pending set on an enabled channel's event; IMR masks the IRQ output only; S bit selects software end-of-interrupt; vector = VR bits 7-4 + channel | `[D]` | *MC68901* §3-4 |

## PSG (`psg.rs`, `ay-3-8910` YM2149 variant)

| Fact | Value | Label | Source |
|---|---|---|---|
| Addresses | `$FF8800` write = register select, read = selected register; `$FF8802` write = data | `[H]` | *Internals* pp.59-60 |
| `$FF8802` read | `$FF` | `[H]` | *Internals* p.60 ("will give you $FF only") |
| `$FF8801`, `$FF8803` | act as `$FF8800`, `$FF8802` (the chip sees A1 only) | `[H?]` | not documented; EmuTOS uses only the even addresses |
| Register select | the low 4 bits pick the register; upper bits nonzero select nothing (chip address) | `[D]` | *YM2149* "Address" (DA7-DA4 are the chip select) |
| Registers | R0-R15 widths and meaning | `[D]` | *YM2149* Table 3 |
| Port direction | R7 bit 6 port A, bit 7 port B: 1 = output; an output port reads back its latch | `[D]` | *YM2149* "Mixer" |
| Input port read | `$FF`, drives nothing | `[H?]` | the ST wires port A and B as outputs |
| Port A | bit 0 side select, 1 drive 0 select, 2 drive 1 select, 3 RTS, 4 DTR, 5 Centronics strobe, 6 general output, 7 reserved; output only | `[D]` | *ST spec* p.29 ("I/O Port A (Output Only)"), *Internals* p.60 (read with `pdftotext -raw`; the layout text shifts the rows) |
| Clock | "2 MHz" at the YM's CLOCK pin | `[H]` | *Internals* schematic (figure 2.3-2) |
| Clock ratio | CPU clock / 4, SEL keeping the master clock equal to the input | `[H?]` | the 2 MHz label and the 8 MHz CPU |
| Tone, noise, envelope | `fT = fM/(16 TP)`, `fN = fM/(16 NP)`, `fE = fM/(256 EP)` with 32 steps; shapes per Table 4 | `[D]` | *YM2149* sections (1), (2), (5), (6), Table 4 |
| D/A | 5-bit: step 31 full scale, each step 2^(-1/4) lower, step 0 silent; fixed level L at step 2L + 1 | `[D]` | *YM2149* Fig. 1 |
| RESET instruction | the YM is not reset | `[H?]` | no source wires the 68000's RESET output to the YM |
| Audio | mono, 44 100 Hz, sample `n` taken at cycle `ceil((n + 1) × clock / 44 100)` | — | a host choice, not a hardware fact |

## ACIAs (`acia.rs`, `mc6850`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Addresses | `$FFFC00` keyboard control/status, `$FFFC02` keyboard data, `$FFFC04` MIDI control/status, `$FFFC06` MIDI data (bytes) | `[H]` | *Internals* p.62 |
| Odd and word accesses | no fact: the machine stops | — | — |
| Registers | control, status, master reset, TDR/shift register, RDRF/overrun sequence, CTS/DCD | `[D]` | *MC6850* |
| Power-up | in master reset until software resets it | `[D]` | *MC6850* "Master Reset" ("must be set immediately after power-up") |
| Clock | 500 kHz | `[D]` | *ST spec* pp.12, 17 (transmit and receive clock inputs of 500 KHz), *Internals* p.47 |
| Clock ratio | CPU clock / 16; a bit is `divide × 16` CPU cycles | `[H?]` | 8 MHz / 16 = 500 kHz; Hatari uses 501 327 Hz, the same ratio |
| Line speeds | keyboard ÷64 (7812.5 baud), MIDI ÷16 (31 250 baud), 8N1 | `[D]` | *ST spec* pp.12, 17 (7812.5 bits/s by dividing by 64; 31.25 Kbaud by dividing by 16, eight data bits, one start and one stop bit), *Internals* p.47 (set by software through the control register) |
| CTS, DCD, RTS | not connected; CTS and DCD read low | `[H]` | *Internals* p.47; `[O]` status `$02` after the reset |
| Interrupt | both IRQs on GPIP I4, MFP channel 6 | `[H]` | *Internals* p.61, *TOS hyp* `$118` |
| Transmit start | the transmitter takes a written character at its next bit-clock edge | `[O]` | Hatari: TDRE back within one bit after the write (`--trace acia`) |
| Bit-clock phase | edges counted from the last control-register write | `[H?]` | not documented |
| RESET instruction | the ACIAs are not reset | `[D]` | the MC6850 has no reset input |

## IKBD (`ikbd`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Commands, reports, defaults | §3-§9 (relative mouse default, threshold 1, Y=0 at top, joystick 1 events, port 0 a joystick after any joystick command) | `[H]` | *IKBD* |
| Relative mouse record | `%111110LR` (bit 1 left, bit 0 right), dx, dy; split beyond -128..127 | `[H]` | *IKBD* §4.1 (page image) |
| Joystick event | `$FE`/`$FF` (joystick 0/1), then `%x000yyyy` (x trigger) | `[H]` | *IKBD* §5.1 |
| Mouse buttons in joystick mode | fire buttons, no mouse records: right = joystick 1, left = joystick 0 | `[D]` | *ST spec* p.13 (port 0 pin 6 "Left Button / Fire Button", pin 9 "Right Button / Joy1 Fire"); *Internals* p.69; the STe manual's prose has them the other way round but its pin list agrees with the spec |
| Stick bits | bit 0 up, 1 down, 2 left, 3 right | `[H?]` | the document and *Internals* p.77 say only "a bit for each direction" |
| Scan codes | Appendix A, 95 keys | `[H]` | *IKBD* Appendix A (page images) |
| Version code | `$F1` | `[O]` | *Hatari io* line 148; *IKBD* §8 allows `$F0` (first release) or `$F1` (second) |
| Reset reply delay | 502 002 CPU cycles after the RESET's second byte (or power-up) | `[O]` | Hatari `--trace ikbd_all,acia` (VBL 6 cycle 150 494 to VBL 10 cycle 11 472); *IKBD* §9.1 bound 300 ms |
| Power-up reply | sent while the ACIA is still in reset, so it is lost | `[O]` | Hatari: "ikbd acia not initialized, can't send byte=0xf1" |
| Clock at power-up | 00-00-00 00:00:00, BCD, one-second steps | `[O]` | Hatari: `$1C` at 1.2 s answers `00 00 00 00 00 01` |
| Calendar carries | two-digit year, leap when divisible by 4 | `[H?]` | not documented |
| Output queue | 256 bytes; a packet that does not fit is dropped | `[H?]` | *IKBD* §9.14 "<tbd>"; the 6301 has 128 bytes of RAM (*Internals* p.69) |
| Memory load/read/execute (`$20-$22`) | a load (count 0-128; a larger count keeps its first 128 bytes) is kept in a model of the controller memory, a read answers `$F6 $20` and six bytes from the address (zero where nothing was loaded), an execute is recorded and not run | `[H?]` | *IKBD* §9.24-9.26 give the formats only; the 6301's own ROM and RAM are not modelled |
| Joystick monitoring (`$17`) | every `rate` hundredths of a second `%000000ab` (a joystick 1 fire, b joystick 0 fire) then `%nnnnmmmm` (joystick 1 stick high, joystick 0 low); keys, mouse and events are not reported; PAUSE stops the samples; a rate of 0 samples as fast as 1 | `[H]` / `[H?]` | *IKBD* §5.3, §9.18 (the layout is in a garbled page: joystick 1 first is the document's order of the two fire bits and nibbles) |
| Fire button monitoring (`$18`) | joystick 1's fire button, eight samples a byte, the first the most significant bit, eight samples in the time a character takes (1280 cycles each) | `[H]` | *IKBD* §5.4, §9.19 |
| Joystick key code mode (`$19`) | make/break cursor keys on closure, repeated every T tenths until R tenths, then every V tenths; R = 0 uses V only; fire buttons are `$74` (joystick 0) and `$75` (joystick 1) | `[H]` | *IKBD* §5.5, §9.20 |
| Key code repeat spacing | T spacing holds while the next pair still falls before the breakpoint; V counts from the last pair; a spacing of 0 does not repeat | `[H?]` | the document does not say what happens at the breakpoint itself |
| Key code mode sticks | only joystick 0's stick makes cursor keys | `[H?]` | §9.20 says "joystick 0", §5.5 "either joystick" |
| Status inquiries | answered as the setting command (so the host can send them back), padded to 8 bytes after `$F6`; ignored in the monitoring modes; `$87` is `$07, action` | `[H]` | *IKBD* §9.27 |
| Threshold, scale, key code distance of 0 | read as 1 | `[H?]` | the document says "positive integers" |
| Threshold outside relative mode | ignored | `[H]` | *IKBD* §9.6 "valid only in RELATIVE MOUSE POSITIONING mode" |
| Key code mode and the Y origin | not affected | `[H]` | *IKBD* §9.5 |
| Motion while paused | accumulates and goes out as the fewest packets on resume; a button press sends it at once | `[H]` | *IKBD* §9.14 |
| Setting the clock | restarts the second: the next one is a whole second after the command | `[H?]` | the document gives no phase |
| Reset | a packet being sent is cut off, the queue is emptied, loaded controller memory is gone, the clock is kept | `[H?]` / `[H]` | *IKBD* §9.1 keeps the clock; the rest is not documented |
| Keyboard line | 7812.5 baud, 8N1, 10 bits a character (1024 CPU cycles a bit) | `[H]` *Internals* p.47; the IKBD's own bit clock equal to the ACIA's `[H?]` | — |
| Key names | the host's key names for the same position (`KeyA`, `Digit1`, `Escape`, `NumpadEnter`, `IntlBackslash` for the ISO key, `Undo`, `Help`) | — | a naming choice, not a hardware fact |

## Floppy images (`st-floppy`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Boot sector | BPB fields little-endian: bytes/sector 11, sectors/cluster 13, reserved 14, FATs 16, directory entries 17, total sectors 19, media 21, sectors/FAT 22, sectors/track 24, sides 26, hidden 28 | `[H]` | *Internals* pp.178-179 |
| Executable boot sector | the big-endian word sum of the 512 bytes is `$1234`; the program starts at offset 30 | `[H]` | *Internals* p.179 |
| Geometry rule | a plausible BPB that agrees with the size; a plausible BPB that disagrees is an error; otherwise the size table | — | design §13 (never guess) |
| Size table | 360K 80×1×9, 400K 80×1×10, 720K 80×2×9, 800K 80×2×10, 820K 82×2×10 | `[H?]` | the common ST formats |
| Track format | gap I 60 × `$4E`; per sector 12 × `$00`, 3 × `$A1`, `$FE` C H R N CRC, 22 × `$4E`, 12 × `$00`, 3 × `$A1`, `$FB`, data, CRC, 24 × `$4E`; gap IV `$4E` | `[D]` | *WD177X* p.1-18 |
| Short gaps | gaps I and III down to 2 bytes; gap II fixed | `[D]` | *WD177X* p.1-18 "Non-Standard Formats" |
| Sector size code | N = 2 for 512 bytes | `[D]` | *WD177X* sector length table |
| Track length | 6250 bytes (250 kbit/s MFM, 300 rpm) | `[H?]` | drive speed not sourced |
| Sector order on a track | 1, 2, 3, … (no interleave) | `[H?]` | the image does not record it |
| 11 sectors per track | refused: they do not fit 6250 bytes in the data sheet's format | — | — |

## DMA and floppy controller (`dma.rs`, `wd1770`)

| Fact | Value | Label | Source |
|---|---|---|---|
| `$FF8604` | FDC register (A1/A0 from the mode) or the sector count, read and write | `[H]` | *Internals* p.59 |
| `$FF8604` read, upper byte | reads as 1s | `[H]` | *Internals* p.59 ("consistently read 1"); Hatari shows `$00` — a known oracle difference, masked in the comparisons |
| `$FF8606` write (mode) | bit 1 A0, bit 2 A1, bit 3 HDC, bit 4 sector count, bit 6 DMA off, bit 8 write | `[D]` | *ST spec* p.28 (bit 5 reserved, 0), *Internals* p.59 (with `pdftotext -raw`) |
| Mode bit 7 | "FDC/_HDC": set selects the floppy controller as the DMA's device, clear the hard disk; not decoded (the device is the one bit 3 selected) | `[D]` for the meaning, `[H?]` for not decoding it | *ST spec* p.28; *Internals* lists bits 3 and 7 both as FDC/HDC |
| `$FF8606` read (status) | bit 0 set = no error, bit 1 sector count not zero, bit 2 FDC DRQ; other bits 0 | bits 1-2 `[H]`, bit 0 `[O]`, the rest `[H?]` | *Internals* p.58 prints bit 0 "0=no error"; Hatari reads `$..b1` after a good transfer and EmuTOS retries a read when bit 0 is clear, so the book is taken to be wrong here. Hatari's upper bits are leftovers (`$79b1`), masked in the comparisons |
| DMA address | `$FF8609` high, `$FF860B` mid, `$FF860D` low; read and write; written low first | `[D]` | *ST spec* p.36 note 841017B ("must be loaded in low, mid, high order"), p.29; *Internals* p.59 |
| FIFO | written to memory a burst at a time; the burst is 16 bytes and holds the CPU for 32 clocks | `[O]` | P16; *ST spec* p.5 says the FIFO is 32 bytes (the second 16 bytes are not modelled: no probe separates them) |
| Sector count | 8 bits, counts 512-byte sectors; the DMA stops taking bytes at 0 | `[H?]` | — |
| Write bit toggled | clears the FIFO and the error bit | `[D]` for the status, `[H?]` for the FIFO | *ST spec* p.5 (the status is "cleared by toggling Write/_Read in the DMA Mode Control Register"), p.18; EmuTOS's `$0190`/`$0090` pattern |
| DMA into ROM or the cartridge | ignored | `[H?]` | as a CPU write |
| DMA into I/O or nonexistent memory | no fact: the machine stops | — | — |
| FDC | WD1772, clocked by the CPU clock (8 MHz), so its times are the data sheet's | `[D]` *WD177X*; the clock `[H?]` | — |
| Type I status S5 | set once the spin-up phase is over (also when skipped with h = 1) | `[D]` | *WD177X* status register description; `[O]` `$E4` after RESTORE `$0B` |
| Force Interrupt | during a command only Busy resets; when idle the status becomes Type I; MO (the motor output) is kept either way | `[D]` | *WD177X* status register description and "MO signal goes back to a logic 0" after idle revolutions |
| Writing, DMA side | with the mode register's write bit set the DMA fills the 16-byte FIFO from memory at the DMA address and hands the controller one byte per data request; the sector count falls by one for every 512 bytes handed over | `[H?]` | mirrors the read direction above; the FIFO refill burst size is not documented |
| Writing, drive side | the byte the controller writes lands under the head at the revolution position; WRITE SECTOR and WRITE TRACK (format) both work this way; `F5` writes `A1` with a missing clock, `F6` writes `C2`, `F7` the CRC | `[D]` | *WD177X* write sector / write track command descriptions (the chip crate does the byte expansion) |
| Track back to sectors | the written track is decoded as the controller would read it (ID and data fields, CRC-CCITT) and the image takes the sectors; a sector the image's geometry lacks is ignored | `[D]` format, `[H?]` ignoring | *WD177X* p.1-18; a `.st` cannot change geometry |
| WRITE SECTOR end status | the shared `wd1770` leaves the DRQ status bit set after the last byte (status `$82`); the error bits are clear | `[H?]` | unchecked against the data sheet or Hatari; the machine treats a command as finished by BUSY alone |
| Write protect | the user's write-protect tab makes the controller see the write-protect line (WRITE commands end with status bit 6) and the drive takes nothing | `[D]` | *WD177X* status register, write protect |
| INTRQ | GPIP I5, active low | `[H]` | *Internals* p.61 |
| INTRQ reset | by reading the status register or by loading the command register with a new command (any command but Force Interrupt) | `[D]` | a WD177X data sheet transcription, <https://www.worldofsam.org/node/1097>, "INTRQ"; EmuTOS writes a write command without reading the previous status and then looks at GPIP I5 |
| HDC (ACSI) with nothing connected | writes vanish, I5 stays high | `[O]` | EmuTOS's probe in Hatari |
| HDC read | no fact: the machine stops | — | — |
| RESET instruction | FDC and DMA not reset | `[H?]` | — |

## Floppy drives (`drive.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Selects | PSG port A bit 1 drive 0, bit 2 drive 1, active low; bit 0 side (clear = side 1) | `[D]` | *ST spec* p.21, p.29 (Side0 select active high, drive selects active low, high at reset); `[O]` TOS writes `$07` to deselect, `$05` for drive A |
| Revolution | 6250 byte times | `[H?]` | 300 rpm, 250 kbit/s |
| Index pulse | the first 64 byte times of a revolution, only with a disk | `[H?]` | — |
| No disk | no index pulses, no data, write-protected | `[O]` | status `$E4` |
| A disk | not write-protected | `[O]` | Hatari statuses `$24`, `$A4`, `$A6` with a disk |
| Disk swap | write-protect reads set for 0.5 s (15 625 byte times) after a disk goes in or out while running; nothing for a disk there from power-on | `[H?]` | the sensor is blocked by the moving disk; TOS detects a change by it |
| Motor | the motor line reaches both drives | `[H?]` | — |
| Head range | cylinders 0-85 | `[H?]` | — |
| Drives fitted | 2 by default, 1 with `drives = 1`; drive B absent then | `[H?]` | — |

## Raster palette and the sync/resolution log (`st-shifter`, `scheduler.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Palette timing | a palette write takes effect on the raster line it is made on and every line below it, until the next write to that colour; the granularity is one line | `[H?]` | a choice; a change inside a line is Phase 3 (spec section 5) |
| First picture line | raster line 63 of the 313-line 50 Hz frame, 34 of the 263-line 60 Hz frame | `[O]` (50 Hz) / `[H?]` (60 Hz) | scenario S10: Hatari 2.6.1's first band change from a VBL-reset HBL counter lands on picture line 17 = raster line 80 = 63 + 17; ours lands one line earlier because our VBL handler runs before the HBL of the frame's last line, which then counts for the new frame (the order of the two at the frame boundary has no source); the 60 Hz line was not measured |
| Palette log | every palette write that changes a colour, with its line, in time order, at most 16 384 a frame; the palette register itself always takes the write | `[H?]` | a bound for memory, not hardware |
| Sync mode and resolution writes | logged with the raster line and the cycles since the line began, at most 256 a frame (the oldest dropped), shown in the stop report; the picture does not use them and no border opens | `[H?]` | overscan is Phase 3 (spec section 5) |

## ACSI hard disks (`acsi.rs`, `dma.rs`, `retroclawd-tools` `rcdisk hd`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Command block | 6 bytes written to `$FF8604` with the mode register selecting the command register (bit 3); the first byte with A0 = 0, the rest with A0 = 1; byte 0 = controller number (bits 7-5) and operation code (bits 4-0); bytes 1-3 the block address (21 bits; byte 1 bits 7-5 the device number), byte 4 the block count, byte 5 the control byte | `[H]` | *ACSI application notes* (Atari, 27 September 1985, <https://mirrorservice.org/sites/www.bitsavers.org/pdf/atari/ST/Atari_ST_GEM_Programming_1986/GEM_0942.pdf>, SHA-256 `a027f606fe71b0df5c33f2e86f38f20216a891bb1b4e613bdf3e8bf00e74f9fb`) sections 2-3 |
| Operation codes of the document | `$00` TEST UNIT READY, `$08` READ, `$0A` WRITE, `$0B` SEEK, `$1A` MODE SENSE | `[H]` | same |
| Completion status | one byte read at `$FF8604`: `$00` no error, `$04` not ready, `$20` invalid command, `$21` invalid address, `$23` volume overflow, `$24` invalid argument, `$25` invalid device number (the scan of the last two codes is poor) | `[H]` / `[H?]` for `$24`, `$25` | same, "Command Errors" |
| Handshake | DMA address, toggle the write bit, direction, select the sector count register, load the count (the trigger), select the command register, the select byte with A0 = 0, the others with A0 = 1, the last byte, then the status | `[H]` | same, section 3 |
| Command byte acknowledge | the controller raises GPIP I5 after every command byte but the last, and the driver waits for it before the next | `[H?]` | the document's "Command Phase" diagram shows the line beside each byte; EmuTOS waits on I5 after the select byte (observed, S11) |
| Completion | the transfer is made when the last byte arrives, all at once; I5 falls and stays low until the status is read | `[H?]` | the document gives no timing |
| Sector count | the DMA moves at most sector count x 512 bytes; the count falls by the sectors the transfer covers; a short count limits a READ and refuses a WRITE (nothing is written, DMA error bit) | `[H?]` | the document says the count triggers and bounds the DMA |
| Transfer outside RAM | refused: nothing moves, the DMA error bit is set (status bit 0 clear, the polarity of `[O]` above) | `[H?]` | the deferred item of slices 3-5 |
| Block count 0 | 256 blocks | `[H?]` | the SCSI convention; the document is silent |
| INQUIRY (`$12`), REQUEST SENSE (`$03`), MODE SENSE (`$1A`) data, READ CAPACITY (extended command `$1F`, then a SCSI-2 `$25` block) | SCSI-2 layouts: INQUIRY 36 bytes (direct access, 31 more bytes), sense in fixed format with the key of the last failed command, MODE SENSE a header and one block descriptor, READ CAPACITY the last block and 512 | `[H?]` | not in the 1985 notes; these are what EmuTOS's driver sends (observed, S11) and the SCSI-2 layouts; owner: download the ACSI/SCSI references to settle |
| Extended command | first byte `$1F` in the operation field, then a 10-byte descriptor (11 bytes in all) | `[H?]` | observed from EmuTOS |
| Target not there | its select byte is ignored, I5 stays high, a read of the HDC register is the known stop | `[O]` | EmuTOS's probe in Hatari (as before) |
| Images | a raw image of whole 512-byte blocks, at most 512 MiB; the guest's writes change the copy in memory only | `[H?]` | a choice |
| Root sector | `hd_siz` Motorola long at `$1C2`, four 12-byte partition headers at `$1C6` (flag: bit 0 exists, bit 7 bootable; id `GEM` < 16 MB, `BGM`, `XGM`; start and size Motorola longs in 512-byte sectors), bad sector list at `$1F6`/`$1FA` | `[H]` | *Atari Compendium* chapter 4 (http://bus-error.nokturnal.pl/atari_compendium/html/chap4.htm, page SHA-256 `36b88a35bf8858582348cc8f31bb75bf216f5302aca95214b04bf5d58ac2fff3`) |
| Extended partitions | the first sector holds a standard root sector's headers: a partition (start from that sector) and an `XGM` link to the next (start `[H?]` from the first extended partition) | `[H]` / `[H?]` | same |
| FAT16 | the BPB of a floppy with media `$F8`; 16-bit sector counts, so a partition of 32 MB or more has logical sectors above 512 bytes | `[H]` / `[H?]` | same page and *Internals* pp.178-179 |

## GEMDOS programs (`st-floppy` `boot.rs`; `retroclawd-tools` `rc-common` `prg.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Executable header | 28 bytes: `$601A`, text, data, bss and symbol table sizes (longs at `$02 $06 $0A $0E`), a reserved long, `PRGFLAGS` at `$16`, `ABSFLAG` word at `$1A` | `[H]` | *Atari Compendium* chapter 2 (http://bus-error.nokturnal.pl/atari_compendium/html/chap2.htm, page SHA-256 `6941f8b1af4f32561703335fc5644944c929febf569c34724cc09a79b62a6fd9`) |
| Fixups | after the symbol table a long, the offset of the first longword to fix up (0: none), then bytes: 0 ends, 1 advances 254, an even 2-254 advances and fixes up; a fixup adds the load address | `[H]` | same |
| Offset origin | the first fixup offset counts from the start of the text segment (the text and data share one offset space) | `[H?]` | the document says "from the beginning" of the executable |
| Start | a program starts at byte 0 of its text with the base page at 4(sp) | `[H]` | same |
| Symbol table | not written; the document leaves its layout to the toolchain | `[H]` | same |
| AUTO folder | TOS runs `AUTO\*.PRG` of the boot drive before the desktop; a floppy with no executable boot sector but a FAT12 file system and an `AUTO` folder is enough | `[O]` | EmuTOS 1.4, scenario S8P and `tests/prg.rs` |
| User mode | a program from AUTO runs in user mode: it may not touch `$0-$7FF` (a bus error, EmuTOS's crash screen) | `[H]` | *Internals* p.55 (the bus error rule) and observed |
| Generated floppy | 720K, 512-byte sectors, 2 per cluster, 1 reserved, 2 FATs of 3 sectors, 112 root entries, media `$F9`, time stamp 2026-01-01 | `[H]` | *Internals* pp.178-179 (the BPB fields) |

## USART, RS-232, MIDI and printer (`mc68901` `usart.rs`, `serial.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| UCR | CLK (bit 7: divide by 16), WL1 WL0 (bits 6-5: 8, 7, 6, 5 bits), ST1 ST0 (bits 4-3: `00` synchronous, `01` 1 stop, `10` 1.5 stop, `11` 2 stop), PE (bit 2), E/O (bit 1) | `[D]` | *MC68901* §6.1.3 Figure 6-2 |
| Baud clock | timer D drives the USART clocks; a timer output toggles on each time-out, so one clock period is two time-outs; a bit is one period (CLK 0) or sixteen (CLK 1) | `[D]` `[H]` | *MC68901* §5.1.1 (output toggles), §6.1 (divide by 16); *Internals* p.61 ("Timer D sends the timing signal for the MFP's built-in serial interface") |
| Character time | (1 start + data bits + parity + stop bits) bit periods | `[D]` | *MC68901* §6.1.1, Figure 6-2 |
| RSR | BF 7, OE 6, PE 5, FE 4, F/S or B 3, M/CIP 2, SS 1, RE 0; BF set when a word reaches the buffer and cleared by reading UDR; OE rises when the held-back overrun word meets a read buffer, clears on an RSR read, and blocks new words until then; RE = 0 clears every flag at once | `[D]` | *MC68901* §6.2.2 Figure 6-3, §6.2.3 |
| Peers' faults | the built-in peers deliver clean frames; a test or scripted peer can send a character with a parity, framing or break fault (`AtariSt::rs232_receive_fault`, `rs232_end_break`) | `[H?]` | a choice, no line noise is modelled |
| Receive flags | the RSR is latched when a word is transferred to the buffer and its flags cannot change until the word is read (CIP excepted); PE is set when the word has a parity error (parity enabled) and cleared otherwise, FE when a *non-zero* word has no stop bit; each describes the word in the buffer | `[D]` | *MC68901* §6.2.2 |
| Overrun | a word due while the buffer is full is lost without touching the buffer or the RSR; OE is set once the UDR has been read and cleared by reading the RSR; no word is assembled while OE is set | `[D]` | *MC68901* §6.2.2 |
| Break | an all-zero word with no stop bit: B is set (not FE), the buffer holds 0; B clears when the break has ended (a non-zero bit came in) and the RSR has been read since it began, and that raises the receive error interrupt again (end of break); a peer ends it with `rs232_end_break` or by sending the next character | `[D]` | *MC68901* §6.2.2 (the B bit) |
| Break and the buffer | a break received while the buffer is full is no overrun: B shows once the buffer has been read; after a lost word as well, both B and OE show | `[D]` | *MC68901* §6.2.3 |
| Error interrupts | one interrupt per character: on the receive error channel when it is enabled, else on the buffer full channel with the buffer full one | `[D]` | *MC68901* §6.2.1 |
| TSR | BE 7, UE 6, AT 5, END 4, B 3, H 2, L 1, TE 0; a write to UDR clears BE, BE sets when the word moves to the shift register; UE sets when a word ends with the buffer empty, cleared by reading TSR or disabling; disabling with an idle shifter sets END at once, with a word going out after it; enabling clears END; AT turns the receiver on after the last word | `[D]` | *MC68901* §6.3, §6.3.2 Figure 6-4 |
| Loopback | H = L = 1: the transmitter feeds the receiver, the receive clock is the transmit clock | `[D]` | *MC68901* §6.3.2 |
| Break transmission | the B bit stops words moving out of the buffer; no break is shown to a peer | `[H?]` | *MC68901* §6.3 describes the line state, the machine has no line level |
| Start of a word | the buffer is moved to the shift register at the next time-out of timer D | `[H?]` | §6.3 says only "a delay before the first bit" |
| Interrupt channels | 9 transmit error (UE, END), 10 transmit buffer empty, 11 receive error, 12 receive buffer full; an overrun uses 12 when 11 is disabled | `[D]` | *MC68901* Table 3-1, §6.2.1 |
| Reset | UCR, RSR, SCR cleared, words in flight stopped; UDR and TSR keep their values | `[D]` / `[H?]` | §2.2.3 lists TSR among the registers reset leaves alone; Figure 6-4 says reset clears TE: the register list is followed |
| Synchronous format | not modelled: a transmitter in it never finishes a word | `[H?]` | the ST does not use it |
| MIDI line | the MIDI ACIA at ÷16: 256 CPU cycles a bit, 2560 a character | `[D]` | *ST spec* p.17 (31.25 Kbaud, ÷16 of 500 kHz, 8 data bits, a start and a stop bit), *Internals* p.47 and section 2.5 |
| Centronics | data lines are PSG port B; the strobe is PSG port A bit 5, a low pulse; BUSY is GPIP I0, high when the printer cannot take a byte | `[D]` (*ST spec* p.16: strobe "TTL levels, active low", BUSY "active high, 1 Kohm pullup resistor to +5 VDC") and `[H]` | *Internals* p.59 (port A list, read one position down: the page's column is shifted), pp.88-89, p.61 |
| Strobe edge | the byte on port B is taken when port A bit 5 falls between two states in which port A is an output | `[H?]` | the document says only that a low pulse marks the data valid; either edge sees the same byte |
| BUSY | low with a printer connected, high without; no busy time after a byte | `[D]` for "high without" (the pull-up), `[H?]` for the rest | *ST spec* p.16: BUSY is active high with a 1 kohm pull-up, so an open port reads busy; the document gives no timing |

## Timing (Phase 3)

Every row here is `[O]`: observed with a timing probe that runs in Hatari 2.6.1 (a black box) and in retroclawd with
byte-identical results (`timing-oracle.md`). The row names its probe.

| Fact | Value | Label | Source |
|---|---|---|---|
| Method | probes P01-P20 of the Phase 3 plan; Hatari with `--cpu-exact true --video-timing ws3` (its default wake-up state) | `[O]` | `timing-oracle.md` |
| Probe harness | a boot floppy loads the body with XBIOS Floprd and jumps to `$90000`; results at `$C000` | `[O]` | P01 (the 16-byte smoke test matches) |
| Bus cycle slot | in `accurate` mode a bus cycle to RAM, ROM, the cartridge, the Shifter, the MMU or the DMA chip starts on a multiple of 4 clocks; one that would start between two waits for the next, so an instruction with internal clocks at its end (6, 10, 14 clocks by the book) costs the next multiple of 4 | `[O]` | P02, variants 0-12, 14, 15, 18-45: the mean loop runs a line equal to Hatari's to 0.035 with this rule (without it 0.3-1.4 runs too many) |
| MFP, YM2149, ACIA | a bus cycle to them waits 4 clocks more than one to RAM (on top of the slot) | `[O]` | P02, variants 13, 16, 17: 12 clocks for `MOVE.B (An),Dn`, 8 for RAM; the E-clock phase did not matter (a wait of 0-9 clocks for the E clock fitted worse) |
| Interrupt acknowledge | the acknowledge waits for the next edge of the E clock (a tenth of the CPU clock, counted from the start of the frame), then 8 clocks more before the stacking; for HBL and VBL (autovector) and the MFP alike | `[O]` | P02: fitted to the mean loop runs a line (all 48 variants within 0.035); the line-by-line counts differ on about a third of the lines (manifest P02, known gap): where inside an instruction the interrupt is taken is modelled to a few clocks only |
| Probe P03 | the alignment rule needed no probe of its own: P02 sweeps instruction lengths of 4 to several hundred clocks and the three kinds of address | `[O]` | plan ruling |
| Video address counter | `$FF8205`/`$FF8207`/`$FF8209` read the address of the next word the Shifter fetches, the high, middle and low byte; read only (a write is ignored). Above the picture it holds the base, then from clock 56 of each of the 200 picture lines it moves 2 bytes every 4 clocks for 80 words (160 bytes a line) and holds there to the end of the line; below the picture it holds base + 32000. The first picture line is 63 of the 313-line frame | `[O]` | P04: the border lines agree exactly with Hatari's, the picture lines to one word (probe fit: window start 56-59; 56 chosen) |
| VBL request | in `accurate` mode the VBL is requested 64 clocks into line 0, after the HBL of the frame boundary; a program counting HBLs from the VBL numbers the lines as Hatari does (`fast` mode keeps the request with the HBL, as the EmuTOS trace comparison needs) | `[O]` | P02 (the first counts after the VBL) and P04 (the line a counter value belongs to); optimum 38-70 clocks, 64 chosen |
| Interrupt acknowledge, E clock | the E clock's edges fall 6 clocks past a multiple of 10, counted from the frame start | `[O]` | P02 joint fit with the VBL delay: 9 of 48 variants equal line by line |
| Access point | a device sees a write at the start of its 4-clock bus cycle and a read 3 clocks in | `[O]` | P06: the MFP timer A data register read after the control register started the timer, four prescalers and 64 starting points each: 254 of 256 reads equal Hatari's, two differ by one count (manifest P06, known gap) |
| MFP timer start | in `accurate` mode a write to a timer control register ($FFFA19, 1B, 1D) restarts the MFP tick count at that clock, so the prescaler runs from the write exactly and not from the next tick of a clock running since power-on | `[O]` | P06: with the global tick clock the first counts came up to a tick (3.26 clocks) late, and differently depending on when the program started; the state schema went to 2 for the two new fields of the clock |
| Palette write along a line | in `accurate` mode a write to a palette register at clock `c` of a line changes the pixels from `c - 79` on (the first pixel of the picture is drawn at clock 79: the fetch starts at 56 and the Shifter takes 23 clocks to draw the word), one low resolution pixel or two medium resolution pixels a clock; a write on an earlier line applies to the whole line | `[O]` | P07 / S12: all 122 rows of red in Hatari's picture start and end on the same pixel as ours |
| Resolution written along a line | in `accurate` mode a line is drawn whole in the resolution the register has 464 clocks into it; a write at or before that clock decides it, a later one (a switch back) does not and starts the next line; no part of a line is drawn in another resolution; `fast` mode draws the frame in the register's final resolution | `[O]` | P35 (lines 109 to 179 of the picture medium, as ours, pixel for pixel; the sample clock gives the same picture for 464-467 and another at 460 and 468) and P45 (every block of medium lines equal but two lines at an edge: Hatari's two captures of it differ by a line there); S20, S21 |
| Canvas with borders | `borders = on` (accurate mode) draws 832 x 552: raster lines 34 to 310, clocks 31 to 447 of each line, the 640 x 400 picture at (96, 58); the border colour is palette entry 0 at each pixel, data fetched by an opened left or right border is drawn in the border area | `[O]` | S13: Hatari 2.6.1 `--borders on`, P07 at VBL 700: canvas size and picture position measured; strips and border equal within the similarity limit |
| Picture window (top and bottom border) | the sync mode at the start of each line decides: the picture starts at line 34 when it says 60 Hz and at line 63 when it says 50 Hz, and ends at line 234 or 263; a line that starts in the other mode misses the start (no picture that frame) or the end (the picture goes on) | `[O]` | P12: Hatari's counters at lines 120 and 300 for 60 Hz written on each line 10-265 and 50 Hz on the next: effects at lines 33 (+29 lines), 62 (no picture), 233 (-29 lines), 262 (+37 lines); the model equals them on those four; line 257 (-5 lines) is a known gap |
| Right border | a line fetches 44 bytes more when the resolution is high at clock 370 and low again by 390, or the sync mode is 50 Hz at 362, 60 Hz at 364 and 50 Hz at 372 | `[O]` | P09 and P10: the +44 appears on the same two frames (resolution) and the same frame (sync mode) as Hatari's, at the 4-clock steps of the sweep |
| Left border | a line fetches 26 bytes more, before the picture, when it begins in high resolution and the register is back to low within 16 clocks | `[O]` | P09: the +26 appears on the same two frames as Hatari's |
| Not modelled | a switch to high resolution inside the picture cuts the line short in Hatari (P09: -106 bytes), some switches near the end of a line lose the next line (-160), 60 Hz on line 257 shortens the picture by 5 lines | `[O]` | P09 and P12 sweeps |
| Floppy DMA burst | a 16-byte burst holds the CPU off the bus for 32 clocks (8 words); `accurate` mode only | `[O]` | P16: a counted loop loses 1060 clocks in a frame in which 528 bytes (33 bursts) moved |
| FDC data register and DMA mode write | an access to `$FF8604` and a write of `$FF8606` take 4 clocks more; a read of `$FF8606`, the address counter and the other registers do not | `[O]` | P18: loop lengths of 16 variants equal Hatari's |
| ACIA access | waits for the E clock (10-clock period, phase as the interrupt acknowledge's) and then 8 clocks more | `[O]` | P18: five alignments of an ACIA read give 50 and 60-clock loops as in Hatari (extra values 6 to 8 fit) |
| Floppy rotation | a revolution is 6267 byte times of 256 clocks (200 ms of CPU clocks); the index pulse lasts 116 bytes; the headers of a 9-sector track are 614 bytes apart (gap III 40) | `[O]` | P17: index low 30280 and high 570 loop runs of 52 clocks, Read Address completions 3568 runs apart |
| Floppy step rates | 20 steps take 6, 12, 2 and 3 ms each at the 8 MHz data sheet times; ours agree with Hatari's to 0.25% | `[O]` | P19 |
| Keyboard answer latency | the answer to `$0D`, `$16`, `$1C`, `$21` is ready 6500 cycles after the command, to a status inquiry 5000 cycles after; the byte spacing is one character time | `[O]` | P20: means of Hatari's figures, which vary by about 30 loop runs between starts |
| MFP transmitter | Hatari sends at once with no RS-232 device; ours takes the data sheet's time (10 bits of 16 timer D periods x 2) | `[D]` | P21: Hatari has no byte time to copy |
| ACSI command | done at once: the completion request is present at the first look | `[O]` | P22: loops 1 for all four commands in both; Hatari leaves the DMA address counter unchanged after a read, ours moves it on as the ACSI notes have it |

## STE and Mega ST (Phase 4)

Every row is `[O]` unless it says otherwise: observed in Hatari 2.6.1 (`--machine ste` with EmuTOS 256K, `--machine megast` with
EmuTOS 192K) with the probe named, or in its I/O trace (`capture_hatari_trace.py --machine`).

| Fact | Value | Label | Source |
|---|---|---|---|
| STE ROM | 256 KB at `$E00000`, the reset vector read through `$0-$7`; nothing at `$FC0000` | `[O]`/`[H]` | EmuTOS 256K's reset PC `$E00030`; *Internals* "1.4 and above" |
| Probes that answer with a bus error on the STE | `$FFFA40-$FFFA5F` (coprocessor), `$FF8C80-$FF8C87` (SCC), `$FF8E00-$FF8E0F` (VME), `$FF8926-$FF89FF` (Falcon sound), `$FFFE00-$FFFE3F` | `[O]` | EmuTOS goes on in Hatari without a trace line for them |
| STE Shifter registers | `$FF820D` video base low, `$FF820F` line offset, `$FF8264` and `$FF8265` pixel scroll: read back as written (bytes); `$FF8201` keeps six bits; `$FF820A` reads with bits 7-2 set, `$FF8260` with them clear | `[O]` | P27 |
| STE palette | 12 bits, a byte write lands in both bytes of the word; a gun's bit 3 is its least significant bit | `[O]`/`[H]` | P27; *Internals* |
| STE video counter | `$FF8205/07/09` are writable (high byte six bits, low byte's bit 0 reads 0); the counter runs on from the written value until the frame loads the base | `[O]` | P27 |
| Line offset and scroll in the fetch | a line fetches `2 x offset` bytes more than 160; with the scroll register `$FF8265` non-zero and `$FF8264` zero it fetches one more 16-pixel group (8 bytes in low resolution); `$FF8264` non-zero gives no extra fetch | `[O]` | P28 |
| Microwire data register | reads the written word shifted left, one bit per 8 clocks from 8 clocks after the write, zero when out; the mask register reads what was written (Hatari shifts 4, 3, 3, 3 bits per read in the tightest loop: not modelled) | `[O]` | P30 |
| Register access cost | the STE's registers cost the plain aligned access, like the ST's Shifter | `[O]` | P36 |

### Blitter (Mega ST and STE, Phase 4)

Register map `[H]` *Atari ST Internals*, the Blitter chapter (transcribed in the Phase 4 ledger): halftone RAM `$FF8A00-1F`,
source x/y increments `$20/$22`, source address `$24` (24 bits), end masks `$28/$2A/$2C`, destination increments `$2E/$30`,
destination address `$32`, counts `$36/$38`, HOP `$3A`, OP `$3B`, control `$3C`, skew `$3D`. The behaviour is `[O]`: Hatari
2.6.1 `--machine ste` with the probes P33 and P37 to P40; the engine (`crates/chips/blitter`) is checked against every
word of those dumps (`tests/timing_probes.rs`).

| Fact | Value | Label | Source |
|---|---|---|---|
| Source read | the source is read only when HOP is 2 or 3 and OP is not 0, 5, 10 or 15, or when smudge is set (also with HOP 1) | `[O]` | P33 |
| Source address per line | a line's source reads advance by the x increment between reads and not after the last; the y increment is added at the line's end only when the line read; the destination likewise (no x increment after the last word) | `[O]` | P33 |
| Counts | the X count reads back as written, the Y count runs down to 0; a count of 0 written means 65536; a start with the Y count left at 0 by the last blit moves nothing and leaves the control register as written | `[O]` | P33, P40 |
| Control register | after a blit the line number has advanced once a line (down when the destination y increment is negative), busy and HOG read clear; smudge stays | `[O]` | P33 |
| Source shifter | 32 bits wide, the older word above the newer, `(older << 16 | newer) >> skew`; it persists between lines and between blits; FXSR reads one word before a line | `[O]` | P33, P37 |
| NFSR | the last word of a line is not read; the shifter takes the last word that crossed the data bus: the destination word read for a partial mask or an operation that uses it, else the word written before; afterwards the shifter's newer word is the word written | `[O]` | P37, P38 |
| Right to left | a negative destination x increment: the newest word enters the upper half, `(newer << 16 | older) >> skew`; masks keep their roles (the first mask on the first word processed) | `[O]` | P39 |
| Smudge | the halftone line is the source word's low four bits | `[O]` | P33 |
| Start | a write of the control register with bit 7 set starts the blit; a write while busy changes nothing | `[O]`/`[H?]` | the busy write is not probed |
| Address registers | bit 0 is not stored | `[H?]` | |
| Memory the Blitter cannot reach | the I/O area and nonexistent memory stop the machine (no fact), as for the DMA; writes to ROM and the cartridge are ignored | `[H?]` | |

### STE DMA sound and the volume control (Phase 4)

Register map `[H]` *Atari ST Internals* (the STE chapter); the behaviour is `[O]`, Hatari 2.6.1 `--machine ste`, probes P29
(registers and the counter's steps, 11 sets of 32 records) and P41 (the byte rate over 6 million clocks).

| Fact | Value | Label | Source |
|---|---|---|---|
| Register masks | `$FF8901` keeps bits 1-0; the frame base and end registers keep 6 bits of the high byte and all of the middle byte, bit 0 of the low byte reads 0; `$FF8921` keeps bits 7 and 3-0; the counter (`$FF8909/0B/0D`) is read only and reads the base while stopped | `[O]` | P29 |
| Even addresses | `$FF8900` reads 0, the other even addresses `$FF` | `[O]` | P29 |
| Mode | `$FF8921` bit 7 is 1 for mono (one byte a frame), 0 for stereo (left byte then right byte); bits 1-0 select 6258, 12517, 25033, 50066 Hz | `[H]`/`[O]` | P29 (the book's table) |
| Rate | a frame every `N * 8010613 / 8021247` PAL CPU clocks (N = 1280, 640, 320, 160), i.e. the NTSC crystal's 8010613 Hz over N whatever the CPU clock | `[O]` | P41: over 6 million clocks the counters agree to 16 bytes |
| Fetch ahead | after `c` bytes were played the DMA has fetched `8 + 2 * ceil(c / 2)` bytes; the first frame is played 640 clocks after the start | `[O]` | P29 (rate 3 within 6 bytes; Hatari moves the visible counter in lumps at scan lines, ours at scan lines with the same rule, within 16 bytes: `known-gap` for byte equality) |
| Frame end | when the fetch reaches the end: play once stops at once (the play bit clears, the counter reads the base again, GPIP line 7 rises); loop goes on from the base; each frame end is a falling edge on timer A's input (23 events in 32 records of 16-byte frames at 50 kHz, as Hatari) | `[O]` | P29 |
| GPIP line 7 | the monitor detect (1 for colour), inverted while playing | `[O]` | P29 |
| Empty frame | with the end not past the base the play bit stays set in loop mode and clears at once for play once; nothing is played | `[O]`/`[H?]` | P29 (register read-back), the rest is the choice |
| Writes while playing | a change of base or end acts at once; a change of the mode restarts the frame timer | `[H?]` | |
| Memory | the DMA reads RAM, ROM and the cartridge; the I/O area and nothing-fitted read `$FF`; no stop, no write | `[H?]` | |
| Volume control | the Microwire word shifted out through the mask's set bits, at least eleven of them: the first two the address `%10` (another address or fewer than eleven bits changes nothing), then optional don't-care bits, the last nine a function (`%000` input select and loudness, `%001` bass, `%010` treble, `%011` volume, `%100` right front fader, `%101` left front fader) and six data bits; applied when the data is written | `[D]` | *LMC1992* Serial Data Entry and Table I; Internals' table for which registers the ST wires |
| Steps | volume D5-D0: 0 = -80 dB to 40 = 0 dB, 2 dB a step (40 to 47 are 0 dB); each front fader D4-D0 (D5 is a don't care): 0 = -40 dB to 20 = 0 dB, 2 dB a step; bass and treble D3-D0: 0 = -12 dB, 6 = flat, 12 = +12 dB, 2 dB a step; the sum of master and fader is the channel gain | `[D]` | *LMC1992* Table I and note 4 |
| Power-up | flat tone, minimum volume (-80 dB) and minimum faders (-40 dB): silent until a program sets them; a reset of the machine does not change it | `[D]` / `[H?]` | *LMC1992* features; the reset is not in the sheet (the chip has no reset input) |
| Input select | data D3 loudness, D2-D0 the input (0 open, 1-4 inputs); the ST wires it as a mixer of the YM and the DMA sound: the YM at full level for 1, 12 dB down for 0, off for 2-3 | `[D]` for the field, `[H?]` for the ST's meaning | *LMC1992* Table I; Internals' wording is not in hand |
| Tone | bass and treble are first-order shelves: boost `(1 + s/wz) / (1 + s/wp)` with the zero at the sheet's turnover (291 Hz bass, 2454 Hz treble), the pole chosen so the gain at 50 Hz (bass) or 15 kHz (treble) is the programmed step (+-12 dB at the ends); a cut is the inverse; at 12 dB the poles come out at 54 Hz and 12.7 kHz, the sheet's inflection frequencies; applied to the mix after the input select (`tone.rs`), not saved in a state | `[D]` / `[H?]` | *LMC1992* electrical characteristics (bass and treble range), Table I note 5, the analog hints' turnover and inflection formulas; `[H?]` the capacitors of the ST's board (the sheet's 0.0047 uF circuit assumed) and the shape between the figures. Not compared with Hatari's tone filters |
| Audio | 44 100 Hz stereo: a zero-order hold of the frame last played, plus the YM through the volume control; the queue keeps two seconds | | |

### STE enhanced joystick ports (Phase 4)

`[O]` Hatari 2.6.1 `--machine ste`, probe P31 and `timing/scan_p31.py` (which addresses answer). Headless Hatari takes no input,
so only the rest state and the write behaviour are observed; the bits a held button clears are `[H?]`.

| Fact | Value | Label | Source |
|---|---|---|---|
| Rest | `$FF9200` and `$FF9202` read `$FFFF`, `$FF9201` and `$FF9203` `$FF`; the paddles `$FF9210-$FF9217` read `$FF` at even and `$24` at odd addresses; the light pen words `$FF9220/22` read 0 | `[O]` | P31 |
| Writes | accepted and ignored: the reads do not change after writing `$FFFF`, 0 or `$A5A5` to `$FF9200` and `$FF9202` | `[O]` | P31 |
| Bus errors | `$FF9218-$FF921F`, a byte read of `$FF9221` or `$FF9223`, and everything from `$FF9224` | `[O]` | scan_p31.py (the last through the decode table: nothing answers) |
| Pad buttons | active low as at rest: up, down, left, right are bits 0-3 of the pad's nibble of `$FF9202`'s low byte (pad 1 low nibble, pad 2 high); A and B of pad 1 are bits 0 and 2 of `$FF9201`, of pad 2 bits 1 and 3 (*Internals*: Fire 0, 2, 1, 3 from bit 0); C, Option and Pause are bits 0-2 (pad 1) and 4-6 (pad 2) of `$FF9200`'s high byte | `[H?]` | *Internals* register table; the scan matrix of the real ports is not modelled |
| Key names | `Joy1PadUp`...`Joy2PadPause` on the STE only | | |

### Mega ST real-time clock (Phase 4)

An RP5C15 at `$FFFC20-$FFFC3F`; `[O]` Hatari 2.6.1 `--machine megast` (probe P32, a scan of the area, and EmuTOS's own accesses
in the I/O log). Hatari's time registers follow the host clock, so only the other registers are compared; here the time is
the plugin option `rtc_time` counted in machine time (`rtc_mode = stopped` freezes it until the program sets Timer EN). The
*RP5C15* data sheet (address mapping, mode, 12/24, leap year, reset and ADJ descriptions) was added 2026-10-24 and settles
what Hatari does not show; where it and Hatari differ the sheet wins and the difference is listed under "Differences from
Hatari" below.

| Fact | Value | Label | Source |
|---|---|---|---|
| Access | the even addresses read `$FF`, nothing bus-errors, words read `$FF` then the odd register; a register sits at `$FFFC21 + 2n` | `[O]` | scan |
| Time registers | 0-12 (bank 0): seconds, minutes, hours, weekday, day, month, year since 1980, ones digit then tens digit, upper nibble 0; EmuTOS reads them twelve at a time and got `2001-02-03 04:05:06` as digits `6 0 5 0 4 0 / 6 / 3 0 / 2 0 / 1 2` | `[O]` | the host scan, EmuTOS's reads |
| Mode register 13 | keeps four bits, reads with the upper nibble `$F` | `[O]` | P32 |
| Test and reset registers 14, 15 | keep four bits, read with the upper nibble 0 | `[O]` | P32 |
| Alarm bank | bit 0 of the mode register selects bank 1; its registers 2 and 3 read back what was written with the upper nibble `$F` (`$FF`, `$F0`, `$F5`): EmuTOS writes `$0A` and `$05` there and reads them back to find the clock | `[O]` | P32, the I/O log |
| Bank 1 map | 0 clock output select (three bits), 1 ADJ, 2 and 3 minute alarm, 4 and 5 hour alarm, 6 weekday alarm, 7 and 8 day alarm, 9 and 12 nothing (read 0), 10 12/24 hour select, 11 leap year counter, 13-15 as in bank 0; a register keeps only the bits the chip has (3: 3, 5: 2, 6: 3, 8: 2 bits) and reads with the upper nibble `$F`; the alarm does nothing (not wired on the ST) | `[D]` | *RP5C15* address mapping; Hatari returns the time at 0, 1, 4-9 and 12 |
| Mode register bits | D3 Timer EN, D2 Alarm EN, D1 not implemented (reads 0), D0 bank 1 | `[D]` | *RP5C15* MODE register |
| Timer EN | clear: the seconds counter and everything above it hold, the carry from the divider below is held too, so a pause shorter than a second loses nothing; set again: counting goes on from the held part of the second. The clock the option stopped starts when Timer EN is set. Power-on state of the bit is not in the sheet: the clock runs until a program clears the bit | `[D]` / `[H?]` | *RP5C15* MODE register note |
| 12/24 hour select | register 10 bit 0: 1 = 24 hours, 0 = 12 hours; in 12 hour mode the 10-hour digit's bit 1 is set for p.m. | `[D]` | *RP5C15* 12/24 select register |
| Hour 0 in 12 hour mode | shows 12 (a.m.) | `[H?]` | the sheet does not give the hour 12 and 0 cases |
| Leap year counter | register 11 bits 1-0, leap year at 0, counts in step with the year counter; reads `(year + offset) mod 4`, a write sets the offset; the calendar here is always the true one (a counter set out of step does not change February) | `[D]` / `[H?]` | *RP5C15* leap year counter |
| ADJ | bank 1 register 1 bit 0 written 1: seconds 00-29 become 00, 30-59 become 00 with a carry into the minutes; the divider below the seconds starts again; the bit reads 0 (the sheet's 100 us completion time is not modelled) | `[D]` / `[H?]` | *RP5C15* ADJ |
| Reset register 15 | bit 0 set clears all alarm registers, bit 1 set restarts the divider below the seconds counter; bits 2 and 3 are the 16 Hz and 1 Hz outputs (not wired); the register keeps its four bits and reads them back (the sheet says write only) | `[D]` / `[O]` | *RP5C15* reset controller; P32 |
| Writes to the time | a digit write takes effect when it makes a valid field, else it is ignored; the weekday follows the date | `[H?]` | the sheet does not say what an invalid digit does; Hatari ignores all time writes |
| Range | 1980 to 2079 | `[H]` | the year register is two digits since 1980 |

Differences from Hatari 2.6.1, all following the data sheet: the mode register's bit 1 reads 0 (Hatari keeps it); the 10-minute
alarm register has three bits (Hatari four); bank 1 registers 0, 1, 9 and 12 are the clock output select, ADJ and nothing (Hatari
returns the time); Timer EN, ADJ and the reset register's bits 0 and 1 act (Hatari ignores them).

### Blitter on the bus (Phase 4)

`[O]` Hatari 2.6.1 `--machine ste`, probe P34 (a counted loop's iterations per frame while blits of known size run, six blits;
control byte and GPIP at every VBL). The count of nops a word takes is `[H]` *Internals* ("Blitter Execution Times"), checked by
P34 for (HOP, OP) = (0, 0), (2, 3), (3, 1), (1, 5): 1, 2, 3 and 2 nops of 4 clocks.

| Fact | Value | Label | Source |
|---|---|---|---|
| Clocks a word costs | 4 times the nops of the (OP, HOP) table: 1 for OP 0 and 15 and for OP 3 and 12 with HOP 0-1; 2 for OP 5 and 10, for HOP 0-1 with the others, and for OP 3 and 12 with HOP 2-3; 3 for the other operations with HOP 2-3 | `[H]`/`[O]` | Internals; P34 |
| HOG set | the CPU is stopped for the blit's words (20000 words took 80000, 160000 or 240000 clocks); nothing else runs, interrupts wait | `[O]` | P34 (frames of 6669 iterations lost to within the setup instructions) |
| HOG clear | the Blitter takes the bus for 64 clocks of words and 2 clocks to hand it over, then the CPU runs 96 clocks: the CPU keeps 59.2 % (Hatari's loop counts 3942-3953 of 6669 a frame; ours 3947-3949); a blit of 100000 words at 8 clocks took 12.6 frames, as ours | `[O]` | P34 |
| Busy and the line number while it runs | the control byte reads busy and the line number as it advances, the same value at every VBL as ours | `[O]` | P34 (all 105 records equal) |
| GPIP line 3 | high while the Blitter is busy; the falling edge at the end is the "blitter done" interrupt (channel 3 of the B registers) | `[O]` | P34 (`$FFFA01` reads `$B9` while busy, `$B1` after) |
| Bounds | a hogging blit runs in bursts of at most a frame of clocks with a CPU slice between them, so a hostile count costs bounded host time a frame; words past the address space stop the machine | | the choice |
| `fast` mode | the same turns, taken between instructions (the CPU is charged a hogging blit with the instruction that starts it) | | the choice |

### Memory watchpoints (Phase 4, a debugging aid: no hardware fact)

The plugin option `watch` (`$1000-$10FF:w,$400:rw`, up to 16 comma separated entries, `ADDR` or `ADDR-ADDR` inclusive, `r`, `w`
or `rw`) stops the machine after the first *data* access that touches a watched byte (a word access touches two); instruction
fetches (function codes 2 and 6) and the Blitter and DMA do not trip it. The stop is the machine's ordinary one (`StopReason::Watch`):
the access is made, the report names the access, address, value, PC and cycle. The list is configuration, not state: a loaded
state keeps the list of the machine it is loaded into. With no watch the bus checks one flag.

## Engineering Hardware Specification (2026-10-24)

Facts from the *ST spec* (page numbers are the document's own) that the sections above do not already carry, and what they leave
open. `[D]` means the document states it for the original ST; the STE and Mega ST rows say so.

| Fact | Value | Label | Source |
|---|---|---|---|
| RAM test at power-up | the RAM configuration is found by writing `$000a` to the memory configuration register, writing a pattern at `$000000-$0001FF` and reading `$000200`, `$000400` and `$000000` back: a match at `$200` means a 128K bank, at `$400` 512K, at `$0` 2M; the same on `$200000` for bank 1, a miss meaning no bank 1 | `[D]` | *ST spec* pp.4-5; the aliasing of the DRAM rows above is what this test sees |
| RAM and video share the bus | RAM cycles are interleaved between the CPU and the video controller, "the MPU still maintains full memory bandwidth" | `[D]` | *ST spec* p.5; the 4-clock alignment of the `accurate` mode is observed `[O]` |
| DMA | one counter and one channel: a single DMA operation at a time; the bus goes to the DMA and the CPU first come, first served, and the access stays until the operation is complete or control is relinquished | `[D]` | *ST spec* pp.5-6 |
| DMA registers at reset | base address and counter 0, sector count 0, status 1 (error bit set = no error), mode control not affected | `[D]` | *ST spec* p.5 |
| DMA status bits | bit 0 "_Error Status", bit 1 "_Sector Count Zero Status", bit 2 "_Data Request Inactive Status": a name with a leading underscore is active low, so 1 = no error, 1 = count not zero, 1 = data request active | `[D]` | *ST spec* p.28; agrees with the observed `[O]` polarity of bit 0 above (*Internals* p.58 has it the other way round) |
| Video planes | plane 0 is the least significant bit of the colour index; display memory holds the words of the planes in turn (4 planes: 0 1 2 3, 2 planes: 0 1, 1 plane: 0); the screen origin is bit 15 of the first word | `[D]` | *ST spec* pp.7-9 |
| Monochrome | the palette is bypassed; bit 0 of palette entry 0 inverts the picture ("normal video is black 0, white 1"); the border is always black; in multi-plane modes the border is palette entry 0 | `[D]` | *ST spec* p.8 (the emulator draws the border black on the mono monitor since 2026-10-24) |
| Video mode register | the shift mode should be selected during vertical blanking | `[D]` (advice) | *ST spec* p.15 |
| Horizontal blanking counter | MFP timer B in event count mode counts display enable, "the first of which occurs at the end of the first display line"; timer A is reserved for applications on the ST, and on the STE its event input counts DMA sound frames | `[D]` | *ST spec* pp.14-15; *STe manual* §2.8.2 |
| Interrupt priorities | MFP channels, highest first: 15 mono detect, 14 RI, 13 timer A (the table calls it "System Clock / BUSY"), 12 receive buffer full, 11 receive error, 10 transmit buffer empty, 9 transmit error, 8 timer B, 7 FDC (I5), 6 keyboard and MIDI (I4), 5 timer C, 4 timer D, 3 GPU done (I3), 2 CTS, 1 DCD, 0 Centronics BUSY (I0); the ACIA's IRQ status bit tells the keyboard from MIDI | `[D]` | *ST spec* p.31 |
| PSG | register select is write only ("reset: registers all zeros"); `$FF8800` read gives the selected register, which for port B is the parallel interface data | `[D]` | *ST spec* pp.10, 29 |
| RS232 | RTS and DTR out of PSG port A, CTS, DCD and RI into the MFP; the baud rate generator is timer D with the MFP's 2.4576 MHz, 50 to 19200 baud; one byte buffers in each direction | `[D]` | *ST spec* p.16 |
| DMA programming notes | the sector count register should be selected (mode bit 4) before the status error bit is tested; the end of a floppy command is found by polling GPIP I5 (the FDC interrupt), not the FDC busy bit or the DMA sector count zero bit; the 30 ms settle bit must not be set on type II and III commands; a force interrupt should follow a time-out; the motor-on status must go low before a drive is deselected | `[D]` (advice) | *ST spec* p.36 notes 841017F-J; the first one is not modelled: the error bit is the same whichever register is selected |
| Floppy | two daisy-chained drives; 512-byte sectors on floppy and hard disk; "the detection of floppy disk removal is not supported in hardware"; the controller is a WD1772 | `[D]` | *ST spec* pp.18-19 |
| Floppy lines | index pulse, track 00 and write protect are active low with 1 kohm pull-ups (a missing drive reads inactive); motor on, direction, step, write data and write gate are inverted | `[D]` | *ST spec* p.21 |
| ACSI commands | the six-byte block: byte 0 operation code (bits 4-0) and controller number (bits 7-5), byte 1 block address high (bits 4-0) and device number (bits 7-5), bytes 2 and 3 block address, byte 4 block count, byte 5 control; the opcodes of 1986: `$00` test unit ready, `$05` verify track, `$06` format track, `$08` read, `$0A` write, `$0B` seek, `$0D` correction pattern, `$15` mode select, `$1A` mode sense ("subject to change"); `$05`, `$06`, `$08` and `$0A` are multi-sector transfers with an implied seek. Not implemented here: `$05`, `$06`, `$0D`, `$15` answer "invalid command" | `[D]` | *ST spec* pp.19-20 |
| Keyboard | the ikbd is a 1 MHz HD6301V1; the mouse is 100 counts per inch (4 per mm) at most 10 inches a second; two joystick ports, port 0 shared with the mouse | `[D]` | *ST spec* pp.12-13 |
| System startup (STe) | after a reset the GSTMCU maps the first 8 bytes of the ROM at `$E00000` to 0-7; TOS then reads the longword at `$FA0000` and runs the cartridge when it is the magic number, checks for a warm start, initialises the PSG and deselects the drives, sets the palette and the screen address, zeroes memory on a cold start, sets up its variables, the vectors and the MFP, sets the resolution, boots a floppy then a hard disk | `[D]` | *STe manual* §2.13 |
| STE sound path | the DMA fetches its samples during horizontal blanking, transparent to the CPU; the DAC output goes through a four-pole switched low-pass filter at 40 % of the sample rate and a two-pole fixed 16 kHz filter before the LMC1992; the stereo output is also mixed into the monitor's audio and the PSG into both | `[D]` / not modelled | *STe manual* §2.10; the filters are not modelled (the output is a zero-order hold) |
| STE joystick ports | four ports read by the CPU; directions of joystick 0 and 2 are read/write: written values are driven until a read, and not driven after a read until a write; paddles on joysticks 0 and 1, their triggers are bits 1 and 2 of `$FF9202`; a light gun or pen on joystick 0 reports X in pixels of the 320-wide mode (shift left once for 640x200, twice for 640x400) | `[D]` prose, `[H?]` for the exact bits | *STe manual* §2.12.8; the register list the manual refers to is not in it, so the pad bit assignments above stay as observed |
| STE video | 16 intensity levels a gun (4 bits, 4096 colours); the video base may start on any word boundary | `[D]` | *STe manual* §2.9.1, §2.9.2 |

What the document leaves open: the level an empty cartridge slot reads, the DMA FIFO's second half, how a reserved I/O address
is decoded (it says only "a bus error"), the YM2149 and the input port read-back, the STE and Mega ST registers (it is older than both),
the Blitter.
