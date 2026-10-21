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
