#!/usr/bin/env python3
"""Write a binary file as `dc.b` lines for the 68000 assembler (which has no .incbin): bin2dc.py IN.bin OUT.inc"""
import sys

data = open(sys.argv[1], "rb").read()
with open(sys.argv[2], "w", encoding="ascii") as out:
    for i in range(0, len(data), 16):
        out.write("    dc.b " + ",".join("$%02X" % b for b in data[i:i + 16]) + "\n")
    out.write("    dc.b 0\n" if len(data) % 2 else "")
