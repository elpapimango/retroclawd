#!/usr/bin/env python3
"""Writes p38_blitter_masks.s: raw destination words for skew, the forced/skipped reads and end masks together.

The method of gen_p37.py with the end masks of gen_p33.py's sweep: the dump shows what the masks do to the data
that stays in the shifter after the word. Result at $C000: header 'P38', then 80 bytes a case."""
import os

import gen_p33


def cases():
    out = []
    for flags in (0x00, 0x40, 0x80):
        for skew in (0, 3, 9):
            for m1, m3 in ((0x3FFF, 0xFFFF), (0xFFFF, 0xFFFC), (0x3FFF, 0xFFFC)):
                out.append(gen_p33.case(hop=2, op=3, skew=skew | flags, m1=m1, m2=0xFFFF, m3=m3, xc=3, yc=4))
    return out


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "p38_blitter_masks.s"), "w", newline="\n") as f:
        f.write(gen_p33.source(cases(), raw=True, name="p38"))
    print(len(cases()), "cases")
