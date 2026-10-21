#!/usr/bin/env python3
"""Writes the 68000 code of the Atari ST scenarios (README.md explains each instruction): tone.bin (for --program at
$8000) and keys-boot.bin (the boot program of keys.st). No 68000 assembler exists in this project yet, so the code
is written here as words, one instruction per line.

    make_scenarios.py OUT_DIR
"""
import os
import struct
import sys


def move_b(value, addr):
    """MOVE.B #value,(addr).W"""
    return [0x11FC, value, addr]


# tone.bin: YM2149 tone A, period 284 ($11C), level 15, mixer: tone A only; then BRA.S *.
TONE = (
    move_b(0, 0x8800) + move_b(0x1C, 0x8802)  # R0 = $1C (fine tone A)
    + move_b(1, 0x8800) + move_b(0x01, 0x8802)  # R1 = $01 (coarse tone A)
    + move_b(7, 0x8800) + move_b(0x3E, 0x8802)  # R7 = $3E (tone A on, the rest off; ports in)
    + move_b(8, 0x8800) + move_b(0x0F, 0x8802)  # R8 = $0F (level 15)
    + [0x60FE]  # BRA.S *
)

# keys-boot.bin, position independent (TOS runs a boot sector from wherever it loaded it).
KEYS = [
    0x3F3C, 0x0022,  # 0:  MOVE.W #34,-(SP)       XBIOS Kbdvbase
    0x4E4E,          # 4:  TRAP #14
    0x548F,          # 6:  ADDQ.L #2,SP
    0x2040,          # 8:  MOVEA.L D0,A0           the vector table
    0x43FA, 0x0022,  # 10: LEA handler(PC),A1      handler at 46 = 12 + $22
    0x2149, 0x0018,  # 14: MOVE.L A1,24(A0)        joyvec, the table's 7th long
    0x3F3C, 0x0002,  # 18 loop: MOVE.W #2,-(SP)    dev 2: the console
    0x3F3C, 0x0002,  # 22: MOVE.W #2,-(SP)         BIOS Bconin
    0x4E4D,          # 26: TRAP #13
    0x588F,          # 28: ADDQ.L #4,SP
    0x3F00,          # 30: MOVE.W D0,-(SP)         the character
    0x3F3C, 0x0002,  # 32: MOVE.W #2,-(SP)         dev 2
    0x3F3C, 0x0003,  # 36: MOVE.W #3,-(SP)         BIOS Bconout
    0x4E4D,          # 40: TRAP #13
    0x5C8F,          # 42: ADDQ.L #6,SP
    0x60E4,          # 44: BRA.S loop              18 - 46 = -28
    0x2248,          # 46 handler: MOVEA.L A0,A1   the joystick buffer: header, joystick 0, joystick 1
    0x7000,          # 48: MOVEQ #0,D0
    0x1029, 0x0002,  # 50: MOVE.B 2(A1),D0         joystick 1's state
    0xE208,          # 54: LSR.B #1,D0             fire ($80) -> $40: a palette value the Shifter keeps
    0x31C0, 0x8240,  # 56: MOVE.W D0,$FF8240.W     palette 0
    0x4E75,          # 60: RTS
]


def words(ws):
    return b"".join(struct.pack(">H", w) for w in ws)


def main() -> int:
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, code in [("tone.bin", TONE), ("keys-boot.bin", KEYS)]:
        with open(os.path.join(out, name), "wb") as f:
            f.write(words(code))
    return 0


if __name__ == "__main__":
    sys.exit(main())
