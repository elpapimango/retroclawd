#!/usr/bin/env python3
"""Writes p37_blitter_raw.s: the destination words the Blitter writes, raw, for skews with no end masks.

The method of gen_p33.py, but each case dumps 40 destination words (from word 64 of the area) instead of a checksum, so that
the shifter's first words can be read off. Result at $C000: header 'P37', then 80 bytes a case."""
import os

import gen_p33

FULL = 0xFFFF


def cases():
    out = []
    for flags in (0x00, 0x80, 0x40, 0xC0):
        for skew in range(16):
            out.append(gen_p33.case(hop=2, op=3, skew=skew | flags, m1=FULL, m2=FULL, m3=FULL, xc=3, yc=4))
    return out


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "p37_blitter_raw.s"), "w", newline="\n") as f:
        f.write(gen_p33.source(cases(), raw=True, name="p37"))
    print(len(cases()), "cases")
