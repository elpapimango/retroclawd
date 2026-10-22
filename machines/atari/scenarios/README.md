# Atari ST scenario media

`build.sh` writes them into `software/scenarios/` (git-ignored), using `rcdisk` from retroclawd-tools and
`make_scenarios.py`. The 68000 code is written as words in `make_scenarios.py`, one instruction per line with its
offset, because the project has no 68000 assembler yet. `crates/machines/atari-st/tests/scenarios.rs` runs each
program and checks what it does.

## `tone.bin` (S4)

Loaded with `--program tone.bin --org 0x8000`. It writes the YM2149 registers through `$FF8800` (select) and
`$FF8802` (data) (*Atari ST Internals* pp.59-60):
- R0/R1 = 284, the tone A period;
- R7 = `$3E`, tone A only;
- R8 = 15, the level.

Then it loops (`BRA.S *`). The pitch is `fT = fMaster / (16 TP)` (Yamaha YM2149 catalog, section (1)), with the
master clock the CPU clock / 4 (`docs/hardware-facts.md`, PSG): 8 021 247 / 4 / (16 × 284) = 441.3 Hz.

## `hd.img` (S11)

`rcdisk hd new hd.img --size 8M` makes an 8 MiB disk with an AHDI root sector and one `GEM` partition formatted FAT16;
`rcdisk hd add` puts `HELLO.TXT` (39 bytes) in its root directory. The machine takes it as ACSI target 0
(`--acsi0 hd.img`). EmuTOS probes the targets (TEST UNIT READY, INQUIRY, READ CAPACITY, READ of the root sector), shows
DISK C, and a double click on its icon lists `C:\*.*` with the file.

## `hello.s` (S8P)

A GEMDOS program written in 68000 assembly: `pea msg(pc)`, `move.w #9,-(sp)` (Cconws), `trap #1`, `addq.l #6,sp`, then a loop.
`build.sh` assembles it with `rcasm`'s 68000 back end through `rcld` (`hello.layout.toml`, `machine = "atari-st"`) into
`hello.prg`, a relocatable GEMDOS executable with a `.text` section of code and a `.data` section holding the string. The
host hands `--program hello.prg` to the plugin as program media; the plugin builds a 720K floppy with `AUTO\HELLO.PRG`
(`st_floppy::boot`) and EmuTOS runs it before the desktop, writing `HELLO FROM RCASM` at the top left of the console.

## `raster.st` (S10)

A blank disk with a boot program (`rcdisk boot`) that takes the machine over: it installs its own HBL handler at `$68`
and VBL handler at `$70`, clears 32 000 bytes at `$60000`, points the video base there (`$FF8201`/`$FF8203`) so every
pixel is colour 0, and idles with `MOVE.W #$2100,SR`. The VBL handler zeroes D0 and D1 and writes palette entry 0
(`$FF8240`). The HBL handler counts lines in D1 and on every 20th adds `$0111` to D0 (masked with `$0777`) and writes it to
palette entry 0: bands of 20 lines, each a step lighter. Hatari draws the same bands (see the manifest's comment for the
measured difference).

## `serial.bin` (S9S)

Loaded with `--program serial.bin --org 0x8000 --serial-loopback --midi-loopback`. It programs the MFP USART
(*MC68901* section 6): timer D in delay mode with prescaler 4 and data 1 (`TDDR` `$FFFA25`, `TCDCR` `$FFFA1D`),
`UCR` `$88` (divide by 16, 8N1), `RSR` 1 and `TSR` 1. For each character of `SERIAL` it waits for `TSR` bit 7 (buffer
empty), writes `UDR` (`$FFFA2F`), waits for `RSR` bit 7 (buffer full) and copies `UDR` to `$0600`+i. The host's serial
loopback sends each character back, so the characters come in at the rate timer D sets. Then it resets the MIDI ACIA
(`$FFFC04` = 3), sets divide by 16 and 8N1 (`$15`) and does the same with `MIDI` (transmit empty is status bit 1, receive
full bit 0) into `$0610`+i, and loops. After the run memory `$0600-$061F` holds `SERIAL`, zeros, `MIDI`, zeros.

## `keys.st` (S2)

A blank 720K disk (`rcdisk create`) with an executable boot sector (`rcdisk boot`: checksum `$1234`, code at offset
30, *Internals* p.179). TOS runs the boot program before the desktop. The program:

1. **Joystick handler.** It calls XBIOS 34 `Kbdvbase` (`trap #14`, *Internals* p.199). The table it returns holds seven
   longs, `midivec` … `joyvec`, and the program puts a routine into `joyvec`, the seventh (offset 24). TOS calls
   that routine for each joystick packet. *Internals* p.199 says the packet's address is in D0, but EmuTOS 1.4, the
   firmware these scenarios run, passes it in A0 (and on the stack), with D0 holding the last byte. This was
   observed by stopping at the routine: A0 pointed at `$FF $00 $80`, the header and then joystick 0's and joystick
   1's state. The routine therefore takes joystick 1's byte at offset 2 from A0, shifts it right by one, and writes
   it into palette 0 (`$FF8240`). Fire (`$80`) becomes `$040`, a green the Shifter keeps
   (its palette fields are 3 bits).
2. **Echo loop.** It loops on BIOS 2 `Bconin(2)` and BIOS 3 `Bconout(2, c)` (`trap #13`, pp.143-144), echoing every
   typed character on the console.

## `files.st` (S3)

A 720K disk with `HELLO.TXT` and `README.TXT`, made with `rcdisk create` and `rcdisk add`. It is not bootable;
the desktop lists both files when drive A is opened.

## `write.st` (S3W)

A blank 720K disk with an executable boot sector (`rcdisk boot`) holding `write-boot.bin`. The program:

1. calls BIOS 7 `Getbpb(0)` (`trap #13`) so TOS has the disk's parameters;
2. fills `$30000` with the four bytes "WRIT" 128 times (512 bytes);
3. calls BIOS 4 `Rwabs` (`trap #13`, *Internals* p.143) to write that buffer to logical sector 30 of drive A, and keeps
   the result (D0) at `$32000`;
4. reads sector 30 back into `$31000` the same way (result at `$32004`);
5. compares the two buffers with `CMPM.L` and prints `WRITE OK` or `WRITE FAILED` on the console (BIOS 3 `Bconout`).

Logical sector 30 of the 720K disk is cylinder 1, side 1, sector 4. `tests/scenarios.rs` checks that the image holds
the pattern there and nowhere near it, and that the disk is marked modified; the manifest's S3W checks the picture.
The branch displacements are worked out from labels by `assemble` in `make_scenarios.py`.
