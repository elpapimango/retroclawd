# Mega Drive / Genesis hardware facts

Every hardware fact the code uses has a row here. Labels: **[D]** data sheet; **[H]** documentation (a named document);
**[O]** observed by a probe on two or more reference emulators (probe id given); **[H?]** unresolved or from memory: never
coded until it has a source. Sources are in `sources.md` (short ids). Reference emulators may be read for understanding, but a
fact learnt from their source stays **[H?]** until a document or a probe on two oracles confirms it (spec section 1).

Document copies live in `software/docs/` (git-ignored); this file cites them by short id and page.

## 1. Clocks and frame structure

| # | Fact | Label | Source |
|---|---|---|---|
| C1 | Master clock 53.693175 MHz (NTSC), 53.203424 MHz (PAL) | [H] | SR-TECH "System master clock rate" |
| C2 | 68000 clock = master / 7 (7.670454 MHz NTSC, 7.600489 MHz PAL) | [H] | SR-TECH CPU; ratio equals C1 / listed rate |
| C3 | Z80 clock = master / 15 (3.579545 MHz NTSC, 3.546894 MHz PAL) | [H] | SR-TECH CPU |
| C4 | YM2612 clock = the 68000 clock; output rate 53.267 kHz NTSC, 52.781 kHz PAL (= YM clock / 144) | [H] | SR-TECH Audio |
| C5 | PSG (SN76489 variant, inside the VDP) clock = the Z80 clock | [H] | SR-TECH Audio |
| C6 | VDP clock 13.423294 MHz NTSC (master / 4); pixel clock 6.711647 MHz (master / 8) in the figure SR-TECH lists; H40 slows the dot clock (master / 10 over the visible part) | [H?] | SR-TECH lists master/8; the H40 divider must come from a document or a probe (P-VDP-DOT) |
| C7 | 3420 master clocks per scanline | [H] | SR-TECH "Master clock cycles per scanline" |
| C8 | NTSC: 262 lines per frame, 896 040 master clocks, 59.92274 Hz | [H] | SR-TECH; 262 x 3420 = 896 040 |
| C9 | PAL: 313 lines per frame = 1 070 460 master clocks, 49.701459 Hz | [H] / conflict | Kabuto "Picture size" (313 total) and SR-TECH refresh rate (53 203 424 / 49.701459 = 1 070 460). SR-TECH also prints "1,067,040 cycles per frame" = 312 lines, which contradicts its own refresh rate: **the 313-line figure is used; the 1 067 040 figure is an error in SR-TECH** (to be shown by P-FRAME on two oracles) |
| C10 | NTSC picture lines (V28): 11 top border, 224 active, 8 bottom border, 3+3+3 sync, 10 blank; V30 gives 8 more lines from each border | [H] | KABUTO "Picture size" |
| C11 | PAL picture lines: 38 top border, 224 active, 32 bottom border, 19 sync/blank | [H] | KABUTO "Picture size" |
| C12 | Horizontal: 13 extra left border and 14 extra right border pixels | [H] | KABUTO "Picture size" |
| C13 | The 68000 loses about 2 of every 128 cycles to bus activity and runs at roughly 480 cycles per line, not 488.57 | [H] | KABUTO "The bus system" (Phase 3 detail; fast mode ignores it) |

## 2. 68000 address map (the main CPU)

| # | Range | What | Label | Source |
|---|---|---|---|---|
| M1 | `$000000-$3FFFFF` | cartridge ROM / RAM | [H] | SR-MAP |
| M2 | `$400000-$9FFFFF` | reserved (Mega CD / 32X) | [H] | SR-MAP |
| M3 | `$A00000-$A0FFFF` | Z80 address space | [H] | SR-MAP |
| M4 | `$A10001` | version register | [H] | SR-MAP |
| M5 | `$A10003 / 5 / 7` | data of ports 1, 2, expansion | [H] | SR-MAP, PD-IO |
| M6 | `$A10009 / B / D` | control of ports 1, 2, expansion | [H] | SR-MAP, PD-IO |
| M7 | `$A1000F, $A10011, $A10013` etc. | serial transmit/receive/control of the three ports | [H] | PD-IO "Serial mode port addresses" |
| M8 | `$A11000` | memory mode register | [H] | SR-MAP |
| M9 | `$A11100` | Z80 bus request | [H] | SR-MAP, PD-Z80 |
| M10 | `$A11200` | Z80 reset | [H] | SR-MAP, PD-Z80 |
| M11 | `$A130F1` SRAM access; `$A130F3..FF` bank registers (SSF2 mapper) | [H] | SR-MAP |
| M12 | `$A14000-$A14003` TMSS "SEGA"; `$A14101` TMSS/cartridge register | [H] | SR-MAP |
| M13 | `$C00000/2` VDP data; `$C00004/6` VDP control; `$C00008-$C0000F` HV counter; `$C00011-$C00017` PSG; `$C0001C/E` debug register | [H] | SR-MAP |
| M14 | `$FF0000-$FFFFFF` 64 KB work RAM, repeated over `$E00000-$FFFFFF` | [H] | SR-MAP; SR-TECH "Main RAM: 64 KB (repeated over the upper 2 MB)" |
| M15 | Read of a `$A1xxxx` address outside M4-M12 and a `$C000xx` address outside M13: unmapped; the machine stops with a report until a fact exists | policy | spec section 1 |

## 3. Z80 address map (the sound CPU)

| # | Range | What | Label | Source |
|---|---|---|---|---|
| Z1 | 8 KB RAM, visible to the 68000 at `$A00000-$A01FFF` byte accesses only | [H] | PD-Z80 ("only use byte accesses") |
| Z2 | The Z80 program starts at `$0000` and must fit 8 KB | [H] | PD-Z80 |
| Z3 | The YM2612 is at Z80 `$4000-$4003` and 68000 `$A04000-$A04003`; the status register must be read from `$A04000`/`$4000` and only bits 7, 1, 0 mean anything | [H] | PD-HW-ISSUES, PD-YM; mirror extent **[H?]** |
| Z4 | PSG at Z80 `$7F11` | [H] | PD-PSG |
| Z5 | Bank register at Z80 `$6000` (nine bits shifted in one at a time) and the 32 KB window `$8000-$FFFF` onto the 68000 map | [H?] | named in SR-TECH "Can access 32 KB of the 68000 memory map at once"; register address and shift protocol from memory: confirm by P-Z80MAP |
| Z6 | VDP ports are readable/writable from the Z80 at `$7F00-$7F1F` | [H?] | confirm by P-Z80MAP |
| Z7 | A 68000 request is made by writing `$0100` to `$A11100`; the grant is seen when bit 0 of `$A11100` reads 0; release by writing `$0000` | [H] | PD-Z80 |
| Z8 | Reset: assert by writing `$0000` to `$A11200`, release with `$0100`; RAM cannot be accessed while the Z80 is in reset | [H] | PD-Z80 |
| Z9 | After asserting reset wait at least 192 (68000) cycles before releasing, or the YM2612 may misbehave | [H] | PD-Z80 |
| Z10 | A Z80 access to the 68000 bus makes the Z80 wait for the bus arbiter; the 68000 finishes its current cycle first; the average Z80 delay is 3.3 Z80 cycles, the average 68000 delay 11 cycles | [H] | KABUTO "The bus system" |
| Z11 | The Z80 waits while the VDP does a DMA from the 68000 bus | [H] | KABUTO "The bus system", PD-HW-ISSUES |
| Z12 | Z80 interrupt: the VDP's VBlank pulse (IRQ line of the Z80); position and width | [H?] | confirm by P-Z80INT |

## 4. I/O ports

| # | Fact | Label | Source |
|---|---|---|---|
| I1 | Three ports (player 1, player 2, expansion), each with a control register (bit 6-0 direction, 0 = input) and a data register; bit 7 of control enables the external interrupt | [H] | PD-IO |
| I2 | Control `$A10009/B/D`, data `$A10003/5/7`, byte accesses, addresses two bytes apart | [H] | PD-IO |
| I3 | The external interrupt (IRQ 2) fires when pin 6 (TH) is an input and goes high to low, with control bit 7 = 1, VDP register `$0B` bit 3 = 1 and the 68000 mask allowing it | [H] | PD-IO "External interrupt" |
| I4 | Serial mode: control register bits 7-6 speed (4800/2400/1200/300 bps), bit 5 TR as RxData, bit 4 TL as TxData, bit 3 interrupt on receive; status bits: bit 0 transmit busy, bit 1 receive ready, bit 2 error; reading the serial control register is required before TxData/RxData is valid | [H] | PD-IO |
| I5 | Pad read: TH output selects the button group; 3-button state machine; 6-button third pulse and timeout | [H?] | PD-CONTROLLERS read, rows added in Task 12 |
| I6 | Version register bits (overseas/domestic, PAL/NTSC, version number, Mega CD present) | [H?] | rows added in Task 6 from PD / a second document |

## 5. VDP

| # | Fact | Label | Source |
|---|---|---|---|
| V1 | Registers `$00-$17`, written by the control port word `$8000 | reg << 8 | value` | [H] | PD-VDPREG, PD-VDPCMD |
| V2 | A register write clears the pending address (a following data access needs a new address) | [H] | PD-VDPCMD "Set register" |
| V3 | Write address command: two words; bits 13-10... `((address & $3FFF) << 16) | ((address & $C000) >> 14) | command`; commands: VRAM write `$40000000`, CRAM write `$C0000000`, VSRAM write `$40000010`, VRAM read `$00000000`, CRAM read `$00000020`, VSRAM read `$00000010`; DMA adds `$80` to the low word | [H] | PD-VDPCMD |
| V4 | After each data word the address increases by register `$0F` | [H] | PD-VDPCMD |
| V5 | Register `$00`: bit 5 LCB blank left column, bit 4 IE1 H-interrupt enable, bit 1 M3 freeze HV counter | [H] | PD-VDPREG |
| V6 | Register `$01`: bit 6 DISP, bit 5 IE0 V-interrupt enable, bit 4 M1 DMA enable, bit 3 M2 V30 (only correct on PAL) | [H] | PD-VDPREG |
| V7 | Register `$02` plane A base = bits 5-3 x `$2000`; `$04` plane B base = bits 2-0 x `$2000`; `$03` window base bits 5-1 x `$800` (bit 1 clear in H40); `$05` sprite base bits 6-0 x `$200` (bit 0 clear in H40); `$0D` hscroll base bits 5-0 x `$400` | [H] | PD-VDPREG |
| V8 | Register `$07` background colour: bits 5-4 palette, bits 3-0 index | [H] | PD-VDPREG |
| V9 | Register `$0A` H-interrupt counter reload | [H] | PD-VDPREG |
| V10 | Register `$0B`: bit 3 IE2 external interrupt, bit 2 VSCR (0 full, 1 2-cell), bits 1-0 HSCR (00 full, 01 8-line, 10 per tile row, 11 per line) | [H] | PD-VDPREG |
| V11 | Register `$0C`: bit 7 and bit 0 RS0/RS1 (both set = H40, both clear = H32), bit 3 shadow/highlight, bits 2-1 interlace (00 off, 01 mode 1, 11 mode 2) | [H] | PD-VDPREG |
| V12 | Register `$10` plane size: HSZ bits 1-0 and VSZ bits 5-4: 32/64/128 wide x 32/64/128 high; only 32x32, 64x32, 128x32, 32x64, 64x64, 32x128 work | [H] | PD-VDPREG |
| V13 | Registers `$11/$12` window position (X in 16 px steps, Y in 8 px steps, bit 7 = right/below) | [H] | PD-VDPREG |
| V14 | Registers `$13/$14` DMA length (0 means 65 536), `$15/$16/$17` DMA source (bits 22-1; `$17` bits 7-6: 0x = bus transfer with bit 6 as source bit 23, 10 = VRAM fill, 11 = VRAM copy) | [H] | PD-VDPREG, PD-HW-ISSUES |
| V15 | A bus-to-VDP DMA cannot cross a 128 KB source boundary: it wraps within the block | [H] | PD-HW-ISSUES (note: KABUTO says "CRAM DMA wrapping aborts" is wrong) |
| V16 | A fill starts when the fill value is written to the data port after the command | [H] | PD-VDPCMD |
| V17 | VRAM 64 KB, CRAM 64 x 9 bit, VSRAM 40 words (64 words from model 2 VA4; column -1 is garbage on early, copy of column 0 on later) | [H] | SR-TECH, PD-HW-ISSUES |
| V18 | Interrupts: VBlank level 6, HBlank level 4, external level 2 | [H] | SR-TECH |
| V19 | HV counter tables (HCounter ranges, VCounter increment point, HBlank set/clear, VBlank set/clear, F flag) for H32/H40 and PAL/NTSC V28/V30 | [H] | KABUTO "H/V counter values" (from the SpritesMind thread); values copied into code only with these rows |
| V20 | Sprites: 80 per frame (64 in H32), 20 per line (16 in H32), link list walk with loop guard; sprite cache behaviour | [H] | SR-TECH, KABUTO "Precise details of sprite rendering" |
| V21 | (closed) The disagreement of `p20_fifo_stall` and `p25_loop_forms` with Genesis Plus GX, BlastEm and clownmdemu was the probes' fault: they wrote registers 11-23 before enabling Mode 5, which three of the cores ignore (fact V29). With the rule in place the loops agree with Genesis Plus GX within one H step | [O] probes p20, p23, p24, p25 | |
| V22 | Status register bits (FIFO empty/full, VINT pending, sprite overflow, collision, odd frame, VBlank, HBlank, DMA busy, PAL) | [H?] | to read from a document in Task 8 |
| V23 | Debug register `$C0001C` | [H] | KABUTO; not emulated in Phase 1 (stop) |

## 6. Cartridge header and mappers

| # | Fact | Label | Source |
|---|---|---|---|
| H1 | ROM header format at `$100`: console name, copyright, names, serial, checksum, I/O support, ROM start/end, RAM start/end, SRAM info, region field | [H] | PD-ROMHDR (read in Task 5) |
| H2 | SSF2 mapper registers `$A130F3-$A130FF` | [H] | SR-MAP; protocol PD-BEYOND4MB (Task 27) |
| H3 | SRAM enable register `$A130F1` | [H] | SR-MAP; PD-SRAM (Task 27) |
| H4 | Sonic & Knuckles lock-on | [H] | PD-LOCKON (Task 27) |

## 7. YM2612 and PSG

Rows are added in Tasks 16 and 17 from PD-YM, PD-PSG, the SpritesMind research and probes. Known from this phase:

| # | Fact | Label | Source |
|---|---|---|---|
| S1 | Registers `$30-$FF` live in a rotating shift register: a write takes effect only when the register comes round; the minimum interval between writes is half a YM2612 sample (about 33.6 Z80 cycles) | [H] | KABUTO "The YM2612" |
| S2 | The busy flag is not usable | [H] | KABUTO, PD-HW-ISSUES |
| S3 | Low volume distortion ("ladder effect") of the discrete YM2612 versus the integrated YM3438; sign-bit behaviour of the DAC | [H] | SR-TECH "Hardware issues" |

## 8. Probe register

| Probe | Question | State |
|---|---|---|
| P-FRAME | Master clocks per frame NTSC and PAL (counts, frame rate reported by the cores) | to do (Task 3) |
| P-Z80MAP | Z80 map: bank register address and protocol, VDP ports at `$7F00`, YM mirror extent | to do |
| P-Z80INT | Z80 interrupt position and width | to do |
| P-VDP-DOT | H40 dot clock over a line | Phase 3 |

| Cartridge area above a small ROM | A ROM repeats at its power-of-two size across `$000000-$3FFFFF`; Sonic & Knuckles (overseas name) does not: its upper 2 MB is the lock-on socket and reads `$FF` with nothing locked on, so its own lock-on check fails over to the standalone title | [O] Genesis Plus GX source reading (cart mapping) and probe `p08b_rom_map` on three cores; standalone S&K boots to the SEGA logo | `genesis/src/cart.rs` `mirror_floor` |

## 9. Timing (accuracy work)

| Id | Fact | Class / source | Used in |
|---|---|---|---|
| M16 | The 68000 loses 2 CPU cycles at the start of an external bus cycle every 128 CPU cycles (the bus refresh); the refresh interval restarts when the delay is taken, so one in 130 cycles is lost (1.5%) | [O] probe `p22_line_cycles`: 229 iterations of the loop in 20 lines on Genesis Plus GX, PicoDrive and BlastEm against 232 without; source reading: Genesis Plus GX and BlastEm both model it (128 cycles, 2 cycles) | `genesis/src/bus.rs` `bus_cycle` |
| V20 | The V counter changes between H = $A2 and $AE of the previous line (ours: $A5); the H counter advances one step per 16 master clocks in H40 within the picture (probes `p27_hv_hint`, `p28_v_increment`: four cores agree with ours within 3 steps) | [O] probes p23, p27, p28 | `vdp/src/frame.rs` `read_hv` |
| V21 | Open: loops that write the VDP or RAM in a tight loop measured with the H counter (`p20_fifo_stall`, `p25_loop_forms`) differ from Genesis Plus GX, BlastEm and clownmdemu by up to 15 H steps for some code alignments while matching PicoDrive; the cause is not found (not the H counter scale, not instruction timing: `p23`, `p24` agree). Ecco's and Sonic 2's title animation phases drift from Genesis Plus GX for this or a related reason; PicoDrive drifts from Genesis Plus GX more than we do | [H?] | |
| Z13 | A Z80 access through the `$8000` window waits 3 Z80 clocks on average for the 68000's bus (longer while a VDP DMA from the 68000's bus runs) and costs the 68000 about 10 CPU cycles (Genesis Plus GX 10-11, BlastEm 8) | [O] probe `p30b_z80_rom_loop` (68000 iteration count in 20 lines 229 -> 208 on Genesis Plus GX, 214 BlastEm, 213 PicoDrive, 209 here); [H] the wait: Nemesis' bus timing notes, as cited by Genesis Plus GX and BlastEm | `genesis/src/z80bus.rs` `z80_takes_68k_bus` |
| V28 | The data port's FIFO has four entries that the VDP empties in its external access slots; while the picture is drawn a line has 18 slots in H40 and 16 in H32 (the rest are used for the picture), a word of VRAM needs two of them and a word of CRAM or VSRAM one, and a 68000 write to a full FIFO waits for the oldest entry; while the display is off or the line is blank nothing is held up (the status register's bit 9 means empty, bit 8 full) | [O] probe `p29_fifo_rate` (200 VRAM writes take 21 lines in H40, 24 in H32, 7 with the picture off; CRAM and VSRAM 11): Genesis Plus GX, PicoDrive and BlastEm agree; `p31_*` (the status bits), `p33_slots_*` (when the n-th write is accepted); slot positions fitted to Genesis Plus GX within about 16 clocks (`scenarios/fitslots.py`) | `vdp/src/fifo.rs` |
| V29 | Registers 11 to 23 cannot be written while Mode 4 is selected (register 1 bit 2 clear): a probe that sets 40 cells before it enables Mode 5 stays in 32 cells | [O] probes `p34_*` (the first version wrote register 12 first: Genesis Plus GX, BlastEm and clownmdemu ignored it, PicoDrive did not); [H] Genesis Plus GX's comment names Captain Planet & Avengers | `vdp/src/ports.rs` `write_register` |
| V30 | VDP DMA of 1000 units from line 20 takes: bus to VRAM 111 lines in H40 (125 in H32), bus to CRAM 55 (62), fill 55 (62), copy 111 (125); with the display off or in vertical blanking 9 / 5 / 5 / 9 lines in H40 (12 / 6 / 6 / 12 in H32) | [O] probes `p34_*`, `p35_*` (Genesis Plus GX; BlastEm and PicoDrive within one line) | `vdp/src/dma.rs` |

## 10. Serial EEPROM cartridges

| Id | Fact | Class / source | Used in |
|---|---|---|---|
| H5 | Some cartridges hold a 24Cxx serial EEPROM instead of battery RAM, driven by two lines (SCL, SDA) through the cartridge area `$200000-$3FFFFF` and detected by the serial number in the header; the chip follows the I2C protocol of the 24Cxx data sheets (start, device byte `1010 A2 A1 A0 R/W`, one or two word address bytes, data with acknowledge, page wrap on writes, sequential reads) and the one-byte X24C01 takes its 7-bit address and R/W in the first byte | [D] 24Cxx/X24C01 data sheets; [H] Genesis Plus GX source for which games and chips (read for understanding, the list is rewritten) | `genesis/src/eeprom.rs` |
| H6 | Electronic Arts boards: SCL on D6, SDA on D7 (in and out), written through odd bytes or words | [H] Genesis Plus GX source reading | `Board::Ea` |
| H7 | Sega boards: SCL on D1, SDA on D0 (in and out) | [H] same | `Board::Sega` |
| H8 | Acclaim 2 MB boards (NBA Jam): any write sets SCL from D1 and SDA from D0 (a word write is one write), SDA is read on D1 | [H] same | `Board::Acclaim16` |
| H9 | Acclaim 4 MB boards: an odd byte write sets SDA from D0, an even one SCL from D0; a word write with D0 low shows the chip in `$200000-$2FFFFF` for reads, with D0 high the ROM; reset shows the ROM again | [H] same | `Board::Acclaim32` |
| H10 | Not done: the Codemasters J-Cart boards (Micro Machines, Brian Lara Cricket) | | |
| V31 | Interlace (register 12 bit 1) alternates a field flag (status bit 4) every frame, starting with the even field when it is switched on; mode 2 (bits 2 and 1) uses 8x16 tiles (64 bytes each), scroll and sprite coordinates in rows twice as dense (sprite Y 10 bits from 256) and shows the rows `2 * line + field` of the double-height picture, one field per frame (the single-field output of Genesis Plus GX; weaving two fields is not done); the V counter shifts left by one in mode 2 and bit 0 takes the value of bit 8 in both modes | [O] probes `p40_im2`, `p41_im1` (pictures identical to Genesis Plus GX on four frames), `p42_interlace_counter` (Genesis Plus GX and BlastEm identical to the byte) | `vdp/src/render.rs`, `frame.rs` |
| H11 | Sonic & Knuckles lock-on: a second cartridge plugged into the top of a Sonic & Knuckles cartridge shows its ROM from `$200000` (mirrored by its own size) and brings its own battery RAM (mapped over `$200000` by `$A130F1` bit 0 as usual); Sonic 3 does not have it mapped at power on although its ROM ends below it. The 256 KB chip Sonic 2 needs at `$300000` is not emulated (no dump) | [O] `scenarios/compare_lockon.py`: Sonic 3 locked onto Sonic & Knuckles matches Genesis Plus GX (`sk.bin`/`sk2chip.bin` stand-ins) at frames 200, 1400, 2000 and 3000 with Start/C presses, within animation phase at the others; [H] Genesis Plus GX source reading | `genesis/src/cart.rs` `lock_on` |
| H12 | J-Cart (Codemasters): two more pads answer in `$380000-$3FFFFF`; a read gives pad 3's seven pins (TH on bit 6) in the low byte and pad 4's six pins in the high byte, with the serial EEPROM's data line on D7 when the board has one; a write drives TH of both pads from bit 0. The Codemasters EEPROM boards take their clock and data from D1 and D0 of any write in `$300000-$37FFFF` (Brian Lara Cricket, which has no J-Cart, also in the upper half and reads the data line there). Games are recognised by serial number, or by the first vector long and header checksum for the ones without | [O] probe `p45_jcart` (Genesis Plus GX; the other three cores have no J-Cart); [H] Genesis Plus GX source reading | `genesis/src/io.rs`, `cart.rs`, `eeprom.rs` |
| H13 | Game Genie codes `XXXX-YYYY`: eight characters of `ABCDEFGHJKLMNPRSTVWXYZ0123456789` (5 bits each) scramble a 24-bit address and a 16-bit word (`ijklm nopIJ KLMNO PABCD EFGHd efgha bcQRS TUVWX` for characters 1-8, capitals the address, lowercase the data, most significant first); the word at the address reads as the data. Pro Action Replay style codes `AAAAAA:DDDD`/`FFxxxx:DD` patch the ROM or poke work RAM once a frame. Only the effect of the codes is emulated, not the cartridge's ROM or menus | [H] PicoDrive's documentation of the code format (worked example `SCRA-MBLE` = `059C44:B078`, a test) | `genesis/src/cheats.rs` |
| V32 | Option `interlace_output = weave` shows interlace mode 2 as both fields woven into a 448-line picture (each frame replaces its own field's rows; the other rows keep the previous frame's); the default shows the one field, as Genesis Plus GX does | [H] design choice; the field's rows equal the single-field picture | `vdp/src/frame.rs` |
| I7 | Sega Mouse in a controller port: TH falling starts a packet, each toggle of TR (after a delay of two reads before TL follows it) advances it through nine nibbles read on D3-D0: `$B`, `$F`, `$F`, the sign bits (X in bit 0, Y in bit 1), the buttons (start, middle, right, left from D3 to D0), X high, X low, Y high, Y low (two's complement bytes); Y counts up as positive (the host's down is negative); the motion is spent once a whole packet was read | [O] probe `p46_mouse` against Genesis Plus GX with the mouse moved in the frame of the read (`scenarios/compare_devices.py`: identical); the other cores were not asked | `genesis/src/io.rs` |
| I8 | EA 4 Way Play (port 1 = four pads, port 2 = the selector): a write to port 2 with UP and DOWN low latches TH, TR and TL; TR and TL choose the pad that port 1 reads and that TH writes of port 1 reach, TH high makes port 1 read `$7C` (D1 and D0 low: the adapter's identification); port 2 reads `$7F` | [H] Genesis Plus GX source reading; unit test only (the libretro frontend has no way to select the adapter here) | `genesis/src/io.rs` |
| I9 | Menacer in port 2: buttons D0 B, D1 trigger (A), D2 C, D3 start, active high, TH reads high; where it points makes the VDP, on the picture line of its Y, raise the external interrupt (level 2, register 11 bit 3) and latch the counters (V = the line, H from X scaled by 289/320 and halved, through the counter's skip) when the port's control register has bit 7 set; the latch is held while register 0 bit 1 is set, else it shows only on that line | [O] probe `p47_menacer` against Genesis Plus GX at four positions (identical); `scenarios/compare_devices.py` | `genesis/src/io.rs`, `vdp/src/frame.rs` |
| V33 | `--execution-mode fast` leaves out the three bus timing effects the references measure (refresh delay M16, Z80 bus cost Z13, FIFO slots V28), `accurate` (the default for this machine) has them; neither changes the instruction timing of the 68000 or the Z80 | [O] probes p22, p29, p30: with them the machine matches Genesis Plus GX, without them MAME and clownmdemu | `genesis/src/bus.rs`, `z80bus.rs`, `vdp/src/fifo.rs` |
| V34 | A write to a colour (CRAM) or to the backdrop register during the picture of a line changes the colour of the pixels from where the beam is: the line is worked out as palette indices at its start and coloured at its end, replaying the writes at the pixel `(clock - picture start) / 8` (H40; 10 in H32) of the moment the VDP takes the word from its FIFO, which is a slot, so the edge of a colour change moves in steps of the slots (16 pixels in a group of three, then a gap). Genesis Plus GX draws the line before any write of it (no staircase); BlastEm shows the 16-pixel groups | [O] probe `p48_cram_midline` (a staircase of edges against BlastEm: same group structure, offsets differ by the fit of the slot table) | `vdp/src/frame.rs` `flush_line` |
| V35 | PAL (313 lines): a 1000-unit bus-to-VRAM DMA from line 20 takes 111 lines with V28 and with V30 (same as NTSC); from line 200 it ends 31 lines later in V28 (picture ends at 224) and 46 lines later in V30 (picture ends at 240) | [O] probes `p49`, `p49b`, `p49c_*` (Genesis Plus GX, PicoDrive, BlastEm; clownmdemu differs) | `vdp/src/dma.rs` |
| V36 | The HBLANK status bit (bit 2) changes twice a line, near H counter 0x18 and 0xF3 in H40 (and 0x1A, 0xF6 in H32 run), the same on all four oracles and here to within the polling loop's jitter (about 8 counts) | [O] probe `p50_hblank_flag` (too jittery for an exact manifest scenario) | `vdp` HV table V19 |

## 11. Sega CD: discs (slice 1)

| Id | Fact | Class / source | Used in |
|---|---|---|---|
| D1 | A raw data sector is 2352 bytes: 12 bytes of sync (`00`, ten `FF`, `00`), a 4-byte header (minutes, seconds, frames in BCD, then the mode byte), then 2048 user bytes in mode 1 (so user data starts at offset 16). The header of the first sector of the program area is 00:02:00 mode 1 and counts up by one frame per sector; so LBA 0 is absolute time 00:02:00 (150 sectors of lead-in are not counted) | [O] the first two sectors of the Dragon's Lair RomM image (`00FFFFFFFFFFFFFFFFFFFF00 00 02 00 01`, then `... 00 02 01 01`, user data starting `SEGADISCSYSTEM`); [H?] ECMA-130 not read yet | `disc/src/disc.rs` `read_raw`, `msf` |
| D2 | A second is 75 frames (sectors); the audio track and data track sector sizes in a BIN are 2352 bytes | [H?] Red Book / ECMA-130 from memory, to confirm; only the MSF arithmetic of cue sheets and headers uses 75 | `disc/src/cue.rs`, `disc.rs` |
| D3 | Cue sheet commands used: `FILE`, `TRACK nn <AUDIO|MODE1/2048|MODE1/2352|MODE2/2048|MODE2/2352>`, `PREGAP`, `POSTGAP`, `INDEX nn mm:ss:ff` (INDEX 00 starts the pregap, INDEX 01 the track; positions are from the start of the file; PREGAP adds silence that is not in the file) | [H] cue sheet format as documented by the CDRWIN manual and used by the images on the server (all 24 cue sheets there are single-BIN); not a hardware fact | `disc/src/cue.rs` |

## 12. Sega CD: maps and gate array registers (slice 2)

Source `MCD-HW` = Sega Mega-CD Hardware Manual, version 1.0, 14 Oct 1991 (page numbers are the manual's own). It is a scan with a watermark and a few smudged words; where a bit could not be read it is marked `[H?]` and goes to a probe. Facts are written here in our own words.

| Id | Fact | Class / source | Used in |
|---|---|---|---|
| C1 | Main 68000 map with the Mega-CD attached: `$000000-$01FFFF` the BIOS ROM (128 KB, read only); `$020000-$03FFFF` a 128 KB window onto the 512 KB program RAM, bank chosen by `$A12002` bits 7-6, usable only while the sub-CPU is halted or has granted its bus; `$200000-$23FFFF` word RAM; the gate array registers at `$A12000-$A1202F`. Cartridge-boot (mode 1) puts the BIOS ROM at `$400000`, the program RAM window at `$420000`, word RAM at `$600000` instead (not built in this design) | [H] MCD-HW p15-16 and fig. p12-13; [H] Plutiedev "Using the Sub-CPU in mode 1" | `segacd` bus |
| C2 | Sub 68000 map: `$000000-$07FFFF` program RAM (512 KB; holds the sub program, the CD's table of contents and PCM sample data), with `$000000-$01FFFF` write-protectable in `$200` byte steps from the main side (`$A12002` bits 15-8, one bit per `$200` range 0 to 7 in the order the manual lists: the field covers `$0-$1FDFF`); `$080000-$0BFFFF` word RAM in 2M mode (256 KB); `$0C0000-$0DFFFF` the sub's 128 KB word RAM bank in 1M mode, and in 1M mode `$080000-$0BFFFF` shows the same 128 KB as a "dot image" (one 4-bit pixel per byte); `$0E0000-$0FDFFF` and `$FE4000-$FEFFFF`, `$FF4000-$FF7FFF` reserved; `$FE0000-$FE3FFF` backup RAM (8 KB, data on odd addresses only); `$FF0000-$FF3FFF` PCM chip (low byte only); `$FF8000-$FF81FF` sub registers | [H] MCD-HW p12-15 | `segacd` sub bus |
| C3 | Sub interrupt levels: 6 sub-code (96-byte buffer done), 5 CDC (error correction or buffering done), 4 CDD (a status command reception finished), 3 timer reached zero, 2 software interrupt requested by the main CPU, 1 graphics operation finished (2M mode). Mask register `$FF8032`: bit n enables level n (the manual draws bits 1 and 3 and says IEN1-6; reading gives the mask). The manual advises raising level 2 from the main CPU's vertical interrupt | [H] MCD-HW p15, p30 | `mcd-gate-array` |
| C4 | Main `$A12000`: bit 15 IEN2 (is level 2 enabled on the sub side; read), bit 8 IFL2 (write 1 raises sub level 2 when enabled, read 0 while it is being served, 1 when it has not been taken), bit 1 SBRQ (write 1 requests the sub bus, read 1 when granted, write 0 releases), bit 0 SRES (write 0 resets the sub-CPU, 1 runs it; read 0 while the reset is in progress, 1 when finished). A read-modify-write with bit instructions must not be used. Power-on value 0002 [read from the register table, smudged] | [H] MCD-HW p56, p21 | `mcd-gate-array` |
| C5 | Forced reset of the gate array: the main CPU writes the word `$FF00` to `$A12002`, then the bytes `$03`, `$02`, `$00` to `$A12001`, in exactly this order, with these sizes; anything else is not a reset. It clears everything but the main-side registers | [H] MCD-HW p56; [H] Plutiedev (same sequence) | `mcd-gate-array` |
| C6 | Main `$A12002` and sub `$FF8002` (one register, two views). Bits 15-8 WP7..WP0 (main writes, sub reads), bits 7-6 BK1/BK0 (main), bit 2 MODE (0 = 2M, 1 = 1M; written by the sub, read by both), bit 1 DMNA, bit 0 RET, bits 4-3 on the sub side are the write priority mode PM1/PM0 (00 normal, 01 underwrite, 10 overwrite, 11 forbidden: only in effect for sub writes into word RAM in 1M mode and for rotation writes in 2M mode). Power-on: `$0001` on both (2M mode, word RAM with the main CPU) | [H] MCD-HW p22-25, p57; the smudged bit positions of PM and BK are [H?] | `mcd-gate-array` |
| C7 | 2M mode word RAM hand-over: the main CPU writes DMNA = 1 to give the 256 KB to the sub-CPU (DMNA reads 0 until the hand-over finished, then 1); the sub-CPU writes RET = 1 to give it back (RET reads 1 when the main CPU has it). Only the main CPU can set DMNA, only the sub-CPU RET. After setting a bit, software must read until both have the wanted value. A sub access while RET = 1 in 2M mode (the RAM belongs to main) hangs the sub-CPU until a forced reset | [H] MCD-HW p23-25, p29 | `segacd` word RAM |
| C8 | 1M/1M mode: word RAM is two 128 KB banks, one for each CPU, `$200000-$21FFFF` for main and `$0C0000-$0DFFFF` for the sub. A swap is requested by the main CPU writing DMNA = 0 (the manual's main page says 0; its sub page says writing 1 requests; the two disagree, scan unclear) and is done by the sub CPU through RET: RET = 0 means bank 0 to main and bank 1 to sub, RET = 1 the reverse; DMNA reads as pending until the swap happened. Exact bit sense and who sets what are `[H?]`, to be decided by a probe against two oracles before this is coded | [H] MCD-HW p13, p24-25, p57 (contradictory scan); [H?] | `segacd` word RAM |
| C9 | Communication registers: command words `$A12010-$A1201E` written by the main CPU (read-only for the sub side at `$FF8010-$FF801E`), status words `$A12020-$A1202E` written by the sub-CPU (`$FF8020-$FF802E`; read-only for main); eight words each. Flag word `$A1200E` = `$FF800E`: the high byte (CFM) is written by the main CPU and read by the sub, the low byte (CFS) the reverse; simultaneous read and write of the same flag may return the old value. Power-on all zero | [H] MCD-HW p28-29, p59-60, p21 | `mcd-gate-array` |
| C10 | Stopwatch: a 12-bit counter, one count per 30.72 us, wraps from 4095 to 0; read from `$A1200C` (main) and `$FF800C` (sub); a write of zero by the sub-CPU clears it. 30.72 us is exactly 384 clocks of a 12.5 MHz sub-CPU | [H] MCD-HW p28, p59; the 12.5 MHz is [H] Sega Retro technical page (not fetched: blocked), consistent with the 384 | `mcd-gate-array` |
| C11 | General purpose timer `$FF8030`: write n (0..255) starts a count down at 30.72 us per count, level 3 interrupt when it reaches zero, so the period is (n + 1) x 30.72 us; n = 0 gives no interrupt; a read returns the value written; reset value 0 | [H] MCD-HW p30 | `mcd-gate-array` |
| C12 | H-INT vector `$A12006` (word, power-on `$FFFF`): when the VDP's horizontal interrupt goes to the main CPU, the long word the CPU would fetch from `$000070` is replaced with a jump address whose high part comes from the vector table value and whose low 16 bits are this register. Exact fetch behaviour is `[H?]`, to be probed | [H] MCD-HW p58 (wording unclear); [H?] | `segacd` bus |
| C13 | CDC register window on the sub side (`$FF8004` mode / register address, `$FF8006` register data, `$FF8008` host data, `$FF800A` DMA address) and on the main side (`$A12004` mode, `$A12008` host data): the destination bits DD2-DD0 pick main read (010), sub read (011), PCM RAM DMA (100), program RAM DMA (101), sub word RAM DMA (111); bit 15 EDT (end of transfer), 14 DSR (data set ready), 13 UBR (upper byte ready). DMA address in units of 8 bytes (limits: PCM 4 KB window bits 12 and up zero... see slice 3). Used from slice 3 | [H] MCD-HW p26-27, p58 | slice 3 |
| C14 | Sub-CPU register table (offsets from `$FF8000`, word units): 00 reset/LED/version, 02 memory mode, 04-0A CDC, 0C stopwatch, 0E flag, 10-1E commands, 20-2E status, 30 timer, 32 interrupt mask, 34 fader, 36 CDD control, 38-40 CDD status (5 words), 42-4A CDD command (5 words), 4C-56 font colour/bit/data, 58-66 rotation, 68 sub-code address, 100-17E sub-code data, 180-1FE its image. The main side has `$A12000` to `$A1202E` as above | [H] MCD-HW p19-20 | `mcd-gate-array` |
| C15 | Sub-CPU `$FF8000` reset register: writing 0 to bit 0 (RES0) resets the peripherals, which come back (bit 0 reads 1) about 100 ms later; chip version bits read in the register's upper nibble of the low byte; two LED bits (red and green, both 0 after power-on or reset) tell the BIOS's state to the user. Exact bit positions of the LEDs and the version nibble are `[H?]` (smudged) and not needed for emulation beyond returning plausible values | [H] MCD-HW p21-22; [H?] | `mcd-gate-array` |
