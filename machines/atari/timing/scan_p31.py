#!/usr/bin/env python3
"""Finds which addresses of the STE port area answer (the others bus-error and the probe stops): one Hatari run a address.

    scan_p31.py ROM HATARI

Prints, for each address, the word read there ('--' when it did not answer). Sentinel $AA55 is written first."""
import os
import subprocess
import sys

here = os.path.dirname(os.path.abspath(__file__))
tools = os.path.join(here, "..", "..", "..", "..", "retroclawd-tools")


def run(rom, hatari, addr, size):
    op = "move.w" if size == 2 else "move.b"
    src = "\n".join(
        [
            "    .org $90000",
            "    move.w  #$2700,sr",
            "    lea     $c000,a2",
            "    move.l  #$aa55aa55,(a2)",
            f"    {op}   ${addr:06x},d0",
            "    move.l  d0,(a2)",
            "    move.l  #$11223344,4(a2)",
            "done:",
            "    bra.s   done",
            "",
        ]
    )
    asm = os.path.join(here, "_scan.s")
    st = os.path.join(here, "_scan.st")
    open(asm, "w", newline="\n").write(src)
    subprocess.run([sys.executable, os.path.join(here, "mkprobe.py"), os.path.join(tools, "target", "debug"), asm, st], check=True, capture_output=True)
    out = os.path.join(here, "_scan.bin")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run(
        [sys.executable, os.path.join(tools, "scripts", "capture_hatari_mem.py"), rom, st, "400", "0xC000", "8", out, "--hatari", hatari, "--machine", "ste", "--memsize", "1"],
        capture_output=True,
        timeout=120,
    )
    d = open(out, "rb").read()
    if d[4:8] != bytes.fromhex("11223344"):
        return "stopped?"
    if d[:4] == bytes.fromhex("aa55aa55"):
        return "--"
    return d[:4].hex()


if __name__ == "__main__":
    rom, hatari = sys.argv[1:3]
    for addr in (0xFF9210, 0xFF9212, 0xFF9214, 0xFF9216, 0xFF9218, 0xFF921A, 0xFF9220, 0xFF9222, 0xFF9224):
        print("%06x word" % addr, run(rom, hatari, addr, 2), flush=True)
    for addr in (0xFF9201, 0xFF9203, 0xFF9211, 0xFF9217, 0xFF9219, 0xFF9221, 0xFF9223, 0xFF9225):
        print("%06x byte" % addr, run(rom, hatari, addr, 1), flush=True)
