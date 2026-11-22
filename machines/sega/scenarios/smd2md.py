#!/usr/bin/env python3
"""De-interleave a Super Magic Drive (.smd) dump into a plain ROM: smd2md.py IN.smd OUT.md

A .smd file is a 512-byte header followed by 16 KB blocks; in each block the first 8 KB hold the bytes at odd ROM
offsets and the second 8 KB those at even offsets. The result is validated by the ROM header's own checksum (the sum of the
words from $200 to the ROM end, at $18E), so a wrong layout is reported, not written.
"""
import struct
import sys


def deinterleave(data):
    body = data[512:]
    out = bytearray(len(body))
    for b in range(0, len(body) - len(body) % 16384, 16384):
        block = body[b:b + 16384]
        out[b + 1:b + 16384:2] = block[:8192]
        out[b:b + 16384:2] = block[8192:]
    return bytes(out)


def header_checksum_ok(rom):
    if rom[0x100:0x104] != b"SEGA" and rom[0x100:0x103] != b"SEG":
        return False
    end = struct.unpack(">I", rom[0x1A4:0x1A8])[0] + 1
    end = min(end, len(rom)) & ~1
    total = sum(struct.unpack(">%dH" % ((end - 0x200) // 2), rom[0x200:end])) & 0xFFFF
    return total == struct.unpack(">H", rom[0x18E:0x190])[0]


if __name__ == "__main__":
    rom = deinterleave(open(sys.argv[1], "rb").read())
    if not header_checksum_ok(rom):
        sys.exit("the header checksum does not match: not a .smd with this layout")
    open(sys.argv[2], "wb").write(rom)
    print("ok", len(rom))
