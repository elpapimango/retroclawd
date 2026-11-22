#!/usr/bin/env python3
"""Wrap a 68000 program in a valid Mega Drive ROM: mkrom.py CODE.bin OUT.md [--region JUE] [--name NAME]

CODE.bin is assembled to run at $200 (rcasm -c 68000 -f bin, `.org $200`). The ROM gets the vector table (initial SSP
$00FFFE00, reset vector $200, every other vector pointing at a `bra.s *` stub placed after the code), the header at $100
(layout: https://plutiedev.com/rom-header) with the checksum at $18E (the sum of the words from $200 to the end, modulo
65536) and the ROM end at $1A4, padded to a multiple of 128 bytes with $FF.
"""
import argparse
import struct


def field(text, size):
    data = text.encode("ascii")
    assert len(data) <= size, text
    return data + b" " * (size - len(data))


def build(code, region="JUE", name="RETROCLAWD PROBE", size=0, pattern=False, vectors_at=(), serial="00000000-00"):
    if len(code) % 2:
        code += b"\x00"
    stub_addr = 0x200 + len(code)
    body = code + b"\x60\xFE"
    size = max(size, (0x200 + len(body) + 127) & ~127)
    rom = bytearray(b"\xFF" * size)
    if pattern:
        # every 4 KB block distinct: the byte is the block number, so a mirror or an open bus shows in what is read
        for i in range(size):
            rom[i] = (i >> 12) & 0xFF
    vectors = struct.pack(">II", 0x00FFFE00, 0x200) + struct.pack(">I", stub_addr) * 62
    rom[0:len(vectors)] = vectors
    for n, addr in vectors_at:
        rom[4 * n:4 * n + 4] = struct.pack(">I", addr)
    rom[0x200:0x200 + len(body)] = body
    header = (field("SEGA MEGA DRIVE", 16) + field("(C)RCLW 2026.OCT", 16) + field(name, 48) + field(name, 48)
              + field("GM " + serial, 14) + b"\x00\x00" + field("J", 16)
              + struct.pack(">IIII", 0, size - 1, 0xFF0000, 0xFFFFFF) + field("", 12) + field("", 12) + field("", 40)
              + field(region, 3) + field("", 13))
    assert len(header) == 0x100, len(header)
    rom[0x100:0x200] = header
    total = sum(struct.unpack(">%dH" % ((size - 0x200) // 2), rom[0x200:])) & 0xFFFF
    rom[0x18E:0x190] = struct.pack(">H", total)
    return bytes(rom)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("code")
    ap.add_argument("out")
    ap.add_argument("--region", default="JUE")
    ap.add_argument("--name", default="RETROCLAWD PROBE")
    ap.add_argument("--size", type=lambda v: int(v, 0), default=0, help="pad the ROM to this many bytes")
    ap.add_argument("--pattern", action="store_true", help="fill the padding with the block number of each 4 KB")
    ap.add_argument("--serial", default="00000000-00", help="the product number after GM (11 characters)")
    ap.add_argument("--vector", action="append", default=[], help="N=ADDR: exception vector N points at ADDR (28 = level 4, 30 = level 6)")
    a = ap.parse_args()
    vecs = [(int(v.split("=")[0], 0), int(v.split("=")[1], 0)) for v in a.vector]
    open(a.out, "wb").write(build(open(a.code, "rb").read(), a.region, a.name, a.size, a.pattern, vecs, a.serial))
