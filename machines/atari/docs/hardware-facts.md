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
| Unused bits read back | palette `$F888` bits, `$FF820A` bits 7-2 and `$FF8260` bits 7-2 read as 0 | `[H?]` | not documented in *Internals* |
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
| GPIP I1-I4, I6 | low | `[O]` | same read; I4 (ACIA) is low before EmuTOS resets the ACIAs |
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
