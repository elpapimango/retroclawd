#!/usr/bin/env python3
"""Writes p39_blitter_backwards.s: raw destination words of blits that run right to left (negative x increments).

Dumps 40 words from word 100 of the area; the blits start at word 130 and run backwards. Result at $C000: header 'P39',
then 80 bytes a case."""
import os

import gen_p33


def cases():
    out = []
    for flags in (0x00, 0x80, 0x40):
        for skew in (0, 3, 9):
            for m1, m3 in ((0xFFFF, 0xFFFF), (0x7FFF, 0xFFFE)):
                out.append(gen_p33.case(hop=2, op=3, skew=skew | flags, sx=-2, dx=-2, so=130, do=130, m1=m1, m2=0xFFFF, m3=m3, xc=3, yc=4))
    return out


if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "p39_blitter_backwards.s"), "w", newline="\n") as f:
        f.write(gen_p33.source(cases(), raw=100, name="p39"))
    print(len(cases()), "cases")
