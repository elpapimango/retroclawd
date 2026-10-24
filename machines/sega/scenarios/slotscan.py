#!/usr/bin/env python3
"""Turn a p33 probe's HV samples into times (master clocks since the start of the V-counter line) for each oracle.

    slotscan.py PROBE_NAME [--h32]

Each sample is the time from the HV read at the start of the handler to the one after the n-th write; the H counter is converted to a
clock with the model of vdp/src/frame.rs read_hv (the end of a line counts 0xA5..0xB6 then 0xE4..0xFF over 780 clocks).
"""
import argparse
import os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(HERE), "software", "out", "probes")
LINE = 3420


def clock_of(h, h40=True):
    off = 780 if h40 else 774
    if h >= (0xE4 if h40 else 0xE9):
        base = 18 if h40 else 15
        steps = 46 if h40 else 38
        first = 0xE4 if h40 else 0xE9
        return (h - first + base) * off // steps - off
    if h >= (0xA5 if h40 else 0x85) and h <= (0xB6 if h40 else 0x93):
        first = 0xA5 if h40 else 0x85
        steps = 46 if h40 else 38
        return (h - first) * off // steps - off
    return (h * (16 if h40 else 20))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("--h32", action="store_true")
    ap.add_argument("--count", type=int, default=44)
    a = ap.parse_args()
    times = {}
    for core in ("gpgx", "picodrive", "blastem", "clownmdemu"):
        data = open(os.path.join(OUT, f"{a.name}.md.{core}.ram"), "rb").read()[: 4 * a.count]
        t = []
        for n in range(a.count):
            w0 = int.from_bytes(data[4 * n:4 * n + 2], "big")
            w1 = int.from_bytes(data[4 * n + 2:4 * n + 4], "big")
            dv = ((w1 >> 8) - (w0 >> 8)) & 255
            t.append(dv * LINE + clock_of(w1 & 255, not a.h32) - clock_of(w0 & 255, not a.h32))
        times[core] = t
    print("n   " + " ".join(f"{k[:6]:>8}" for k in times))
    for n in range(a.count):
        print(f"{n + 1:2d}  " + " ".join(f"{times[k][n]:8d}" for k in times))


main()
