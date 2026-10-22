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
| *YM2149* | Yamaha YM2149 catalog, September 1992, https://bitsavers.org/components/yamaha/YM2149_199209.pdf (SHA-256 `a4b946bf525fc1a7c6dec9e7662ce2342d3a3c627f980c7c3b1a396df7e91d20`) |

The Atari *ST Hardware Specification* (1986) is not available from here (HTTP 403 from cpcwiki); every `[H?]` and
`[O]` row below should be checked against it first.

## Memory map and decode (`io.rs`, `bus.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| I/O area | `$FF8000-$FFFFFF`, supervisor only, user access = bus error | `[H]` | *Internals* p.55 |
| Device pages | `$FF8000` memory configuration, `$FF8200` video, `$FF8400` reserved, `$FF8600` DMA, `$FF8800` sound, `$FFFA00` MFP, `$FFFC00` ACIAs | `[H]` | *Internals* p.55 |
| `$FF8006` | bus error | `[O]` | *Hatari io*: "Bus error reading at address $ffff8006 PC=$fc007c" |
| `$FF8A3C` | bus error (no Blitter on the ST) | `[O]` | *Hatari io*: "Bus error reading at address $ffff8a3c PC=$fc0638" |
| `$FF860F` byte | open: reads `$FF`, no bus error | `[O]` | *Hatari io*: "IO read.b $ffff860f = $ff pc=fc0638" |
| `$FFFC21`, `$FFFC25`, `$FFFC27`, `$FFFC3B` bytes | open: read `$FF`, writes ignored, no bus error (EmuTOS's Mega ST clock probe) | `[O]` | *Hatari io* lines 5-12 |
| Any other I/O address | unknown: the machine stops | — | — |
| `$400000-$4FFFFF` with 4 MB | bus error | `[O]` | *Hatari io* `--memsize 4`: "Bus error reading at address $4fffff" (agrees with slice 1's nonexistent-memory rule) |

## MMU (`mmu.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Register | `$FF8001`, byte, read/write; bits 3-2 bank 0, bits 1-0 bank 1: `00` 128K, `01` 512K, `10` 2M, `11` reserved | `[H]` | *Internals* pp.55-56 |
| Bits 7-4 | read back as 0 | `[H?]` | Hatari reads `$00` at power-on; nothing documents the upper bits |
| Power-on value | `$00` | `[O]` | *Hatari io* line 1 "IO read.b $ffff8001 = $00" |
| Banks fitted per RAM size | 256K = 128K + 128K, 512K = 512K + none, 1M = 512K + 512K, 2M = 2M + none, 2.5M = 2M + 512K, 4M = 2M + 2M | `[O]` | final `$FF8001` written by EmuTOS per `--memsize`: `$00`, `$04`, `$05`, `$08`, `$09`, `$0A` |
| Address multiplexing | the word address splits into a column (low n bits) and a row (next n bits), n = 8/9/10 for a bank configured 128K/512K/2M; smaller chips ignore the top lines, so in a 2M bank `$408` aliases `$8` on 512K and `$208` on 128K | `[H?]` | DRAM row/column addressing (16 chips of 64K/256K/1M bits, *Internals* p.55); agrees with the offsets EmuTOS 1.4's sizing checks (`$40008`, `$208`, `$80008`, `$408`, `$8` at `$FC01BC-$FC0202`) and with Hatari's final value for all six sizes |
| No RAM at an address (above the configured total, or an empty bank) | reads `$FF`, writes ignored, no bus error | `[H?]` | slice 1 choice; a forum report says writes vanish and reads return bus noise |

## Shifter (`st-shifter`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Video base | `$FF8201` bits 23-16, `$FF8203` bits 15-8; 256-byte steps; read/write | `[H]` | *Internals* p.56 |
| Video address counter | `$FF8205/7/9`, read only | `[H]` | *Internals* p.56 |
| Counter timing | unknown: an access stops | — | — |
| Sync mode | `$FF820A` bit 1: 0 = 60 Hz, 1 = 50 Hz; bit 0: external sync | `[H]` | *Internals* p.57 |
| Palette | 16 words `$FF8240-$FF825E`, `$0RGB` with 3 bits per gun (`$700`, `$070`, `$007`); medium resolution uses 0-3; read/write | `[H]` | *Internals* p.57 |
| Unused bits read back | palette `$F888` bits and `$FF820A` bits 7-2 read as 0 | `[H?]` | not documented in *Internals* |
| `$FF8260` bits 7-2 | read as 1 | `[O]` | Hatari reads `$FC` in low resolution (EmuTOS at `$FC69C2`) |
| Resolution | `$FF8260` bits 1-0: 0 = 320×200 4 planes, 1 = 640×200 2 planes, 2 = 640×400 1 plane | `[H]` | *Internals* p.58 |
| Resolution 3 | treated as a black picture | `[H?]` | reserved |
| High resolution colour | palette 0 bit 0: clear = black on white, set = inverted | `[H]` | *Internals* p.58 |
| Bitplanes | each 16-pixel group is N consecutive words (plane 0 first), most significant bit = leftmost pixel | `[H]` | *Internals* video RAM chapter ("Video display"), and the screen formats in *TOS hyp* |
| Colour monitor in high resolution, mono monitor in low or medium | no picture (black) | `[H?]` | *Internals* p.57: "High-res operation puts the ST at a 70 Hz screen frequency" |
| 3-bit level to 8-bit RGB | `level * 255 / 7` | — | a display choice, not a hardware fact |

## Timing and interrupts (`scheduler.rs`, `machine.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| Colour frame | 50 Hz: 512 clocks × 313 lines; 60 Hz: 508 × 263 | `[H]` | *forum*, cited in `scheduler.rs` |
| Mono line | 224 clocks | `[H]` | *forum* |
| Mono lines per frame | unknown: `--monitor mono` is refused | — | — |
| Sync mode at power-on | 50 Hz on a PAL machine, 60 Hz on an NTSC one | `[H?]` | TOS writes the register before showing anything (*Hatari io* line 39) |
| HBL | level 2 autovector, vector `$68` | `[H]` | *TOS hyp* |
| VBL | level 4 autovector, vector `$70` | `[H]` | *TOS hyp* |
| MFP | level 6, vectored | `[H]` | *TOS hyp* (`$78` "Level 6, MFP"), *Internals* p.33 |
| MFP channels | 0 BUSY, 1 DCD, 2 CTS, 3 Blitter done, 4 Timer D, 5 Timer C (200 Hz), 6 ACIA, 7 FDC/ACSI, 8 Timer B (display enable), 9-12 USART, 13 Timer A, 14 RI, 15 mono detect | `[H]` | *TOS hyp* vectors `$100-$13C`; *MC68901* §3 channel order |
| MFP timer clock | 2 457 600 Hz | `[H]` | *ST-Log 34*: "the 68901 timers count cycles of a 2.4576 MHz input clock"; consistent with TOS's 200 Hz Timer C programmed ÷64 × 192 (*Hatari io* lines 70-74) |
| TBI | display enable; not connected yet (its timing is unknown) | — | *TOS hyp* `$120` |
| GPIP wiring | I0 Centronics BUSY, I1 DCD, I2 CTS, I3 reserved, I4 keyboard and MIDI ACIAs, I5 FDC and HDC, I6 RI, I7 mono detect; Timer B input counts lines with display enable | `[H]` | *Internals* p.61 |
| GPIP I7 | mono detect: low = mono monitor | `[H]` | *Internals* p.~95 (video connector pin 4) |
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
| Port A | bit 0 side select, 1 drive 0 select, 2 drive 1 select, 3 RTS, 4 DTR, 5 Centronics strobe, 6 general output, 7 reserved | `[H]` | *Internals* p.60 (read with `pdftotext -raw`; the layout text shifts the rows) |
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
| Clock | 500 kHz | `[H]` | *Internals* p.47 |
| Clock ratio | CPU clock / 16; a bit is `divide × 16` CPU cycles | `[H?]` | 8 MHz / 16 = 500 kHz; Hatari uses 501 327 Hz, the same ratio |
| Line speeds | keyboard ÷64 (7812.5 baud), MIDI ÷16 (31 250 baud), 8N1 | `[H]` | *Internals* p.47 (set by software through the control register) |
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
| Mouse buttons in joystick mode | fire buttons, no mouse records: right = joystick 1, left = joystick 0 | `[H]` §8 for joystick mode, `[H?]` for which button | *Internals* p.69 (the buttons share the fire lines) |
| Stick bits | bit 0 up, 1 down, 2 left, 3 right | `[H?]` | the document and *Internals* p.77 say only "a bit for each direction" |
| Scan codes | Appendix A, 95 keys | `[H]` | *IKBD* Appendix A (page images) |
| Version code | `$F1` | `[O]` | *Hatari io* line 148; *IKBD* §8 allows `$F0` (first release) or `$F1` (second) |
| Reset reply delay | 502 002 CPU cycles after the RESET's second byte (or power-up) | `[O]` | Hatari `--trace ikbd_all,acia` (VBL 6 cycle 150 494 to VBL 10 cycle 11 472); *IKBD* §9.1 bound 300 ms |
| Power-up reply | sent while the ACIA is still in reset, so it is lost | `[O]` | Hatari: "ikbd acia not initialized, can't send byte=0xf1" |
| Clock at power-up | 00-00-00 00:00:00, BCD, one-second steps | `[O]` | Hatari: `$1C` at 1.2 s answers `00 00 00 00 00 01` |
| Calendar carries | two-digit year, leap when divisible by 4 | `[H?]` | not documented |
| Output queue | 256 bytes; a packet that does not fit is dropped | `[H?]` | *IKBD* §9.14 "<tbd>"; the 6301 has 128 bytes of RAM (*Internals* p.69) |
| Memory load/read/execute (`$20-$22`) | consumed; reads answer zeros; nothing executes | `[H?]` | the 6301's memory is not modelled |
| Joystick monitoring, fire monitoring, joystick key code (`$17-$19`) | parameters consumed, no reports | `[H?]` | Phase 2 |
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
| `$FF8606` write (mode) | bit 1 A0, bit 2 A1, bit 3 HDC, bit 4 sector count, bit 6 DMA off, bit 8 write | `[H]` | *Internals* p.59 (with `pdftotext -raw`) |
| Mode bit 7 | described as FDC/HDC as well; not decoded | `[H?]` | *Internals* lists bits 3 and 7 both as FDC/HDC |
| `$FF8606` read (status) | bit 0 set = no error, bit 1 sector count not zero, bit 2 FDC DRQ; other bits 0 | bits 1-2 `[H]`, bit 0 `[O]`, the rest `[H?]` | *Internals* p.58 prints bit 0 "0=no error"; Hatari reads `$..b1` after a good transfer and EmuTOS retries a read when bit 0 is clear, so the book is taken to be wrong here. Hatari's upper bits are leftovers (`$79b1`), masked in the comparisons |
| DMA address | `$FF8609` high, `$FF860B` mid, `$FF860D` low; read and write; written low first | `[H]` | *Internals* p.59 |
| FIFO | 16 bytes, written to memory a burst at a time; no bus stall in Phase 1 | `[H?]` | design §8 |
| Sector count | 8 bits, counts 512-byte sectors; the DMA stops taking bytes at 0 | `[H?]` | — |
| Write bit toggled | clears the FIFO and the error bit | `[H?]` | EmuTOS's `$0190`/`$0090` pattern |
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
| Selects | PSG port A bit 1 drive 0, bit 2 drive 1, active low; bit 0 side (clear = side 1) | `[H]` *Internals* p.60; polarity `[H?]` | `[O]` TOS writes `$07` to deselect, `$05` for drive A |
| Revolution | 6250 byte times | `[H?]` | 300 rpm, 250 kbit/s |
| Index pulse | the first 64 byte times of a revolution, only with a disk | `[H?]` | — |
| No disk | no index pulses, no data, write-protected | `[O]` | status `$E4` |
| A disk | not write-protected | `[O]` | Hatari statuses `$24`, `$A4`, `$A6` with a disk |
| Disk swap | write-protect reads set for 0.5 s (15 625 byte times) after a disk goes in or out while running; nothing for a disk there from power-on | `[H?]` | the sensor is blocked by the moving disk; TOS detects a change by it |
| Motor | the motor line reaches both drives | `[H?]` | — |
| Head range | cylinders 0-85 | `[H?]` | — |
| Drives fitted | 2 by default, 1 with `drives = 1`; drive B absent then | `[H?]` | — |

## USART, RS-232, MIDI and printer (`mc68901` `usart.rs`, `serial.rs`)

| Fact | Value | Label | Source |
|---|---|---|---|
| UCR | CLK (bit 7: divide by 16), WL1 WL0 (bits 6-5: 8, 7, 6, 5 bits), ST1 ST0 (bits 4-3: `00` synchronous, `01` 1 stop, `10` 1.5 stop, `11` 2 stop), PE (bit 2), E/O (bit 1) | `[D]` | *MC68901* §6.1.3 Figure 6-2 |
| Baud clock | timer D drives the USART clocks; a timer output toggles on each time-out, so one clock period is two time-outs; a bit is one period (CLK 0) or sixteen (CLK 1) | `[D]` `[H]` | *MC68901* §5.1.1 (output toggles), §6.1 (divide by 16); *Internals* p.61 ("Timer D sends the timing signal for the MFP's built-in serial interface") |
| Character time | (1 start + data bits + parity + stop bits) bit periods | `[D]` | *MC68901* §6.1.1, Figure 6-2 |
| RSR | BF 7, OE 6, PE 5, FE 4, F/S or B 3, M/CIP 2, SS 1, RE 0; BF set when a word reaches the buffer and cleared by reading UDR; OE rises when the held-back overrun word meets a read buffer, clears on an RSR read, and blocks new words until then; RE = 0 clears every flag at once | `[D]` | *MC68901* §6.2.2 Figure 6-3, §6.2.3 |
| Parity, framing, break | never raised: the peers deliver clean frames | `[H?]` | a choice, no line noise is modelled |
| TSR | BE 7, UE 6, AT 5, END 4, B 3, H 2, L 1, TE 0; a write to UDR clears BE, BE sets when the word moves to the shift register; UE sets when a word ends with the buffer empty, cleared by reading TSR or disabling; disabling with an idle shifter sets END at once, with a word going out after it; enabling clears END; AT turns the receiver on after the last word | `[D]` | *MC68901* §6.3, §6.3.2 Figure 6-4 |
| Loopback | H = L = 1: the transmitter feeds the receiver, the receive clock is the transmit clock | `[D]` | *MC68901* §6.3.2 |
| Break transmission | the B bit stops words moving out of the buffer; no break is shown to a peer | `[H?]` | *MC68901* §6.3 describes the line state, the machine has no line level |
| Start of a word | the buffer is moved to the shift register at the next time-out of timer D | `[H?]` | §6.3 says only "a delay before the first bit" |
| Interrupt channels | 9 transmit error (UE, END), 10 transmit buffer empty, 11 receive error, 12 receive buffer full; an overrun uses 12 when 11 is disabled | `[D]` | *MC68901* Table 3-1, §6.2.1 |
| Reset | UCR, RSR, SCR cleared, words in flight stopped; UDR and TSR keep their values | `[D]` / `[H?]` | §2.2.3 lists TSR among the registers reset leaves alone; Figure 6-4 says reset clears TE: the register list is followed |
| Synchronous format | not modelled: a transmitter in it never finishes a word | `[H?]` | the ST does not use it |
| MIDI line | the MIDI ACIA at ÷16: 256 CPU cycles a bit, 2560 a character | `[H]` | *Internals* p.47 and section 2.5 (31 250 baud, 500 kHz / 16) |
| Centronics | data lines are PSG port B; the strobe is PSG port A bit 5, a low pulse; BUSY is GPIP I0, high when the printer cannot take a byte | `[H]` | *Internals* p.59 (port A list, read one position down: the page's column is shifted), pp.88-89, p.61 |
| Strobe edge | the byte on port B is taken when port A bit 5 falls between two states in which port A is an output | `[H?]` | the document says only that a low pulse marks the data valid; either edge sees the same byte |
| BUSY | low with a printer connected, high without; no busy time after a byte | `[H?]` | the document gives no timing |
