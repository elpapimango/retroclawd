#!/usr/bin/env python3
"""Finds which addresses of the Mega ST clock area answer (the others bus-error and the probe stops): one Hatari run a address.

    scan_p32.py ROM HATARI

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
    asm = os.path.join(here, "_scan32.s")
    st = os.path.join(here, "_scan32.st")
    open(asm, "w", newline="\n").write(src)
    subprocess.run([sys.executable, os.path.join(here, "mkprobe.py"), os.path.join(tools, "target", "debug"), asm, st], check=True, capture_output=True)
    out = os.path.join(here, "_scan32.bin")
    if os.path.exists(out):
        os.remove(out)
    subprocess.run(
        [sys.executable, os.path.join(tools, "scripts", "capture_hatari_mem.py"), rom, st, "400", "0xC000", "8", out, "--hatari", hatari, "--machine", MACHINE, "--memsize", "1"],
        capture_output=True,
        timeout=120,
    )
    d = open(out, "rb").read()
    if d[4:8] != bytes.fromhex("11223344"):
        return "stopped?"
    if d[:4] == bytes.fromhex("aa55aa55"):
        return "--"
    return d[:4].hex()


MACHINE = "megast"


if __name__ == "__main__":
    rom, hatari = sys.argv[1:3]
    for addr in range(0xFFFC1E, 0xFFFC42, 1):
        print("%06x byte" % addr, run(rom, hatari, addr, 1), flush=True)
    for addr in range(0xFFFC1E, 0xFFFC42, 2):
        print("%06x word" % addr, run(rom, hatari, addr, 2), flush=True)
