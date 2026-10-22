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


def text(s):
    """The bytes of `s`, a NUL and a pad byte when needed to end on a word boundary."""
    b = s.encode("ascii") + bytes([0])
    return b + (bytes([0]) if len(b) % 2 else b"")


# write-boot.bin: fills $30000 with "WRIT" x 128, writes it to logical sector 30 of drive A (BIOS 4 Rwabs, write),
# reads it back into $31000, compares the two and prints WRITE OK or WRITE FAILED on the console. The two Rwabs
# results (D0) are kept at $32000 and $32004. Getbpb (BIOS 7) comes first, as a TOS program does before it uses a
# disk it has not asked about. Rwabs(rwflag, buf, cnt, recno, dev): the arguments go on the stack right to left
# (Internals p.143). Branch displacements are worked out from the labels by `assemble`.
def assemble(items, data):
    """Two passes over `items`: ints are words, ("L", name) a label, ("S", opcode, name) a short branch (the opcode's
    low byte is the displacement) and ("D", name) a 16-bit displacement word relative to its own address (LEA d16(PC)).
    `data` is a list of (label, bytes) placed after the code. Returns the program's bytes."""
    labels, at = {}, 0
    for it in items:
        if isinstance(it, tuple) and it[0] == "L":
            labels[it[1]] = at
        else:
            at += 2
    for name, blob in data:
        labels[name] = at
        at += len(blob)
    out, at = [], 0
    for it in items:
        if isinstance(it, tuple) and it[0] == "L":
            continue
        if isinstance(it, tuple) and it[0] == "S":
            disp = labels[it[2]] - (at + 2)
            assert -128 <= disp <= 127 and disp != 0, (it, disp)
            out.append(it[1] | (disp & 0xFF))
        elif isinstance(it, tuple) and it[0] == "D":
            disp = labels[it[1]] - at
            assert 0 <= disp < 0x8000, (it, disp)
            out.append(disp)
        else:
            out.append(it)
        at += 2
    return words(out) + b"".join(blob for _, blob in data)


def rwabs(rwflag, buf):
    return [
        0x3F3C, 0x0000,          # MOVE.W #0,-(SP)        dev A
        0x3F3C, 0x001E,          # MOVE.W #30,-(SP)       recno
        0x3F3C, 0x0001,          # MOVE.W #1,-(SP)        cnt
        0x2F3C, buf >> 16, buf & 0xFFFF,  # MOVE.L #buf,-(SP)
        0x3F3C, rwflag,          # MOVE.W #rwflag,-(SP)
        0x3F3C, 0x0004,          # MOVE.W #4,-(SP)        Rwabs
        0x4E4D,                  # TRAP #13
        0xDFFC, 0x0000, 0x000E,  # ADDA.L #14,SP
    ]


WRITE_ITEMS = (
    [0x3F3C, 0x0000, 0x3F3C, 0x0007, 0x4E4D, 0x588F]  # Getbpb(0): MOVE.W #0,-(SP); MOVE.W #7,-(SP); TRAP #13; ADDQ.L #4,SP
    + [0x41F9, 0x0003, 0x0000,  # LEA $30000.L,A0
       0x303C, 0x007F,          # MOVE.W #127,D0
       ("L", "fill"),
       0x20FC, 0x5752, 0x4954,  # MOVE.L #"WRIT",(A0)+
       0x51C8, 0xFFF8]          # DBRA D0,fill
    + rwabs(1, 0x30000)         # write
    + [0x23C0, 0x0003, 0x2000]  # MOVE.L D0,$32000.L
    + rwabs(0, 0x31000)         # read back
    + [0x23C0, 0x0003, 0x2004,  # MOVE.L D0,$32004.L
       0x41F9, 0x0003, 0x0000,  # LEA $30000.L,A0
       0x43F9, 0x0003, 0x1000,  # LEA $31000.L,A1
       0x303C, 0x007F,          # MOVE.W #127,D0
       ("L", "cmp"),
       0xB388,                  # CMPM.L (A0)+,(A1)+
       ("S", 0x6600, "fail"),   # BNE.S fail
       0x51C8, 0xFFFA,          # DBRA D0,cmp
       0x45FA, ("D", "okmsg"),  # LEA okmsg(PC),A2
       ("S", 0x6000, "print"),  # BRA.S print
       ("L", "fail"),
       0x45FA, ("D", "failmsg"),  # LEA failmsg(PC),A2
       ("L", "print"),
       0x7000,                  # MOVEQ #0,D0
       ("L", "next"),
       0x101A,                  # MOVE.B (A2)+,D0
       ("S", 0x6700, "done"),   # BEQ.S done
       0x3F00,                  # MOVE.W D0,-(SP)        the character
       0x3F3C, 0x0002,          # MOVE.W #2,-(SP)        dev 2: the console
       0x3F3C, 0x0003,          # MOVE.W #3,-(SP)        BIOS Bconout
       0x4E4D,                  # TRAP #13
       0x5C8F,                  # ADDQ.L #6,SP
       ("S", 0x6000, "next"),   # BRA.S next
       ("L", "done"),
       0x60FE]                  # BRA.S *
)


def words(ws):
    return b"".join(struct.pack(">H", w) for w in ws)


def main() -> int:
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, code in [("tone.bin", TONE), ("keys-boot.bin", KEYS)]:
        with open(os.path.join(out, name), "wb") as f:
            f.write(words(code))
    write = assemble(WRITE_ITEMS, [("okmsg", text("WRITE OK")), ("failmsg", text("WRITE FAILED"))])
    with open(os.path.join(out, "write-boot.bin"), "wb") as f:
        f.write(write)
    return 0


if __name__ == "__main__":
    sys.exit(main())
